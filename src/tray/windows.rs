// Windows tray icon in the notification area. A second start finds the
// running tray through a named event and sets it to open the window.

use std::ptr;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetSystemMetrics, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW,
    QS_ALLINPUT, SM_CXSMICON, TranslateMessage, WM_QUIT,
};

use super::Window;

/// Per user session; set by a second start to open the window
const OPEN_EVENT: &str = "Local\\bibi-brightness-open";

/// Resource id of assets/icon.ico in brightness.exe (see build.rs)
const ICON_ID: u16 = 1;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Tells a tray that is already running to open the window (or nothing, for
/// --tray). Returns false if there is none.
fn notify_running(show_window: bool) -> bool {
    let name = wide(OPEN_EVENT);
    let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if event.is_null() {
        return false;
    }
    unsafe {
        if show_window {
            SetEvent(event);
        }
        CloseHandle(event);
    }
    true
}

/// Runs until "Quit" is chosen.
pub fn run(show_window: bool) -> Result<(), String> {
    if notify_running(show_window) {
        return Ok(());
    }
    let name = wide(OPEN_EVENT);
    // Auto-reset: one wake per SetEvent
    let event = unsafe { CreateEventW(ptr::null(), 0, 0, name.as_ptr()) };
    if event.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // Another tray started at the same moment
        unsafe {
            if show_window {
                SetEvent(event);
            }
            CloseHandle(event);
        }
        return Ok(());
    }

    let open = MenuItem::new("Open", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::with_items(&[&open, &PredefinedMenuItem::separator(), &quit])
        .map_err(|e| e.to_string())?;
    // The small icon size for the current DPI, picked from the .ico
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    let icon = Icon::from_resource(ICON_ID, Some((size, size))).map_err(|e| e.to_string())?;
    let _tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("BIBI Brightness\nClick to show or hide the sliders")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| e.to_string())?;

    let mut window = Window::default();
    if show_window {
        window.open();
    }

    'run: loop {
        // Sleep until a window message (tray clicks, menu) or a second start
        let woke = unsafe { MsgWaitForMultipleObjects(1, &event, 0, INFINITE, QS_ALLINPUT) };
        if woke == WAIT_OBJECT_0 {
            window.open();
        }

        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_QUIT {
                break 'run;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // tray-icon queues these while the messages above are dispatched
        for ev in TrayIconEvent::receiver().try_iter() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                window.toggle();
            }
        }
        for ev in MenuEvent::receiver().try_iter() {
            if ev.id == *open.id() {
                window.open();
            } else if ev.id == *quit.id() {
                break 'run;
            }
        }
    }

    // An open window stays open
    unsafe { CloseHandle(event) };
    Ok(())
}
