use crate::{autostart, tray};
use std::path::PathBuf;

fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| "Can't find the user data folder.".into())
}

pub(super) fn exe_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("$HOME is not set.")?;
    Ok(PathBuf::from(home).join(".local/bin/brightness"))
}

pub(super) fn integration_exists() -> bool {
    data_dir().is_ok_and(|dir| {
        dir.join("applications/brightness.desktop").is_file()
            && dir
                .join("icons/hicolor/256x256/apps/bibi-brightness.png")
                .is_file()
    })
}

pub fn install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = exe_path()?;
    let data = data_dir()?;
    let desktop = data.join("applications/brightness.desktop");
    let icon = data.join("icons/hicolor/256x256/apps/bibi-brightness.png");
    for path in [&target, &desktop, &icon] {
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    }
    // Copy then rename: Linux cannot overwrite an executable that is running.
    let staged = target.with_extension(format!("new-{}", std::process::id()));
    std::fs::copy(&exe, &staged).map_err(|e| format!("Can't stage the installed binary: {e}"))?;
    std::fs::write(&icon, include_bytes!("../../assets/icon.png")).map_err(|e| e.to_string())?;
    let entry = format!(
        "[Desktop Entry]\nName=BIBI Brightness\nComment=Monitor brightness sliders (DDC/CI)\nExec={}\nIcon=bibi-brightness\nType=Application\nCategories=Settings;\nStartupWMClass=brightness\n",
        autostart::exec_quote(&target.to_string_lossy())
    );
    std::fs::write(&desktop, entry).map_err(|e| format!("Can't write the menu entry: {e}"))?;
    let previous_autostart = autostart::registered();
    std::fs::rename(&staged, &target)
        .map_err(|e| format!("Can't install {}: {e}", target.display()))?;
    super::record_version(&target)?;
    if previous_autostart {
        autostart::set_exe(Some(&target))?;
    }
    tray::quit_running()?;
    super::restart(&target)
}
