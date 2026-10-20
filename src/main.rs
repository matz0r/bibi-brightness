// Hide the console window on Windows; no effect on Linux.
#![windows_subsystem = "windows"]

use ddc_hi::{Ddc, Display, DisplayInfo};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

const BRIGHTNESS: u8 = 0x10; // VCP feature code for brightness

/// Brightness presets: (linked monitors, other monitors) in percent.
const PRESETS: &[(u8, u8)] = &[(45, 20), (60, 25), (70, 30), (80, 40), (90, 45)];

/// What the UI knows about a monitor.
struct Monitor {
    key: String,  // stable identity (model + serial), used to remember links
    name: String, // shown in the UI
    percent: f32,
}

/// UI -> worker
enum Cmd {
    Scan,
    Set(usize, f32),
}

/// worker -> UI
enum Event {
    Scanning,
    Found(Vec<Monitor>),
    Error(String),
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Brightness")
            .with_app_id("brightness")
            .with_inner_size([520.0, 250.0]),
        ..Default::default()
    };
    eframe::run_native(
        "brightness",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

// ---------------------------------------------------------------------------
// Worker thread: owns the monitor handles. DDC/CI is slow (tens to hundreds
// of ms per write), so it never runs on the UI thread. While a write is in
// progress, newer slider values pile up and only the latest one per monitor
// is sent — the UI stays smooth no matter how fast you drag.
// ---------------------------------------------------------------------------

fn worker(rx: Receiver<Cmd>, tx: Sender<Event>, ctx: egui::Context) {
    let mut displays: Vec<(Display, u16)> = Vec::new(); // (handle, max value)

    while let Ok(first) = rx.recv() {
        let mut scan = false;
        let mut sets: HashMap<usize, f32> = HashMap::new();
        for cmd in std::iter::once(first).chain(rx.try_iter()) {
            match cmd {
                Cmd::Scan => scan = true,
                Cmd::Set(i, v) => {
                    sets.insert(i, v);
                }
            }
        }

        if scan {
            let _ = tx.send(Event::Scanning);
            ctx.request_repaint();
            let (found, infos) = scan_displays();
            displays = found;
            let _ = tx.send(Event::Found(infos));
            ctx.request_repaint();
            continue; // queued indices referred to the old list
        }

        for (i, percent) in sets {
            let Some((display, max)) = displays.get_mut(i) else {
                continue;
            };
            let raw = (percent / 100.0 * *max as f32).round() as u16;
            if let Err(e) = display.handle.set_vcp_feature(BRIGHTNESS, raw) {
                let _ = tx.send(Event::Error(format!(
                    "{}: {e}",
                    display_name(&display.info)
                )));
                ctx.request_repaint();
            }
        }
    }
}

fn scan_displays() -> (Vec<(Display, u16)>, Vec<Monitor>) {
    let mut found = Display::enumerate();
    // Sort by bus number on Linux (matches ddcutil's order), by name elsewhere.
    found.sort_by_key(|d| {
        (
            d.info.id.parse::<u64>().unwrap_or(u64::MAX),
            d.info.id.clone(),
        )
    });

    let mut displays = Vec::new();
    let mut monitors: Vec<Monitor> = Vec::new();
    for mut d in found {
        // Skip outputs that don't answer DDC/CI brightness (laptop panels, some TVs, …)
        let Ok(value) = d.handle.get_vcp_feature(BRIGHTNESS) else {
            continue;
        };
        let max = value.maximum().max(1);

        let mut key = display_key(&d.info);
        if monitors.iter().any(|m| m.key == key) {
            key = format!("{key}#{}", monitors.len()); // identical models without serials
        }
        monitors.push(Monitor {
            key,
            name: display_name(&d.info),
            percent: value.value() as f32 * 100.0 / max as f32,
        });
        displays.push((d, max));
    }
    (displays, monitors)
}

fn display_name(info: &DisplayInfo) -> String {
    match (&info.manufacturer_id, &info.model_name) {
        (Some(mfr), Some(model)) => format!("{mfr} {model}"),
        (None, Some(model)) => model.clone(),
        _ => info.id.clone(),
    }
}

fn display_key(info: &DisplayInfo) -> String {
    let serial = info
        .serial_number
        .clone()
        .or(info.serial.map(|s| s.to_string()))
        .unwrap_or_default();
    format!(
        "{}|{}|{}|{}",
        info.manufacturer_id.as_deref().unwrap_or(""),
        info.model_name.as_deref().unwrap_or(""),
        info.model_id.unwrap_or(0),
        serial
    )
}

fn no_monitor_hint() -> String {
    #[cfg(target_os = "linux")]
    {
        let devs: Vec<_> = std::fs::read_dir("/dev")
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with("i2c-"))
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default();
        if devs.is_empty() {
            return "No /dev/i2c-* devices found. Run: sudo modprobe i2c-dev".into();
        }
        let accessible = devs.iter().any(|p| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(p)
                .is_ok()
        });
        if !accessible {
            return "No permission for /dev/i2c-*. Run: sudo usermod -aG i2c $USER \
                    then log out and back in."
                .into();
        }
    }
    "No monitors with DDC/CI found. Check that DDC/CI is enabled in the monitor's menu.".into()
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

