// Hide the console window on Windows; no effect on Linux.
#![windows_subsystem = "windows"]

use ddc_hi::{Ddc, Display, DisplayInfo};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[cfg(target_os = "linux")]
mod setup;
#[cfg(target_os = "linux")]
mod tray;

const BRIGHTNESS: u8 = 0x10; // VCP feature code for brightness

/// Brightness presets: (linked monitors, other monitors) in percent.
const PRESETS: &[(u8, u8)] = &[(45, 20), (67, 25), (75, 33), (80, 40), (90, 45)];

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
    /// Linux: one-time i2c access setup, then rescan
    #[cfg(target_os = "linux")]
    Setup,
}

/// worker -> UI
enum Event {
    Scanning,
    Found(Vec<Monitor>),
    SetFailed(usize, String),
    #[cfg(target_os = "linux")]
    SetupFailed(String),
}

fn main() -> eframe::Result {
    // Linux: icon in the panel; closing the window keeps it running there.
    // --tray starts hidden (autostart), --window is the window process the
    // tray starts.
    #[cfg(target_os = "linux")]
    if !std::env::args().skip(1).any(|a| a == "--window") {
        let hidden = std::env::args().skip(1).any(|a| a == "--tray");
        match tray::run(!hidden) {
            Ok(()) => return Ok(()),
            Err(e) => eprintln!("brightness: no tray available ({e}), opening the window"),
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("BIBI Brightness")
            .with_app_id("brightness")
            .with_inner_size([520.0, 330.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .expect("bundled icon is a valid PNG"),
            ),
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
        #[cfg(target_os = "linux")]
        let mut setup = false;
        let mut sets: HashMap<usize, f32> = HashMap::new();
        for cmd in std::iter::once(first).chain(rx.try_iter()) {
            match cmd {
                Cmd::Scan => scan = true,
                #[cfg(target_os = "linux")]
                Cmd::Setup => setup = true,
                Cmd::Set(i, v) => {
                    sets.insert(i, v);
                }
            }
        }

        #[cfg(target_os = "linux")]
        if setup {
            if let Err(e) = setup::run() {
                let _ = tx.send(Event::SetupFailed(e));
                ctx.request_repaint();
                continue;
            }
            scan = true;
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
            if let Err(e) = retry(2, || display.handle.set_vcp_feature(BRIGHTNESS, raw)) {
                let _ = tx.send(Event::SetFailed(i, e.to_string()));
                ctx.request_repaint();
            }
        }
    }
}

/// Some monitors answer DDC/CI with garbage now and then; try a few times.
fn retry<T, E>(attempts: u32, mut f: impl FnMut() -> Result<T, E>) -> Result<T, E> {
    let mut result = f();
    for _ in 1..attempts {
        if result.is_ok() {
            break;
        }
        result = f();
    }
    result
}

fn scan_displays() -> (Vec<(Display, u16)>, Vec<Monitor>) {
    let mut found = Display::enumerate();
    // On Linux the id is the /dev/i2c-N device number, so this sorts by bus
    // number (matches ddcutil's order); elsewhere it sorts by name.
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
        let Ok(value) = retry(3, || d.handle.get_vcp_feature(BRIGHTNESS)) else {
            continue;
        };
        let max = value.maximum().max(1);
        // Windows (dxva2) gives no EDID, only "Generic PnP Monitor"; ask the
        // monitor for its model name instead.
        if d.info.model_name.is_none() {
            let _ = d.update_capabilities();
        }

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

    // Identical models: add the end of the serial so the sliders can be told apart
    let names: Vec<String> = monitors.iter().map(|m| m.name.clone()).collect();
    for (n, (m, (d, _))) in monitors.iter_mut().zip(&displays).enumerate() {
        if names.iter().filter(|&name| *name == m.name).count() > 1 {
            let serial: Vec<char> = display_serial(&d.info).chars().collect();
            let tail: String = serial[serial.len().saturating_sub(4)..].iter().collect();
            let tail = if tail.is_empty() {
                (n + 1).to_string()
            } else {
                tail
            };
            m.name = format!("{} ({tail})", m.name);
        }
    }
    (displays, monitors)
}

fn display_name(info: &DisplayInfo) -> String {
    match (&info.manufacturer_id, &info.model_name) {
        // "DEL" + "DELL U2518D" would repeat the brand
        (Some(mfr), Some(model)) if !model.to_uppercase().starts_with(&mfr.to_uppercase()) => {
            format!("{mfr} {model}")
        }
        (_, Some(model)) => model.clone(),
        _ => info.id.clone(),
    }
}

fn display_serial(info: &DisplayInfo) -> String {
    info.serial_number
        .clone()
        .or(info.serial.map(|s| s.to_string()))
        .unwrap_or_default()
}

fn display_key(info: &DisplayInfo) -> String {
    format!(
        "{}|{}|{}|{}",
        info.manufacturer_id.as_deref().unwrap_or(""),
        info.model_name.as_deref().unwrap_or(""),
        info.model_id.unwrap_or(0),
        display_serial(info)
    )
}

/// Why no monitor was found, and whether "Set up access" can fix it.
fn no_monitor_hint(setup_tried: bool) -> (String, bool) {
    #[cfg(target_os = "linux")]
    match setup::check() {
        setup::Access::Ok => {}
        _ if setup_tried => {
            return (
                "Still no access to the monitors. Log out and back in, or see the README \
                 for manual setup."
                    .into(),
                false,
            );
        }
        setup::Access::NoDevices | setup::Access::NoPermission => {
            return (
                "Brightness needs a one-time setup to talk to your monitors.".into(),
                true,
            );
        }
    }
    let _ = setup_tried;
    (
        "No monitors with DDC/CI found. Check that DDC/CI is enabled in the monitor's menu."
            .into(),
        false,
    )
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

// Colors from assets/logo.svg
const NIGHT: egui::Color32 = egui::Color32::from_rgb(0x1e, 0x14, 0x50);
const CARD: egui::Color32 = egui::Color32::from_rgb(0x2b, 0x20, 0x62);
const WIDGET: egui::Color32 = egui::Color32::from_rgb(0x3a, 0x2f, 0x66);
const WIDGET_HOVER: egui::Color32 = egui::Color32::from_rgb(0x4d, 0x3f, 0x88);
const VIOLET: egui::Color32 = egui::Color32::from_rgb(0x9b, 0x5c, 0xf6);
const SUN: egui::Color32 = egui::Color32::from_rgb(0xff, 0xd2, 0x3f);
const INK: egui::Color32 = egui::Color32::from_rgb(0x14, 0x0c, 0x2a);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xec, 0xe8, 0xff);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0xa9, 0x9f, 0xd6);

/// Small sun like the one in the logo. Drawn instead of "☀", which the
/// built-in fonts don't have.
fn sun_icon(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let c = rect.center();
    let painter = ui.painter();
    painter.circle_filled(c, size * 0.22, SUN);
    let stroke = egui::Stroke::new(size * 0.09, SUN);
    for k in 0..8 {
        let dir = egui::Vec2::angled(k as f32 * std::f32::consts::FRAC_PI_4);
        painter.line_segment([c + dir * size * 0.34, c + dir * size * 0.48], stroke);
    }
}

fn set_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.slider_rail_height = 6.0;

        let v = &mut style.visuals;
        v.panel_fill = NIGHT;
        v.window_fill = NIGHT;
        v.extreme_bg_color = INK; // background of the value box next to a slider
        v.selection.bg_fill = SUN; // filled part of a slider, active preset, "Linked"
        v.selection.stroke = egui::Stroke::new(1.0, INK);
        v.slider_trailing_fill = true;
        v.warn_fg_color = egui::Color32::from_rgb(0xff, 0xb8, 0x00);
        v.widgets.noninteractive.fg_stroke.color = TEXT;
        v.widgets.noninteractive.bg_stroke.color = WIDGET;
        for (w, fill) in [
            (&mut v.widgets.inactive, WIDGET),
            (&mut v.widgets.hovered, WIDGET_HOVER),
            (&mut v.widgets.active, VIOLET),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.bg_stroke = egui::Stroke::NONE;
            w.fg_stroke.color = TEXT;
            w.corner_radius = egui::CornerRadius::same(8);
        }
    });
}

