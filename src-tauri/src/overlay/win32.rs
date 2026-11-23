//! Thin wrappers over the Win32 calls the overlay needs.

use windows::Win32::Foundation::{CloseHandle, HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MonitorFromWindow,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, SetForegroundWindow, SetWindowDisplayAffinity,
    WDA_EXCLUDEFROMCAPTURE,
};
use windows::core::PWSTR;

pub fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut _)
}

pub fn raw(h: HWND) -> isize {
    h.0 as isize
}

pub fn foreground() -> HWND {
    unsafe { GetForegroundWindow() }
}

pub fn is_alive(h: HWND) -> bool {
    !h.is_invalid() && unsafe { IsWindow(Some(h)).as_bool() }
}

pub fn is_minimized(h: HWND) -> bool {
    unsafe { IsIconic(h).as_bool() }
}

pub fn window_title(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Taskbar / desktop: never treat them as "the game".
pub fn is_shell_window(h: HWND) -> bool {
    matches!(
        class_name(h).as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Progman" | "WorkerW" | "NotifyIconOverflowWindow"
    )
}

/// "cs2.exe" for the process that owns the window.
pub fn process_exe(h: HWND) -> Option<String> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        ok.ok()?;
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Some(full.rsplit('\\').next().unwrap_or(&full).to_string())
    }
}

/// SetForegroundWindow; Windows often refuses it for a window shown programmatically, so fall
/// back to attaching to the foreground thread's input queue (verified in the phase 0 spike).
pub fn force_foreground(h: HWND) -> &'static str {
    unsafe {
        if SetForegroundWindow(h).as_bool() && GetForegroundWindow() == h {
            return "direct";
        }
        let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let me = GetCurrentThreadId();
        let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
        if GetForegroundWindow() == h { "attach-thread-input" } else { "failed" }
    }
}

/// A hidden top-level window that owns the panel. Owned windows are left alone by window
/// managers (e.g. Windhawk "center new windows", which re-centred the panel on every show),
/// stay out of Alt+Tab and the taskbar, and are still focusable.
pub fn create_owner_window() -> windows::core::Result<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, WS_EX_TOOLWINDOW, WS_POPUP};
    use windows::core::w;
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("STATIC"),
            w!("game-overlai owner"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        )
    }
}

pub fn window_rect(h: HWND) -> RECT {
    let mut r = RECT::default();
    let _ = unsafe { GetWindowRect(h, &mut r) };
    r
}

pub fn set_foreground(h: HWND) -> bool {
    unsafe { SetForegroundWindow(h).as_bool() }
}

/// Keeps the panel out of screenshots/streams (and out of our own game captures).
pub fn exclude_from_capture(h: HWND) -> bool {
    unsafe { SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE).is_ok() }
}

/// Screen area to place the panel in, and its DPI scale.
/// Fullscreen game -> the whole monitor; windowed game or no game -> the work area
/// (so the panel doesn't sit on the taskbar).
pub fn placement_area(game: Option<HWND>) -> (RECT, f64) {
    unsafe {
        let monitor: HMONITOR = match game {
            Some(g) => MonitorFromWindow(g, MONITOR_DEFAULTTONEAREST),
            None => {
                let mut p = POINT::default();
                let _ = GetCursorPos(&mut p);
                MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST)
            }
        };
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut mi);
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x.max(96) as f64 / 96.0;

        let covers_monitor = game.is_some_and(|g| {
            let mut r = RECT::default();
            let _ = GetWindowRect(g, &mut r);
            let m = mi.rcMonitor;
            r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
        });
        (if covers_monitor { mi.rcMonitor } else { mi.rcWork }, scale)
    }
}
