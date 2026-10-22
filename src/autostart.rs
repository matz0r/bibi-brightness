// ---------------------------------------------------------------------------
// Start on login (Linux), toggled from the "Autostart" button. An XDG
// autostart entry in ~/.config/autostart starts this binary with --tray, so
// it sits in the panel without opening the window.
// ---------------------------------------------------------------------------

use std::path::PathBuf;

/// Same name as the manual setup in DEVELOPMENT.md used, so the button sees
/// and replaces an entry made that way.
const FILE_NAME: &str = "brightness-tray.desktop";

fn path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("autostart").join(FILE_NAME))
}

pub fn enabled() -> bool {
    path().is_some_and(|p| p.exists())
}

pub fn set(enable: bool) -> Result<(), String> {
    let path = path().ok_or("Can't find the config folder ($HOME is not set).")?;
    if !enable {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("Can't remove {}: {e}", path.display()))
            }
            _ => Ok(()),
        };
    }

    let exe = std::env::current_exe().map_err(|e| format!("Can't find this program: {e}"))?;
    let entry = format!(
        "[Desktop Entry]\n\
         Name=Brightness\n\
         Comment=Monitor brightness sliders (DDC/CI)\n\
         Exec={} --tray\n\
         Icon=bibi-brightness\n\
         Type=Application\n\
         X-GNOME-Autostart-enabled=true\n",
        exec_quote(&exe.to_string_lossy())
    );
    let write = || {
        std::fs::create_dir_all(path.parent().expect("path has a parent"))?;
        std::fs::write(&path, entry)
    };
    write().map_err(|e| format!("Can't write {}: {e}", path.display()))
}

/// Quotes a path for an Exec line. Desktop entry spec: double quotes with
/// " ` $ \ backslash-escaped, then every backslash doubled again by the
/// file's string escaping (so a literal \ becomes four).
fn exec_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
