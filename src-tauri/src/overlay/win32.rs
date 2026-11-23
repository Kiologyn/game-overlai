//! Thin wrappers over the Win32 calls the overlay needs.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::Graphics::Gdi::{
    ScreenToClient,
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
    EnumChildWindows, PostMessageW, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_EXITSIZEMOVE, WM_LBUTTONUP, WM_MOVING, WM_SIZING,
};
use windows::core::{BOOL, PWSTR};

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

/// The panel window, and whether the user lets it into screen captures (settings).
static PANEL: AtomicIsize = AtomicIsize::new(0);
static CAPTURABLE: AtomicBool = AtomicBool::new(false);

/// Shows or hides the panel in screenshots, recordings and streams (OBS, Discord, Win+Shift+S...).
pub fn set_capturable(h: HWND, capturable: bool) -> bool {
    PANEL.store(raw(h), Ordering::SeqCst);
    CAPTURABLE.store(capturable, Ordering::SeqCst);
    let affinity = if capturable { WDA_NONE } else { WDA_EXCLUDEFROMCAPTURE };
    unsafe { SetWindowDisplayAffinity(h, affinity).is_ok() }
}

/// Keeps the panel out of capture while it lives, even if the user lets it in: our own
/// captures of the whole monitor (frames for the agent) must show the game only.
pub struct OutOfCapture(Option<HWND>);

pub fn out_of_capture() -> OutOfCapture {
    let h = hwnd(PANEL.load(Ordering::SeqCst));
    if !CAPTURABLE.load(Ordering::SeqCst) || h.is_invalid() {
        return OutOfCapture(None); // already out
    }
    match unsafe { SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE) } {
        Ok(()) => OutOfCapture(Some(h)),
        Err(e) => {
            tracing::warn!("could not hide the panel from a monitor capture: {e}");
            OutOfCapture(None)
        }
    }
}

