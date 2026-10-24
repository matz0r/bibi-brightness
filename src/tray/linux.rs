// Linux tray icon (StatusNotifierItem over D-Bus): KDE, Cinnamon, XFCE, MATE,
// LXQt, Budgie, Waybar, GNOME with the AppIndicator extension. A second start
// finds the running tray through a socket in $XDG_RUNTIME_DIR.

use ksni::blocking::TrayMethods;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

use super::Window;

struct Tray {
    window: Window,
    quit: Sender<()>,
}

impl Tray {
    fn open_window(&mut self) {
        self.window.open();
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
        self.window.toggle();
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
    let img = image::load_from_memory(include_bytes!("../../assets/icon.png"))
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

fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("bibi-brightness.sock"))
}

/// Tells a tray that is already running to open the window (or nothing, for
/// --tray). Returns false if there is none.
fn notify_running(show_window: bool) -> bool {
    let Some(path) = socket_path() else {
        return false;
    };
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    let _ = stream.write_all(if show_window { b"open" } else { b"" });
    true
}

/// Runs until "Quit" is chosen. Err if the desktop has no SNI tray.
pub fn run(show_window: bool) -> Result<(), ksni::Error> {
    if notify_running(show_window) {
        return Ok(());
    }

    let (quit_tx, quit_rx) = mpsc::channel();
    let handle = Tray {
        window: Window::default(),
        quit: quit_tx,
    }
    .spawn()?;
    if show_window {
        handle.update(Tray::open_window);
    }

    if let Some(path) = socket_path() {
        let _ = std::fs::remove_file(&path); // left over from a crashed tray
        match UnixListener::bind(&path) {
            Ok(listener) => {
                let handle = handle.clone();
                thread::spawn(move || {
                    for mut stream in listener.incoming().flatten() {
                        let mut msg = String::new();
                        let _ = stream.read_to_string(&mut msg);
                        if msg == "open" {
                            handle.update(Tray::open_window);
                        }
                    }
                });
            }
            Err(e) => eprintln!("brightness: can't listen on {}: {e}", path.display()),
        }
    }

    let _ = quit_rx.recv();
    // An open window stays open
    handle.shutdown().wait();
    if let Some(path) = socket_path() {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}