struct App {
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
    monitors: Vec<Monitor>,
    linked: HashSet<String>, // keys of monitors whose sliders move together
    scanning: bool,
    status: Option<String>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        thread::spawn(move || worker(cmd_rx, ev_tx, ctx));
        let _ = cmd_tx.send(Cmd::Scan);

        // Links saved from last run (~/.local/share/brightness/ on Linux)
        let linked = cc
            .storage
            .and_then(|s| s.get_string("linked"))
            .map(|s| {
                s.lines()
                    .filter(|l| !l.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        Self {
            tx: cmd_tx,
            rx: ev_rx,
            monitors: Vec::new(),
            linked,
            scanning: true,
            status: None,
        }
    }

    fn is_linked(&self, i: usize) -> bool {
        self.linked.contains(&self.monitors[i].key)
    }

    /// Set monitor `i`; if it is linked, every linked monitor follows.
    fn set(&mut self, i: usize, value: f32) {
        let follow = self.is_linked(i);
        for j in 0..self.monitors.len() {
            if j == i || (follow && self.is_linked(j)) {
                self.monitors[j].percent = value;
                let _ = self.tx.send(Cmd::Set(j, value));
            }
        }
    }

    /// Linked monitors get `linked`, all others get `other`.
    fn apply_preset(&mut self, linked: f32, other: f32) {
        for j in 0..self.monitors.len() {
            let v = if self.is_linked(j) { linked } else { other };
            self.monitors[j].percent = v;
            let _ = self.tx.send(Cmd::Set(j, v));
        }
    }

    fn handle_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Scanning => {
                    self.scanning = true;
                    self.status = None;
                }
                Event::Found(monitors) => {
                    self.scanning = false;
                    self.status = monitors.is_empty().then(no_monitor_hint);
                    self.monitors = monitors;
                    for m in &mut self.monitors {
                        m.percent = m.percent.round();
                    }
                }
                Event::Error(e) => self.status = Some(e),
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.handle_events();
        let mut links_changed = false;

        egui::CentralPanel::default().show(ui, |ui| {
            ui.spacing_mut().slider_width = 260.0;

            if self.scanning {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Searching for monitors…");
                });
            } else {
                if !self.monitors.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label("Presets");
                        for &(linked, other) in PRESETS {
                            if ui
                                .button(format!("{linked} / {other}"))
                                .on_hover_text(format!(
                                    "Linked monitors: {linked} %, other monitors: {other} %"
                                ))
                                .clicked()
                            {
                                self.apply_preset(linked as f32, other as f32);
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(4.0);
                }

                egui::Grid::new("monitors")
                    .num_columns(3)
                    .spacing([14.0, 10.0])
                    .show(ui, |ui| {
                        for i in 0..self.monitors.len() {
                            let linked = self.is_linked(i);
                            let name = egui::RichText::new(&self.monitors[i].name);
                            ui.label(if linked { name.strong() } else { name });

                            let mut value = self.monitors[i].percent;
                            if ui
                                .add(
                                    egui::Slider::new(&mut value, 0.0..=100.0)
                                        .step_by(1.0)
                                        .suffix(" %")
                                        .fixed_decimals(0),
                                )
                                .changed()
                            {
                                self.set(i, value);
                            }

                            let mut link = linked;
                            if ui
                                .checkbox(&mut link, "Link")
                                .on_hover_text("Linked monitors share one brightness value")
                                .changed()
                            {
                                links_changed = true;
                                let key = self.monitors[i].key.clone();
                                if link {
                                    // Join the group at the group's current value
                                    let group_value = (0..self.monitors.len())
                                        .find(|&j| self.is_linked(j))
                                        .map(|j| self.monitors[j].percent);
                                    self.linked.insert(key);
                                    if let Some(v) = group_value {
                                        self.set(i, v);
                                    }
                                } else {
                                    self.linked.remove(&key);
                                }
                            }
                            ui.end_row();
                        }
                    });
            }

            ui.add_space(8.0);
            if ui
                .add_enabled(!self.scanning, egui::Button::new("Rescan"))
                .clicked()
            {
                self.scanning = true;
                let _ = self.tx.send(Cmd::Scan);
            }
            if let Some(status) = &self.status {
                ui.add_space(4.0);
                ui.colored_label(ui.visuals().warn_fg_color, status);
            }
        });

        // Save links right away instead of only on a clean exit
        if links_changed && let Some(storage) = frame.storage_mut() {
            self.save(storage);
            storage.flush();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let mut keys: Vec<_> = self.linked.iter().cloned().collect();
        keys.sort();
        storage.set_string("linked", keys.join("\n"));
    }
}