impl Drop for OutOfCapture {
    fn drop(&mut self) {
        if let Some(h) = self.0
            && CAPTURABLE.load(Ordering::SeqCst)
        {
            let _ = unsafe { SetWindowDisplayAffinity(h, WDA_NONE) };
        }
    }
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

// ------------------------------------------------------------------ moving and resizing the panel

/// Where the panel may be while the user drags or resizes it.
#[derive(Debug, Clone, Copy)]
pub struct MoveLimits {
    pub bounds: RECT,
    pub min_w: i32,
    pub min_h: i32,
    /// Compact panel: only the width changes.
    pub fixed_h: Option<i32>,
}

static LIMITS: std::sync::Mutex<Option<MoveLimits>> = std::sync::Mutex::new(None);
static ON_MOVED: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

pub fn set_move_limits(limits: MoveLimits) {
    *LIMITS.lock().unwrap() = Some(limits);
}

/// Keeps the user's moves and resizes of `h` inside the current `MoveLimits` (live, while
/// dragging) and calls `on_moved` when a move or resize ends. Subclasses our own window only.
pub fn limit_moves(h: HWND, on_moved: impl Fn() + Send + Sync + 'static) -> bool {
    let _ = ON_MOVED.set(Box::new(on_moved));
    unsafe { SetWindowSubclass(h, Some(move_subclass), 1, 0).as_bool() }
}

unsafe extern "system" fn move_subclass(
    h: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_MOVING | WM_SIZING => {
            let limits = *LIMITS.lock().unwrap();
            if let Some(limits) = limits {
                let r = unsafe { &mut *(lparam.0 as *mut RECT) };
                if msg == WM_MOVING {
                    clamp_move(r, &limits.bounds);
                } else {
                    clamp_size(r, wparam.0 as u32, &limits);
                }
                return LRESULT(1);
            }
        }
        WM_EXITSIZEMOVE => {
            release_mouse_in_children(h);
            if let Some(f) = ON_MOVED.get() {
                f();
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(h, msg, wparam, lparam) }
}

/// The native move/resize loop swallows the button release, so the WebView (a child window)
/// would think the mouse is still pressed and ignore the next click. Tell it the button is up.
fn release_mouse_in_children(h: HWND) {
    unsafe extern "system" fn post_up(child: HWND, _: LPARAM) -> BOOL {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
            let _ = ScreenToClient(child, &mut p);
            let pos = ((p.y as u16 as u32) << 16) | p.x as u16 as u32;
            let _ = PostMessageW(Some(child), WM_LBUTTONUP, WPARAM(0), LPARAM(pos as isize));
        }
        BOOL(1)
    }
    unsafe {
        let _ = EnumChildWindows(Some(h), Some(post_up), LPARAM(0));
    }
}

/// Shifts `r` (size kept) to lie inside `b` as far as it fits.
pub fn clamp_move(r: &mut RECT, b: &RECT) {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let left = r.left.min(b.right - w).max(b.left);
    let top = r.top.min(b.bottom - h).max(b.top);
    *r = RECT { left, top, right: left + w, bottom: top + h };
}

/// Limits the edges being dragged (`WMSZ_*`) to the minimum size and the bounds (the bounds win).
pub fn clamp_size(r: &mut RECT, edge: u32, l: &MoveLimits) {
    let b = &l.bounds;
    // WMSZ_LEFT 1, RIGHT 2, TOP 3, TOPLEFT 4, TOPRIGHT 5, BOTTOM 6, BOTTOMLEFT 7, BOTTOMRIGHT 8
    let left = matches!(edge, 1 | 4 | 7);
    let right = matches!(edge, 2 | 5 | 8);
    let top = matches!(edge, 3..=5);
    let bottom = matches!(edge, 6..=8);
    if left {
        r.left = r.left.min(r.right - l.min_w).max(b.left);
    }
    if right {
        r.right = r.right.max(r.left + l.min_w).min(b.right);
    }
    if top {
        r.top = r.top.min(r.bottom - l.min_h).max(b.top);
    }
    if bottom {
        r.bottom = r.bottom.max(r.top + l.min_h).min(b.bottom);
    }
    if let Some(h) = l.fixed_h {
        if top {
            r.top = r.bottom - h;
        } else {
            r.bottom = r.top + h;
        }
    }
}

/// The area the panel must stay in: the game window (its visible frame) on its monitor's
/// work area, or the whole monitor for a fullscreen game; without a game, the work area under
/// the cursor. Also the monitor's scale.
pub fn panel_bounds(game: Option<HWND>) -> (RECT, f64) {
    let (area, scale) = placement_area(game);
    let Some(g) = game else { return (area, scale) };
    let mut frame = RECT::default();
    let got = unsafe {
        DwmGetWindowAttribute(
            g,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut frame as *mut RECT as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if got.is_err() {
        frame = window_rect(g);
    }
    let r = RECT {
        left: frame.left.max(area.left),
        top: frame.top.max(area.top),
        right: frame.right.min(area.right),
        bottom: frame.bottom.min(area.bottom),
    };
    // A tiny or off-screen game window: use its monitor instead.
    let min = (200.0 * scale) as i32;
    if r.right - r.left < min || r.bottom - r.top < min { (area, scale) } else { (r, scale) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    #[test]
    fn moves_stay_inside() {
        let b = rect(100, 100, 1100, 700);
        let mut r = rect(900, 600, 1300, 800); // past the bottom-right corner
        clamp_move(&mut r, &b);
        assert_eq!(r, rect(700, 500, 1100, 700));
        let mut r = rect(0, 0, 400, 200);
        clamp_move(&mut r, &b);
        assert_eq!(r, rect(100, 100, 500, 300));
    }

    #[test]
    fn resizes_stay_inside_and_above_the_minimum() {
        let l = MoveLimits { bounds: rect(0, 0, 1000, 800), min_w: 300, min_h: 200, fixed_h: None };
        let mut r = rect(-50, 100, 1200, 500); // left and right dragged out
        clamp_size(&mut r, 1, &l);
        clamp_size(&mut r, 2, &l);
        assert_eq!((r.left, r.right), (0, 1000));
        let mut r = rect(700, 100, 750, 500); // left edge dragged past the minimum width
        clamp_size(&mut r, 1, &l);
        assert_eq!((r.left, r.right), (450, 750));
        let mut r = rect(100, 100, 150, 500); // the minimum doesn't push it out of the game
        clamp_size(&mut r, 1, &l);
        assert_eq!(r.left, 0);
        let compact = MoveLimits { fixed_h: Some(56), ..l };
        let mut r = rect(100, 300, 700, 380); // corner drag on the compact panel
        clamp_size(&mut r, 8, &compact);
        assert_eq!(r.bottom - r.top, 56);
    }
}
