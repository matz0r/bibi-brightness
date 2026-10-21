// ---------------------------------------------------------------------------
// Tray icon (Linux, StatusNotifierItem over D-Bus): KDE, Cinnamon, XFCE,
// MATE, LXQt, Budgie, Waybar, GNOME with the AppIndicator extension.
//
// The window runs as a child process (the same binary without --tray):
// winit allows only one event loop per process and can't hide a window on
// Wayland, so "closing to the tray" means ending that process.
// ---------------------------------------------------------------------------

use ksni::blocking::TrayMethods;
use std::process::{Child, Command};
use std::sync::mpsc::{self, Sender};

struct Tray {
    window: Option<Child>,
    quit: Sender<()>,
}

impl Tray {
    fn window_open(&mut self) -> bool {
        // try_wait also reaps a window the user closed
        matches!(self.window.as_mut().map(Child::try_wait), Some(Ok(None)))
    }

    fn open_window(&mut self) {
        if self.window_open() {
            return;
        }
        match std::env::current_exe().and_then(|exe| Command::new(exe).spawn()) {
            Ok(child) => self.window = Some(child),
            Err(e) => eprintln!("brightness: can't start the window: {e}"),
        }
    }

    fn close_window(&mut self) {
        // Links are saved as soon as they change, so killing loses nothing
        if self.window_open()
            && let Some(mut child) = self.window.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl ksni::Tray for Tray {
    const MENU_ON_ACTIVATE: bool = false;

    fn id(&self) -> String {
        "bibi-brightness".into()
    }

    fn title(&self) -> String {
        "BIBI Brightness".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        icon_pixmaps()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "BIBI Brightness".into(),
            description: "Click to show or hide the sliders".into(),
            ..Default::default()
        }
    }

    /// Left click: show the window, or close it if it is open
    fn activate(&mut self, _x: i32, _y: i32) {
        if self.window_open() {
            self.close_window();
        } else {
            self.open_window();
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Open".into(),
                activate: Box::new(|this: &mut Self| this.open_window()),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.quit.send(());
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// The app icon in a few sizes, so the panel doesn't have to scale 256 px down.
fn icon_pixmaps() -> Vec<ksni::Icon> {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("bundled icon is a valid PNG")
        .into_rgba8();
    [16, 22, 24, 32, 48, 64]
        .into_iter()
        .map(|size| {
            let scaled =
                image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3);
            // SNI wants ARGB32 in network byte order
            let data = scaled
                .pixels()
                .flat_map(|p| [p[3], p[0], p[1], p[2]])
                .collect();
            ksni::Icon {
                width: size as i32,
                height: size as i32,
                data,
            }
        })
        .collect()
}

/// Runs until "Quit" is chosen. Err if the desktop has no SNI tray.
pub fn run() -> Result<(), ksni::Error> {
    let (quit_tx, quit_rx) = mpsc::channel();
    let handle = Tray {
        window: None,
        quit: quit_tx,
    }
    .spawn()?;
    let _ = quit_rx.recv();
    // An open window stays open
    handle.shutdown().wait();
    Ok(())
}
