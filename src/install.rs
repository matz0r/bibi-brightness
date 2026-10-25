// ---------------------------------------------------------------------------
// Windows: the "Install" button. Copies brightness.exe to
// %LOCALAPPDATA%\Programs\BIBI Brightness\ (per user, no admin), adds a
// Start menu shortcut and moves autostart there, so deleting the download
// doesn't break anything. Then it ends the tray running from the download
// and starts the installed copy.
// ---------------------------------------------------------------------------

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{autostart, tray};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn env_dir(name: &str) -> Result<PathBuf, String> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| format!("Can't find the folder %{name}%."))
}

/// Where the installed copy lives
fn exe_path() -> Result<PathBuf, String> {
    Ok(env_dir("LOCALAPPDATA")?
        .join("Programs")
        .join("BIBI Brightness")
        .join("brightness.exe"))
}

fn shortcut_path() -> Result<PathBuf, String> {
    Ok(env_dir("APPDATA")?
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join("BIBI Brightness.lnk"))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
        _ => false,
    }
}

/// True when this is the installed copy (the button is hidden then)
pub fn is_installed() -> bool {
    match (std::env::current_exe(), exe_path()) {
        (Ok(exe), Ok(target)) => same_file(&exe, &target),
        _ => false,
    }
}

/// True when an installed copy exists, so the button updates it
pub fn has_installed_copy() -> bool {
    exe_path().is_ok_and(|p| p.exists())
}

/// Installs and starts the installed copy. On Ok the caller closes this
/// window.
pub fn install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Can't find this program: {e}"))?;
    let target = exe_path()?;
    let dir = target.parent().expect("path has a parent");
    std::fs::create_dir_all(dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;

    // An older installed copy may still run (its tray or window). Windows
    // can't overwrite a running exe but can rename it, so move it aside.
    let old = target.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    if target.exists() {
        std::fs::rename(&target, &old)
            .map_err(|e| format!("Can't replace {}: {e}", target.display()))?;
    }
    if let Err(e) = std::fs::copy(&exe, &target) {
        let _ = std::fs::rename(&old, &target);
        return Err(format!("Can't copy to {}: {e}", target.display()));
    }
    // Fails while the old copy still runs; the next install removes it
    let _ = std::fs::remove_file(&old);

    create_shortcut(&shortcut_path()?, &target)?;

    if autostart::enabled() {
        autostart::set_exe(Some(&target))?;
    }

    // The tray of this (downloaded) copy would catch the new start and just
    // open this window again
    tray::quit_running()?;
    Command::new(&target)
        .spawn()
        .map_err(|e| format!("Installed, but can't start {}: {e}", target.display()))?;
    Ok(())
}

/// Start menu shortcut through PowerShell and WScript.Shell; paths go in
/// environment variables so nothing needs quoting.
fn create_shortcut(lnk: &Path, target: &Path) -> Result<(), String> {
    const SCRIPT: &str = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:BIBI_LNK); \
        $s.TargetPath = $env:BIBI_EXE; \
        $s.WorkingDirectory = Split-Path $env:BIBI_EXE; \
        $s.Description = 'Monitor brightness sliders (DDC/CI)'; \
        $s.Save()";
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("BIBI_LNK", lnk)
        .env("BIBI_EXE", target)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Can't add the Start menu shortcut: {e}"))?;
    if !output.status.success() || !lnk.exists() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Can't add the Start menu shortcut: {}",
            err.lines().next().unwrap_or("PowerShell failed")
        ));
    }
    Ok(())
}
