// Windows: a value under HKCU\...\CurrentVersion\Run, the same place the
// "Startup apps" settings page lists.

use std::path::Path;
use std::ptr;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "BIBI Brightness";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn enabled() -> bool {
    let (key, name) = (wide(RUN_KEY), wide(VALUE_NAME));
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    err == ERROR_SUCCESS
}

pub fn set(enable: bool) -> Result<(), String> {
    if enable {
        let exe = std::env::current_exe().map_err(|e| format!("Can't find this program: {e}"))?;
        return set_exe(Some(&exe));
    }
    set_exe(None)
}

/// Points the entry at `exe`, or removes it for None. The Install button
/// uses this to move autostart to the installed copy.
pub fn set_exe(exe: Option<&Path>) -> Result<(), String> {
    let (key, name) = (wide(RUN_KEY), wide(VALUE_NAME));
    let err = if let Some(exe) = exe {
        let command = wide(&format!("\"{}\" --tray", exe.display()));
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        }
    } else {
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            err => err,
        }
    };
    if err != ERROR_SUCCESS {
        let e = std::io::Error::from_raw_os_error(err as i32);
        return Err(format!(
            "Can't change the autostart entry in the registry: {e}"
        ));
    }
    Ok(())
}
