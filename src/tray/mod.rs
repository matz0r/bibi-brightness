// ---------------------------------------------------------------------------
// Tray icon: Linux (StatusNotifierItem) and Windows (notification area).
//
// The window runs as a child process (the same binary with --window):
// winit allows only one event loop per process and can't hide a window on
// Wayland, so "closing to the tray" means ending that process. Windows works
// the same way so both behave alike.
//
// Only one tray runs per user. A second start finds it and asks it to open
// the window instead.
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::run;
#[cfg(windows)]
pub use windows::run;

use std::process::{Child, Command};

/// The window process, if one was started.
#[derive(Default)]
struct Window(Option<Child>);

impl Window {
    fn is_open(&mut self) -> bool {
        // try_wait also reaps a window the user closed
        matches!(self.0.as_mut().map(Child::try_wait), Some(Ok(None)))
    }

    fn open(&mut self) {
        if self.is_open() {
            return;
        }
        // Windows gives the tray the right to take the focus on a click; pass
        // it on so the new window doesn't open behind others.
        #[cfg(windows)]
        unsafe {
            use ::windows_sys::Win32::UI::WindowsAndMessaging::{
                ASFW_ANY, AllowSetForegroundWindow,
            };
            AllowSetForegroundWindow(ASFW_ANY);
        }
        match std::env::current_exe().and_then(|exe| Command::new(exe).arg("--window").spawn()) {
            Ok(child) => self.0 = Some(child),
            Err(e) => eprintln!("brightness: can't start the window: {e}"),
        }
    }

    fn close(&mut self) {
        // Links are saved as soon as they change, so killing loses nothing
        if self.is_open()
            && let Some(mut child) = self.0.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Left click on the icon: show the window, or close it if it is open
    fn toggle(&mut self) {
        if self.is_open() {
            self.close();
        } else {
            self.open();
        }
    }
}
