// Linux: an XDG autostart entry in ~/.config/autostart.

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

fn entry() -> Option<String> {
    std::fs::read_to_string(path()?).ok()
}

pub fn registered() -> bool {
    entry().is_some_and(|s| active_exec(&s).is_some())
}

pub fn enabled() -> bool {
    match (entry(), std::env::current_exe()) {
        (Some(entry), Ok(exe)) => enabled_for(&entry, &exe.to_string_lossy()),
        _ => false,
    }
}

fn active_exec(entry: &str) -> Option<&str> {
    let mut desktop = false;
    let mut exec = None;
    let mut active = true;
    for line in entry.lines().map(str::trim) {
        if line.starts_with('[') {
            desktop = line == "[Desktop Entry]";
            continue;
        }
        if !desktop || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            match (key.trim(), value.trim()) {
                ("Hidden", "true") | ("X-GNOME-Autostart-enabled", "false") => active = false,
                ("Exec", value) => exec = Some(value),
                _ => {}
            }
        }
    }
    if active { exec } else { None }
}

fn enabled_for(entry: &str, exe: &str) -> bool {
    active_exec(entry).is_some_and(|command| {
        command == format!("{} --tray", exec_quote(exe))
            || (!exe.contains(char::is_whitespace)
                && !exe.contains(['"', '\\', '%', '$', '`'])
                && command == format!("{exe} --tray"))
    })
}

pub fn set(enable: bool) -> Result<(), String> {
    if !enable {
        return set_exe(None);
    }
    let exe = std::env::current_exe().map_err(|e| format!("Can't find this program: {e}"))?;
    set_exe(Some(&exe))
}

pub fn set_exe(exe: Option<&std::path::Path>) -> Result<(), String> {
    let path = path().ok_or("Can't find the config folder ($HOME is not set).")?;
    let Some(exe) = exe else {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("Can't remove {}: {e}", path.display()))
            }
            _ => Ok(()),
        };
    };
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
        let staged = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&staged, entry)?;
        std::fs::rename(staged, &path)
    };
    write().map_err(|e| format!("Can't write {}: {e}", path.display()))
}

/// Quotes a path for an Exec line. Desktop entry spec: double quotes with
/// " ` $ \ backslash-escaped, then every backslash doubled again by the
/// file's string escaping (so a literal \ becomes four).
pub fn exec_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '%' => out.push_str("%%"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn autostart_checks_target_arguments_and_disabled_entries() {
        let entry = "[Desktop Entry]\nExec=\"/tmp/bibi brightness\" --tray\n";
        assert!(enabled_for(entry, "/tmp/bibi brightness"));
        assert!(!enabled_for(entry, "/tmp/another"));
        assert!(!enabled_for(
            &entry.replace(" --tray", " --window"),
            "/tmp/bibi brightness"
        ));
        for disabled in ["Hidden=true", "X-GNOME-Autostart-enabled=false"] {
            assert!(!enabled_for(
                &format!("{entry}{disabled}\n"),
                "/tmp/bibi brightness"
            ));
        }
        assert!(!enabled_for("[Other]\nExec=/tmp/bibi --tray", "/tmp/bibi"));
        assert!(enabled_for(
            "[Desktop Entry]\nExec=/tmp/bibi --tray",
            "/tmp/bibi"
        ));
    }
    #[test]
    fn paths_are_quoted_for_desktop_exec() {
        assert_eq!(exec_quote("/tmp/a b%f"), "\"/tmp/a b%%f\"");
        assert_eq!(exec_quote("/tmp/$x"), "\"/tmp/\\\\$x\"");
    }
}
