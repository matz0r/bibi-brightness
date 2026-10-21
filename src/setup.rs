// ---------------------------------------------------------------------------
// One-time Linux setup so the logged-in user can talk to the monitors,
// started from the "Set up access" button. Runs as root through pkexec (the
// desktop's normal password dialog):
//
// - loads i2c-dev now and on every boot, so /dev/i2c-* exists
// - adds a udev rule that gives the user at the seat access to the GPU's
//   i2c buses (uaccess, like ddcutil's rule). No i2c group and no new login
//   needed. Other i2c buses (SMBus, sensors, RAM) stay root only.
// ---------------------------------------------------------------------------

use std::path::PathBuf;
use std::process::Command;

const RULE_FILE: &str = "/etc/udev/rules.d/60-bibi-brightness-i2c.rules";

// The rule must sort before 71-seat.rules / 73-seat-late.rules, which apply
// the uaccess tag. 0x03* = display controllers (VGA, 3D, …).
const SCRIPT: &str = r#"set -e
echo i2c-dev > /etc/modules-load.d/i2c-dev.conf
echo 'SUBSYSTEM=="i2c-dev", KERNEL=="i2c-[0-9]*", ATTRS{class}=="0x03*", TAG+="uaccess"' > "$1"
udevadm control --reload
modprobe i2c-dev
udevadm trigger --subsystem-match=i2c-dev
udevadm settle
"#;

pub enum Access {
    NoDevices,
    NoPermission,
    Ok,
}

pub fn check() -> Access {
    let devs: Vec<PathBuf> = std::fs::read_dir("/dev")
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("i2c-"))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    if devs.is_empty() {
        return Access::NoDevices;
    }
    let accessible = devs.iter().any(|p| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(p)
            .is_ok()
    });
    if accessible {
        Access::Ok
    } else {
        Access::NoPermission
    }
}

pub fn run() -> Result<(), String> {
    let status = Command::new("pkexec")
        .args(["/bin/sh", "-c", SCRIPT, "sh", RULE_FILE])
        .status()
        .map_err(|e| format!("Can't start pkexec ({e}). See the README for manual setup."))?;
    match status.code() {
        Some(0) => Ok(()),
        // 126: password dialog dismissed, 127: not authorized
        Some(126) | Some(127) => Err("Setup cancelled.".into()),
        _ => Err(format!("Setup failed ({status}).")),
    }
}
