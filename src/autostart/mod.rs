// ---------------------------------------------------------------------------
// Start on login, toggled from the "Autostart" button. Starts this binary
// with --tray, so it sits in the tray without opening the window.
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::{enabled, exec_quote, registered, set, set_exe};
#[cfg(windows)]
pub use windows::{enabled, registered, set, set_exe};