struct App {
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
    monitors: Vec<Monitor>,
    linked: HashSet<String>, // keys of monitors whose sliders move together
    scanning: bool,
    status: Option<String>,
    offer_setup: bool, // show the "Set up access" button (Linux)
    setting_up: bool,
    setup_tried: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        set_style(&cc.egui_ctx);
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
            offer_setup: false,
            setting_up: false,
            setup_tried: false,
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

    /// Whether all monitors are at the values of this preset.
    fn preset_active(&self, linked: f32, other: f32) -> bool {
        !self.monitors.is_empty()
            && (0..self.monitors.len()).all(|j| {
                self.monitors[j].percent == if self.is_linked(j) { linked } else { other }
            })
    }

    /// One monitor: name, link toggle and slider. Returns true if the link changed.
    fn monitor_card(&mut self, ui: &mut egui::Ui, i: usize) -> bool {
        let linked = self.is_linked(i);
        let mut link_changed = false;
        egui::Frame::new()
            .fill(CARD)
            .stroke(egui::Stroke::new(1.5, if linked { SUN } else { WIDGET }))
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&self.monitors[i].name).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(if linked { "Linked" } else { "Link" })
                                    .selected(linked),
                            )
                            .on_hover_text("Linked monitors share one brightness value")
                            .clicked()
                        {
                            link_changed = true;
                            self.toggle_link(i);
                        }
                    });
                });

                ui.spacing_mut().slider_width = ui.available_width() - 64.0;
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
            });
        link_changed
    }

    fn toggle_link(&mut self, i: usize) {
        let key = self.monitors[i].key.clone();
        if self.linked.remove(&key) {
            return;
        }
        // Join the group at the group's current value
        let group_value = (0..self.monitors.len())
            .find(|&j| self.is_linked(j))
            .map(|j| self.monitors[j].percent);
        self.linked.insert(key);
        if let Some(v) = group_value {
            self.set(i, v);
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
                    self.setting_up = false;
                    self.offer_setup = false;
                    self.status = None;
                    if monitors.is_empty() {
                        let (hint, offer_setup) = no_monitor_hint(self.setup_tried);
                        self.status = Some(hint);
                        self.offer_setup = offer_setup;
                    }
                    self.monitors = monitors;
                    for m in &mut self.monitors {
                        m.percent = m.percent.round();
                    }
                }
                Event::SetFailed(i, e) => {
                    let name = self.monitors.get(i).map_or("Monitor", |m| m.name.as_str());
                    self.status = Some(format!("{name}: {e}"));
                }
                #[cfg(target_os = "linux")]
                Event::SetupFailed(e) => {
                    self.setting_up = false;
                    self.status = Some(e);
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.handle_events();
        let mut links_changed = false;

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(NIGHT).inner_margin(16))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    sun_icon(ui, 22.0);
                    ui.label(egui::RichText::new("Brightness").size(20.0).strong().color(SUN));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.scanning && !self.setting_up,
                                egui::Button::new("Rescan"),
                            )
                            .on_hover_text("Search for monitors again")
                            .clicked()
                        {
                            self.scanning = true;
                            let _ = self.tx.send(Cmd::Scan);
                        }
                    });
                });
                ui.add_space(6.0);

                if self.setting_up {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Setting up access, enter your password in the dialog…");
                    });
                } else if self.scanning {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Searching for monitors…");
                    });
                } else {
                    if !self.monitors.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new("Presets").color(MUTED));
                            for &(linked, other) in PRESETS {
                                let active = self.preset_active(linked as f32, other as f32);
                                if ui
                                    .add(
                                        egui::Button::new(format!("{linked} / {other}"))
                                            .selected(active),
                                    )
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
                    }

                    for i in 0..self.monitors.len() {
                        if self.monitor_card(ui, i) {
                            links_changed = true;
                        }
                        ui.add_space(4.0);
                    }
                }

                if let Some(status) = &self.status {
                    ui.add_space(4.0);
                    ui.colored_label(ui.visuals().warn_fg_color, status);
                }

                #[cfg(target_os = "linux")]
                if self.offer_setup && !self.scanning && !self.setting_up {
                    ui.add_space(4.0);
                    if ui
                        .add(egui::Button::new("Set up access").selected(true))
                        .on_hover_text(
                            "Asks for your password once, then lets this app (and you) \
                             change monitor brightness",
                        )
                        .clicked()
                    {
                        self.setting_up = true;
                        self.setup_tried = true;
                        let _ = self.tx.send(Cmd::Setup);
                    }
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
