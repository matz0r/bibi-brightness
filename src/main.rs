// Hide the console window on Windows; no effect on Linux.
#![windows_subsystem = "windows"]

use ddc_hi::{Ddc, Display, DisplayInfo};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[cfg(any(target_os = "linux", windows))]
mod autostart;
#[cfg(windows)]
mod install;
#[cfg(target_os = "linux")]
mod setup;
#[cfg(any(target_os = "linux", windows))]
mod tray;

const BRIGHTNESS: u8 = 0x10; // VCP feature code for brightness

/// Brightness presets: (linked monitors, other monitors) in percent.
const PRESETS: &[(u8, u8)] = &[(45, 20), (67, 25), (75, 33), (80, 40), (90, 45)];

/// What the UI knows about a monitor.
struct Monitor {
    key: String,  // stable identity (model + serial), used to remember links
    name: String, // shown in the UI
    slot: String, // where it was found (position + system id), see `Known`
    percent: f32,
}

/// Key and name of the monitor last seen in each slot, so a scan can skip
/// asking the monitor for its name (slow on Windows). The name is checked
/// again in the background after the sliders are shown.
type Known = HashMap<String, (String, String)>;

/// UI -> worker
enum Cmd {
    /// Find the monitors. An empty `Known` asks every monitor for its name.
    Scan(Known),
    Set(usize, f32),
    /// Linux: one-time i2c access setup, then rescan
    #[cfg(target_os = "linux")]
    Setup,
}

/// worker -> UI
enum Event {
    Scanning,
    Found(Vec<Monitor>),
    /// Corrected (key, name) of every monitor after a background name check
    Names(Vec<(String, String)>),
    SetFailed(usize, String),
    #[cfg(target_os = "linux")]
    SetupFailed(String),
}

