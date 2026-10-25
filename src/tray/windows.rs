// Windows tray icon in the notification area. A second start finds the
// running tray through a named event and sets it to open the window. A
// second event makes it quit, for the Install button.

use std::ptr;
use std::time::{Duration, Instant};
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

/// Set by the Install button to end the tray running from the download
const QUIT_EVENT: &str = "Local\\bibi-brightness-quit";

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

/// Ends a running tray (an open window stays open) and waits until it is
/// gone, so a new start doesn't find it. Ok also if none was running.
pub fn quit_running() -> Result<(), String> {
    let quit = wide(QUIT_EVENT);
    let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, quit.as_ptr()) };
    if event.is_null() {
        return Ok(());
    }
    unsafe {
        SetEvent(event);
        CloseHandle(event);
    }
    // The open event exists as long as the tray runs
    let open = wide(OPEN_EVENT);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, open.as_ptr()) };
        if event.is_null() {
            return Ok(());
        }
        unsafe { CloseHandle(event) };
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("The running tray icon doesn't quit. Quit it from its menu and try again.".into())
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
    let quit_name = wide(QUIT_EVENT);
    let quit_event = unsafe { CreateEventW(ptr::null(), 0, 0, quit_name.as_ptr()) };
    if quit_event.is_null() {
        let e = std::io::Error::last_os_error().to_string();
        unsafe { CloseHandle(event) };
        return Err(e);
    }
    let events = [event, quit_event];

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
        // Sleep until a window message (tray clicks, menu), a second start
        // or the Install button
        let woke =
            unsafe { MsgWaitForMultipleObjects(2, events.as_ptr(), 0, INFINITE, QS_ALLINPUT) };
        if woke == WAIT_OBJECT_0 {
            window.open();
        } else if woke == WAIT_OBJECT_0 + 1 {
            break 'run;
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
    unsafe {
        CloseHandle(quit_event);
        CloseHandle(event);
    }
    Ok(())
}
