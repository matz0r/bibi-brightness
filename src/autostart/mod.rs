// ---------------------------------------------------------------------------
// Start on login, toggled from the "Autostart" button. Starts this binary
// with --tray, so it sits in the tray without opening the window.
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::{enabled, set};
#[cfg(windows)]
pub use windows::{enabled, set};