fn main() -> eframe::Result {
    // Icon in the panel (Linux) or notification area (Windows); closing the
    // window keeps it running there. --tray starts hidden (autostart),
    // --window is the window process the tray starts.
    #[cfg(any(target_os = "linux", windows))]
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
    let mut names: Vec<(String, String)> = Vec::new(); // (key, name) per display
    let mut sent_names = Vec::new(); // what the UI has
    let mut unchecked: Vec<usize> = Vec::new(); // names taken from `Known`

    loop {
        // Check one name at a time while idle, so slider moves don't wait
        // for all of them
        let first = if unchecked.is_empty() {
            match rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => return,
            }
        } else {
            match rx.try_recv() {
                Ok(cmd) => cmd,
                Err(mpsc::TryRecvError::Empty) => {
                    let i = unchecked.remove(0);
                    let d = &mut displays[i].0;
                    let _ = d.update_capabilities();
                    let mut raw: Vec<_> =
                        names.iter().map(|(k, n)| (k.clone(), n.clone())).collect();
                    raw[i] = (display_key(&d.info), display_name(&d.info));
                    // Send once all are checked; in between, identical
                    // models would flicker between names
                    let checked = unique_names(raw, &displays);
                    names = checked;
                    if unchecked.is_empty() && names != sent_names {
                        sent_names = names.clone();
                        let _ = tx.send(Event::Names(names.clone()));
                        ctx.request_repaint();
                    }
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        };

        let mut scan = None;
        #[cfg(target_os = "linux")]
        let mut setup = false;
        let mut sets: HashMap<usize, f32> = HashMap::new();
        for cmd in std::iter::once(first).chain(rx.try_iter()) {
            match cmd {
                Cmd::Scan(known) => scan = Some(known),
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
            scan = Some(Known::new());
        }

        if let Some(known) = scan {
            let _ = tx.send(Event::Scanning);
            ctx.request_repaint();
            let (found, infos, from_known) = scan_displays(&known);
            displays = found;
            names = infos
                .iter()
                .map(|m| (m.key.clone(), m.name.clone()))
                .collect();
            sent_names = names.clone();
            unchecked = from_known;
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

/// Returns the monitors that answer DDC/CI brightness, and the indices of
/// those whose name came from `known` instead of the monitor.
fn scan_displays(known: &Known) -> (Vec<(Display, u16)>, Vec<Monitor>, Vec<usize>) {
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
    let mut raw = Vec::new(); // (key, name) before telling identical models apart
    let mut slots = Vec::new();
    let mut percents = Vec::new();
    let mut from_known = Vec::new();
    for (n, mut d) in found.into_iter().enumerate() {
        // Skip outputs that don't answer DDC/CI brightness (laptop panels, some TVs, …)
        let Ok(value) = retry(3, || d.handle.get_vcp_feature(BRIGHTNESS)) else {
            continue;
        };
        let max = value.maximum().max(1);
        // Windows (dxva2) gives no EDID, only "Generic PnP Monitor"; ask the
        // monitor for its model name instead. That takes a second or two per
        // monitor, so use the name from last time if this slot had one and
        // check it later.
        let slot = format!("{n}|{}", d.info.id);
        match known.get(&slot) {
            Some(key_name) if d.info.model_name.is_none() => {
                from_known.push(displays.len());
                raw.push(key_name.clone());
            }
            _ => {
                if d.info.model_name.is_none() {
                    let _ = d.update_capabilities();
                }
                raw.push((display_key(&d.info), display_name(&d.info)));
            }
        }
        slots.push(slot);
        percents.push(value.value() as f32 * 100.0 / max as f32);
        displays.push((d, max));
    }

    let monitors = unique_names(raw, &displays)
        .into_iter()
        .zip(slots)
        .zip(percents)
        .map(|(((key, name), slot), percent)| Monitor {
            key,
            name,
            slot,
            percent,
        })
        .collect();
    (displays, monitors, from_known)
}

/// Tells identical models apart: a running number on the key (when there is
/// no serial) and the end of the serial on the name.
fn unique_names(
    mut raw: Vec<(String, String)>,
    displays: &[(Display, u16)],
) -> Vec<(String, String)> {
    for i in 0..raw.len() {
        if raw[..i].iter().any(|(key, _)| *key == raw[i].0) {
            raw[i].0 = format!("{}#{i}", raw[i].0);
        }
    }
    let names: Vec<String> = raw.iter().map(|(_, name)| name.clone()).collect();
    for (n, ((_, name), (d, _))) in raw.iter_mut().zip(displays).enumerate() {
        if names.iter().filter(|&other| other == name).count() > 1 {
            let serial: Vec<char> = display_serial(&d.info).chars().collect();
            let tail: String = serial[serial.len().saturating_sub(4)..].iter().collect();
            let tail = if tail.is_empty() {
                (n + 1).to_string()
            } else {
                tail
            };
            *name = format!("{name} ({tail})");
        }
    }
    raw
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
        "No monitors with DDC/CI found. Check that DDC/CI is enabled in the monitor's menu.".into(),
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
    /// Showing the monitors from last run while the scan confirms them
    refreshing: bool,
    /// Keys of monitors moved while refreshing; sent once the scan is done
    touched: HashSet<String>,
    status: Option<String>,
    offer_setup: bool, // show the "Set up access" button (Linux)
    setting_up: bool,
    setup_tried: bool,
    #[cfg(any(target_os = "linux", windows))]
    autostart: bool,
    /// Windows: running from the installed copy, so no Install button
    #[cfg(windows)]
    installed: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        set_style(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        thread::spawn(move || worker(cmd_rx, ev_tx, ctx));

        // Links and monitors saved from last run (~/.local/share/brightness/
        // on Linux, %APPDATA%\brightness\ on Windows)
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
        // Shown right away; the scan takes seconds and corrects them
        let monitors: Vec<Monitor> = cc
            .storage
            .and_then(|s| s.get_string("monitors"))
            .map(|s| s.lines().filter_map(parse_monitor).collect())
            .unwrap_or_default();
        let known = monitors
            .iter()
            .map(|m| (m.slot.clone(), (m.key.clone(), m.name.clone())))
            .collect();
        let _ = cmd_tx.send(Cmd::Scan(known));

        Self {
            tx: cmd_tx,
            rx: ev_rx,
            scanning: monitors.is_empty(),
            refreshing: !monitors.is_empty(),
            touched: HashSet::new(),
            monitors,
            linked,
            status: None,
            offer_setup: false,
            setting_up: false,
            setup_tried: false,
            #[cfg(any(target_os = "linux", windows))]
            autostart: autostart::enabled(),
            #[cfg(windows)]
            installed: install::is_installed(),
        }
    }

    /// Sends a slider value to the monitor, or keeps it for after the scan
    fn send_set(&mut self, j: usize, value: f32) {
        self.monitors[j].percent = value;
        if self.refreshing {
            self.touched.insert(self.monitors[j].key.clone());
        } else {
            let _ = self.tx.send(Cmd::Set(j, value));
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
                self.send_set(j, value);
            }
        }
    }

    /// Linked monitors get `linked`, all others get `other`.
    fn apply_preset(&mut self, linked: f32, other: f32) {
        for j in 0..self.monitors.len() {
            let v = if self.is_linked(j) { linked } else { other };
            self.send_set(j, v);
        }
    }

    /// Whether all monitors are at the values of this preset.
    fn preset_active(&self, linked: f32, other: f32) -> bool {
        !self.monitors.is_empty()
            && (0..self.monitors.len())
                .all(|j| self.monitors[j].percent == if self.is_linked(j) { linked } else { other })
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

    /// Returns true if the monitor list changed and should be saved.
    fn handle_events(&mut self) -> bool {
        let mut changed = false;
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Scanning => {
                    // Last run's sliders stay usable while refreshing
                    self.scanning = !self.refreshing;
                    self.status = None;
                }
                Event::Found(mut monitors) => {
                    changed = true;
                    for m in &mut monitors {
                        m.percent = m.percent.round();
                    }
                    // What was moved while refreshing wins over what the
                    // monitor reported
                    let moved: Vec<(usize, f32)> = monitors
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| self.touched.contains(&m.key))
                        .filter_map(|(j, m)| {
                            let old = self.monitors.iter().find(|o| o.key == m.key)?;
                            Some((j, old.percent))
                        })
                        .collect();
                    self.touched.clear();
                    self.refreshing = false;
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
                    for (j, v) in moved {
                        self.send_set(j, v);
                    }
                }
                Event::Names(names) => {
                    changed = true;
                    for (m, (key, name)) in self.monitors.iter_mut().zip(names) {
                        m.key = key;
                        m.name = name;
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
        changed
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let mut save_now = self.handle_events();

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(NIGHT).inner_margin(16))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    sun_icon(ui, 22.0);
                    ui.label(
                        egui::RichText::new("Brightness")
                            .size(20.0)
                            .strong()
                            .color(SUN),
                    );
                    if self.refreshing {
                        ui.spinner().on_hover_text("Checking the monitors…");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.scanning && !self.refreshing && !self.setting_up,
                                egui::Button::new("Rescan"),
                            )
                            .on_hover_text("Search for monitors again")
                            .clicked()
                        {
                            // Asks every monitor for its name again, in
                            // case monitors were swapped between ports
                            self.scanning = true;
                            let _ = self.tx.send(Cmd::Scan(Known::new()));
                        }

                        #[cfg(any(target_os = "linux", windows))]
                        if ui
                            .add(egui::Button::new("Autostart").selected(self.autostart))
                            .on_hover_text(if self.autostart {
                                "Starts in the tray when you log in. Click to turn off"
                            } else {
                                "Start in the tray when you log in"
                            })
                            .clicked()
                        {
                            match autostart::set(!self.autostart) {
                                Ok(()) => self.autostart = !self.autostart,
                                Err(e) => self.status = Some(e),
                            }
                        }

                        #[cfg(windows)]
                        if !self.installed
                            && ui
                                .add(egui::Button::new("Install"))
                                .on_hover_text(if install::has_installed_copy() {
                                    "Replace the installed copy with this version"
                                } else {
                                    "Copy to your user folder and add to the Start menu, \
                                     so it keeps working when you delete the download"
                                })
                                .clicked()
                        {
                            match install::install() {
                                // The installed copy opens its own window
                                Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                                Err(e) => self.status = Some(e),
                            }
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
                            save_now = true;
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

        // Save right away: the tray ends this process instead of closing it
        if save_now && let Some(storage) = frame.storage_mut() {
            self.save(storage);
            storage.flush();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let mut keys: Vec<_> = self.linked.iter().cloned().collect();
        keys.sort();
        storage.set_string("linked", keys.join("\n"));
        let monitors: Vec<String> = self
            .monitors
            .iter()
            .map(|m| format!("{}\t{}\t{}\t{}", m.slot, m.key, m.name, m.percent))
            .collect();
        storage.set_string("monitors", monitors.join("\n"));
    }
}

/// One line of the saved monitor list: slot, key, name, percent.
fn parse_monitor(line: &str) -> Option<Monitor> {
    let mut fields = line.split('\t');
    let (slot, key, name, percent) = (
        fields.next()?,
        fields.next()?,
        fields.next()?,
        fields.next()?,
    );
    Some(Monitor {
        slot: slot.into(),
        key: key.into(),
        name: name.into(),
        percent: percent.parse().ok()?,
    })
}
