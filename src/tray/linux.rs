// Linux tray icon (StatusNotifierItem over D-Bus): KDE, Cinnamon, XFCE, MATE,
// LXQt, Budgie, Waybar, GNOME with the AppIndicator extension. A second start
// finds the running tray through a socket in $XDG_RUNTIME_DIR.

use ksni::blocking::TrayMethods;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use super::Window;

struct Tray {
    window: Window,
    quit: Sender<()>,
    online: Arc<AtomicBool>,
}

impl Tray {
    fn open_window(&mut self) {
        self.window.open();
    }
}

impl ksni::Tray for Tray {
    const MENU_ON_ACTIVATE: bool = false;

    fn watcher_online(&self) {
        self.online.store(true, Ordering::Relaxed);
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        eprintln!("brightness: waiting for the system tray: {reason:?}");
        self.online.store(false, Ordering::Relaxed);
        true
    }

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

/// A bounded local request. A hung peer must not freeze the UI or installation.
fn request(message: &str) -> Option<String> {
    let mut stream = UnixStream::connect(socket_path()?).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(if message == "replace" {
            5
        } else {
            1
        })))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    stream.write_all(message.as_bytes()).ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok()?;
    let mut reply = String::new();
    stream.take(4096).read_to_string(&mut reply).ok()?;
    Some(reply)
}

pub fn available() -> bool {
    request("status").as_deref() == Some("online")
}

/// Installation leaves its caller window alive until the new process starts.
pub fn quit_running() -> Result<(), String> {
    if let Some(reply) = request("replace") {
        if reply == "bye" {
            return Ok(());
        }
        return Err(
            "The running tray did not acknowledge replacement. Quit it and try again.".into(),
        );
    }
    if socket_path().is_some_and(|p| UnixStream::connect(p).is_ok()) {
        return Err("The running tray is not responding. Quit it and try again.".into());
    }
    Ok(())
}

struct SocketGuard(PathBuf);
impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Keep the SNI service alive while the desktop starts or restarts. A lock
/// serializes stale-socket cleanup and prevents simultaneous launches racing.
pub fn run(show_window: bool) -> Result<(), String> {
    let path = socket_path().ok_or("$XDG_RUNTIME_DIR is not set")?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|e| e.to_string())?;
    if let Err(error) = lock.try_lock() {
        if matches!(error, std::fs::TryLockError::WouldBlock) {
            // The owner can still be registering its tray; wait for its socket.
            let start = Instant::now();
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let message = if show_window {
                format!("open\n{}", exe.display())
            } else {
                "ping".into()
            };
            while start.elapsed() < Duration::from_secs(5) {
                if request(&message).is_some() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            return Err("The running tray is not responding".into());
        }
        return Err(error.to_string());
    }
    if UnixStream::connect(&path).is_ok() {
        // A release predating the lock and replacement protocol may still run.
        // Do not unlink its live socket and leave two tray icons running.
        if !show_window {
            return Ok(());
        }
        return Err(
            "An older BIBI tray is running. Quit it from its menu before installing this version."
                .into(),
        );
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    let guard = SocketGuard(path);
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let (quit_tx, quit_rx) = mpsc::channel();
    let online = Arc::new(AtomicBool::new(true));
    let handle = Tray {
        window: Window::default(),
        quit: quit_tx,
        online: online.clone(),
    }
    .assume_sni_available(true)
    .spawn()
    .map_err(|e| e.to_string())?;
    if show_window {
        handle.update(Tray::open_window);
    }
    super::report_ready();
    let mut offline_since = None;
    let mut fallback_shown = false;
    let mut replacement = None;
    loop {
        if quit_rx.try_recv().is_ok() {
            handle.update(|tray| tray.window.close());
            break;
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                let mut msg = String::new();
                if (&mut stream).take(4096).read_to_string(&mut msg).is_err() {
                    continue;
                }
                if msg == "replace" {
                    replacement = Some(stream);
                    break;
                }
                if msg == "status" {
                    let _ = stream.write_all(if online.load(Ordering::Relaxed) {
                        b"online"
                    } else {
                        b"offline"
                    });
                } else if let Some(exe) = msg.strip_prefix("open\n") {
                    let exe = PathBuf::from(exe);
                    if exe.is_absolute() && exe.is_file() {
                        handle.update(move |tray| {
                            // A new download must show its own Install button,
                            // rather than reopening the previously installed UI.
                            if std::env::current_exe().ok().as_ref() != Some(&exe) {
                                tray.window.close();
                            }
                            tray.window.open_from(&exe);
                        });
                    }
                    let _ = stream.write_all(b"ok");
                } else {
                    if msg == "open" {
                        handle.update(Tray::open_window);
                    }
                    let _ = stream.write_all(b"ok");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => {
                eprintln!("brightness: tray socket: {e}");
                break;
            }
        }
        if online.load(Ordering::Relaxed) {
            offline_since = None;
            fallback_shown = false;
        } else {
            let since = offline_since.get_or_insert_with(Instant::now);
            if !fallback_shown && since.elapsed() >= Duration::from_secs(15) {
                handle.update(Tray::open_window);
                fallback_shown = true;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    handle.shutdown().wait();
    drop(listener);
    drop(guard);
    drop(lock);
    if let Some(mut stream) = replacement {
        let _ = stream.write_all(b"bye");
    }
    Ok(())
}
