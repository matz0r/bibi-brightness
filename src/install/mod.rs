//! Per-user installation and version detection. Never execute an unknown
//! installed binary to query its version: older builds don't support --version.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(windows)]
use windows as platform;

pub use platform::install;
use std::path::Path;

pub fn has_installed_copy() -> bool {
    platform::exe_path().is_ok_and(|p| p.is_file())
}

pub fn available() -> bool {
    let Ok(target) = platform::exe_path() else {
        return true;
    };
    let Ok(exe) = std::env::current_exe() else {
        return true;
    };
    needs_install(&exe, &target, platform::integration_exists())
}

fn needs_install(exe: &Path, target: &Path, integrated: bool) -> bool {
    if !target.is_file() {
        return true;
    }
    if let Ok(version) = std::fs::read_to_string(target.with_extension("version"))
        && let Ok(installed) = semver::Version::parse(version.trim())
        && installed > semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
    {
        return false;
    }
    if !integrated {
        return true;
    }
    if std::fs::canonicalize(exe).ok() == std::fs::canonicalize(target).ok() {
        return false;
    }
    // Also offer installation for a rebuilt binary with the same package version.
    match (std::fs::read(exe), std::fs::read(target)) {
        (Ok(a), Ok(b)) => a != b,
        _ => true,
    }
}

pub(super) fn record_version(target: &Path) -> Result<(), String> {
    std::fs::write(target.with_extension("version"), env!("CARGO_PKG_VERSION"))
        .map_err(|e| format!("Can't save the installed version: {e}"))
}

/// Wait for the replacement tray to initialize before closing the old UI.
/// A successful spawn alone doesn't establish that the program actually runs.
pub(super) fn restart(target: &Path) -> Result<(), String> {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let mut command = Command::new(target);
    command.arg("--report-ready");
    if std::env::args().any(|arg| arg == "--tray") {
        command.arg("--tray");
    }
    let mut child = command
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Installed, but can't start {}: {e}", target.display()))?;
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    if rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .is_ok_and(|s| s.trim() == "ready")
    {
        return Ok(());
    }
    let _ = child.kill();
    let _ = child.wait();
    // Restore access to the old UI when the replacement could not initialize.
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("--tray").spawn();
    }
    Err("The installed copy could not start. The current window has been kept open.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_missing_equal_rebuilt_newer_and_incomplete_installations() {
        let dir = std::env::temp_dir().join(format!("bibi-install-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("download");
        let target = dir.join("brightness");
        std::fs::write(&exe, b"new").unwrap();
        assert!(needs_install(&exe, &target, true));
        std::fs::write(&target, b"new").unwrap();
        assert!(!needs_install(&exe, &target, true));
        assert!(needs_install(&exe, &target, false));
        std::fs::write(&target, b"old").unwrap();
        assert!(needs_install(&exe, &target, true));
        record_version(&target).unwrap();
        assert!(needs_install(&exe, &target, true));
        std::fs::write(target.with_extension("version"), "999.0.0").unwrap();
        assert!(!needs_install(&exe, &target, true));
        assert!(!needs_install(&target, &target, true));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
