//! Phase 0 / spike 2: transparent always-on-top Tauri panel over a game.
//!
//!   Alt+Q  -> quick field (new chat), Alt+W -> chat panel; pressing again while typing hides
//!   Esc or the ✕ button -> hide
//!
//! Two input modes:
//!   * focus mode    — the panel takes keyboard focus (windowed / borderless games)
//!   * capture mode  — the panel is shown WITHOUT activation, the game keeps focus and
//!                     keystrokes are taken by a low-level keyboard hook (exclusive fullscreen
//!                     games minimize when they lose focus). Auto-detected; `--nofocus` forces it.
//!
//! The page is served by a local axum server and loaded as an external URL:
//! no Tauri IPC (core -> page via eval, page -> core via HTTP). The window is excluded
//! from screen capture (WDA_EXCLUDEFROMCAPTURE) and sized exactly to the panel.
//!
//!   cargo run                   normal
//!   cargo run -- --nofocus      always use capture mode
//!   cargo run -- --show         show the panel right away (for tests)

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::routing::{get, post};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, ShortcutState};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, ENUM_CURRENT_SETTINGS, ENUM_REGISTRY_SETTINGS, EnumDisplaySettingsW, GetMonitorInfoW,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromWindow,
};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardLayout, ToUnicodeEx, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_CONTROL,
    VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RETURN, VK_RMENU,
    VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CallNextHookEx, GWL_EXSTYLE, GetForegroundWindow, GetMessageW, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, HWND_TOPMOST, IsIconic, IsWindowVisible,
    KBDLLHOOKSTRUCT, MB_ICONASTERISK, MB_ICONHAND, MB_OK, MESSAGEBOX_STYLE, MSG,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetForegroundWindow,
    SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW, ShowWindow,
    WDA_EXCLUDEFROMCAPTURE, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN, WS_EX_NOACTIVATE,
};
use windows::core::PCWSTR;

// logical px
const PANEL_W: f64 = 640.0;
const COMPACT_H: f64 = 56.0;
const EXPANDED_H: f64 = 540.0;
const BOTTOM_MARGIN: f64 = 60.0;

static T0: OnceLock<Instant> = OnceLock::new();
static APP: OnceLock<AppHandle> = OnceLock::new();
/// The window that was in the foreground when the hotkey was pressed (the game).
static GAME_HWND: AtomicIsize = AtomicIsize::new(0);
static NOFOCUS_FORCED: AtomicBool = AtomicBool::new(false);
/// Current panel session uses capture mode (game keeps focus).
static CAPTURE_SESSION: AtomicBool = AtomicBool::new(false);
/// Keyboard hook is swallowing keystrokes and feeding them to the panel.
static INPUT_MODE: AtomicBool = AtomicBool::new(false);
static EXPANDED: AtomicBool = AtomicBool::new(false);
/// True exclusive fullscreen: no window at all (any window on top kicks the game out of
/// fullscreen and Source 2 minimizes). Typing is blind, feedback by sound.
static BLIND: AtomicBool = AtomicBool::new(false);
/// --window-in-exclusive: old behaviour (show the panel anyway), for comparison.
static WINDOW_IN_EXCLUSIVE: AtomicBool = AtomicBool::new(false);
/// What was typed in the current capture session (mirrors the page input).
static TYPED: Mutex<String> = Mutex::new(String::new());

fn beep(kind: MESSAGEBOX_STYLE) {
    let _ = unsafe { MessageBeep(kind) };
}

/// Logs 1 s later whether the game still has focus / got minimized.
fn watchdog(label: &'static str) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1000));
        let game = game_hwnd();
        let (fg, iconic) = (foreground() == game, unsafe { IsIconic(game).as_bool() });
        log(format!("  watchdog after {label}: game-is-fg={fg} game-minimized={iconic}"));
    });
}
/// Monitor rect (physical px) + scale factor the panel is placed on.
static PLACE: Mutex<Option<(RECT, f64)>> = Mutex::new(None);

fn log(msg: impl AsRef<str>) {
    println!("[{:>7} ms] {}", T0.get().unwrap().elapsed().as_millis(), msg.as_ref());
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Compact,
    Expanded,
}

fn panel(app: &AppHandle) -> WebviewWindow {
    app.get_webview_window("panel").expect("panel window")
}

fn panel_hwnd(w: &WebviewWindow) -> HWND {
    HWND(w.hwnd().expect("hwnd").0)
}

fn game_hwnd() -> HWND {
    HWND(GAME_HWND.load(Ordering::SeqCst) as *mut _)
}

fn window_title(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn foreground() -> HWND {
    unsafe { GetForegroundWindow() }
}

fn js(v: impl serde::Serialize) -> String {
    serde_json::to_string(&v).unwrap()
}

/// SetForegroundWindow, falling back to the AttachThreadInput trick.
fn force_foreground(h: HWND) -> &'static str {
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
        if GetForegroundWindow() == h { "attach-thread-input" } else { "FAILED" }
    }
}

// ---------------------------------------------------------------- fullscreen detection

struct FullscreenInfo {
    exclusive: bool,
    details: String,
}

/// Heuristics for "this game will minimize if it loses focus":
/// the game changed the display mode, or the shell reports a D3D exclusive fullscreen app.
fn detect_fullscreen(game: HWND) -> FullscreenInfo {
    unsafe {
        let quns = SHQueryUserNotificationState().ok();
        let d3d_fs = quns == Some(QUNS_RUNNING_D3D_FULL_SCREEN);

        let hmon = MonitorFromWindow(game, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        let _ = GetMonitorInfoW(hmon, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO);
        let dev = PCWSTR(mi.szDevice.as_ptr());
        let mut cur = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        let mut reg = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        let _ = EnumDisplaySettingsW(dev, ENUM_CURRENT_SETTINGS, &mut cur);
        let _ = EnumDisplaySettingsW(dev, ENUM_REGISTRY_SETTINGS, &mut reg);
        let mode_changed = (cur.dmPelsWidth, cur.dmPelsHeight) != (reg.dmPelsWidth, reg.dmPelsHeight);

        let mut r = RECT::default();
        let _ = GetWindowRect(game, &mut r);
        let m = mi.monitorInfo.rcMonitor;
        let covers = r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;

        FullscreenInfo {
            exclusive: d3d_fs || mode_changed,
            details: format!(
                "quns={:?} display={}x{} desktop={}x{} mode_changed={mode_changed} covers_monitor={covers}",
                quns.map(|q| q.0),
                cur.dmPelsWidth,
                cur.dmPelsHeight,
                reg.dmPelsWidth,
                reg.dmPelsHeight
            ),
        }
    }
}

// ---------------------------------------------------------------- placement

fn update_place(app: &AppHandle, game: HWND) {
    let mut r = RECT::default();
    let _ = unsafe { GetWindowRect(game, &mut r) };
    let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
    let monitors = app.available_monitors().unwrap_or_default();
    let mon = monitors
        .iter()
        .find(|m| {
            let (p, s) = (m.position(), m.size());
            cx >= p.x && cx < p.x + s.width as i32 && cy >= p.y && cy < p.y + s.height as i32
        })
        .cloned()
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(m) = mon {
        let (p, s) = (m.position(), m.size());
        let rect = RECT { left: p.x, top: p.y, right: p.x + s.width as i32, bottom: p.y + s.height as i32 };
        *PLACE.lock().unwrap() = Some((rect, m.scale_factor()));
    }
}

/// Size the window exactly to the panel, anchored bottom-centre, so no invisible
/// area of our window eats clicks meant for the game.
fn apply_rect(hwnd: HWND, mode: Mode) {
    let Some((m, sf)) = *PLACE.lock().unwrap() else { return };
    let w = (PANEL_W * sf) as i32;
    let h = ((if mode == Mode::Expanded { EXPANDED_H } else { COMPACT_H }) * sf) as i32;
    let x = m.left + (m.right - m.left - w) / 2;
    let y = m.bottom - h - (BOTTOM_MARGIN * sf) as i32;
    unsafe {
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
    }
    EXPANDED.store(mode == Mode::Expanded, Ordering::SeqCst);
}

fn set_noactivate(hwnd: HWND, on: bool) {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let new = if on { ex | WS_EX_NOACTIVATE.0 as isize } else { ex & !(WS_EX_NOACTIVATE.0 as isize) };
        if new != ex {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new);
        }
    }
}

// ---------------------------------------------------------------- show / hide

fn on_hotkey(app: &AppHandle, mode: Mode) {
    let me = panel_hwnd(&panel(app));
    let visible = unsafe { IsWindowVisible(me).as_bool() } || BLIND.load(Ordering::SeqCst);
    let typing = if CAPTURE_SESSION.load(Ordering::SeqCst) {
        INPUT_MODE.load(Ordering::SeqCst)
    } else {
        foreground() == me
    };
    if visible && typing { hide(app) } else { show(app, mode, true) }
}

/// `from_hotkey`: the foreground window is the game. When opened from the tray the
/// foreground window is the taskbar, so keep the previously remembered game window.
fn show(app: &AppHandle, mode: Mode, from_hotkey: bool) {
    let t = Instant::now();
    let w = panel(app);
    let me = panel_hwnd(&w);
    let fg = foreground();
    if from_hotkey && fg != me && !fg.is_invalid() {
        GAME_HWND.store(fg.0 as isize, Ordering::SeqCst);
    }
    let game = game_hwnd();
    let title = window_title(game);
    let fs = detect_fullscreen(game);
    let capture = NOFOCUS_FORCED.load(Ordering::SeqCst) || fs.exclusive;
    CAPTURE_SESSION.store(capture, Ordering::SeqCst);
    TYPED.lock().unwrap().clear();

    let blind = fs.exclusive && !WINDOW_IN_EXCLUSIVE.load(Ordering::SeqCst);
    BLIND.store(blind, Ordering::SeqCst);
    if blind {
        CAPS.store(unsafe { GetKeyState(VK_CAPITAL.0 as i32) } & 1 != 0, Ordering::SeqCst);
        INPUT_MODE.store(true, Ordering::SeqCst);
        beep(MB_ICONASTERISK);
        let m = if mode == Mode::Expanded { "expanded" } else { "compact" };
        let _ = w.eval(format!("__overlay.show({}, {}, true)", js(m), js(&title))); // page state only
        log(format!(
            "SHOW BLIND over {title:?} (exclusive fullscreen: no window, typing by hook) | game-is-fg={}\n            fullscreen: {}",
            foreground() == game,
            fs.details
        ));
        watchdog("blind show");
        return;
    }

    update_place(app, game);
    set_noactivate(me, capture);
    apply_rect(me, mode);

    let how = if capture {
        unsafe {
            let _ = ShowWindow(me, SW_SHOWNOACTIVATE);
        }
        CAPS.store(unsafe { GetKeyState(VK_CAPITAL.0 as i32) } & 1 != 0, Ordering::SeqCst);
        INPUT_MODE.store(true, Ordering::SeqCst);
        "capture mode: no activation, keyboard hook"
    } else {
        INPUT_MODE.store(false, Ordering::SeqCst);
        let _ = w.show();
        let how = force_foreground(me);
        let _ = w.set_focus();
        how
    };
    let m = if mode == Mode::Expanded { "expanded" } else { "compact" };
    let _ = w.eval(format!("__overlay.show({}, {}, {capture})", js(m), js(&title)));
    log(format!(
        "SHOW {mode:?} over {title:?} | {how} | game-is-fg={} panel-is-fg={} | {} us\n            fullscreen: {}",
        foreground() == game,
        foreground() == me,
        t.elapsed().as_micros(),
        fs.details
    ));
    if capture {
        watchdog("show with window");
    }
}

fn hide(app: &AppHandle) {
    INPUT_MODE.store(false, Ordering::SeqCst);
    let w = panel(app);
    let _ = w.eval("__overlay.hidden()");
    let _ = w.hide();
    let game = game_hwnd();
    if BLIND.swap(false, Ordering::SeqCst) {
        beep(MB_ICONHAND);
        log(format!("HIDE BLIND (cancelled) | game-is-fg={}", foreground() == game));
    } else if CAPTURE_SESSION.load(Ordering::SeqCst) {
        log(format!("HIDE (capture mode, game kept focus: {})", foreground() == game));
    } else if !game.is_invalid() {
        let _ = unsafe { SetForegroundWindow(game) };
        std::thread::sleep(Duration::from_millis(30));
        log(format!("HIDE -> focus back to {:?}: {}", window_title(game), foreground() == game));
    }
}

// ---------------------------------------------------------------- keyboard hook (capture mode)

enum KeyEv {
    Char(String),
    Back,
    Enter,
    Esc,
}

static KEY_TX: OnceLock<Mutex<Sender<KeyEv>>> = OnceLock::new();
static SHIFT: AtomicBool = AtomicBool::new(false);
static CTRL: AtomicBool = AtomicBool::new(false);
static ALT: AtomicBool = AtomicBool::new(false);
static CAPS: AtomicBool = AtomicBool::new(false);

fn translate(vk: VIRTUAL_KEY, scan: u32) -> Option<KeyEv> {
    match vk {
        VK_ESCAPE => return Some(KeyEv::Esc),
        VK_RETURN => return Some(KeyEv::Enter),
        VK_BACK => return Some(KeyEv::Back),
        _ => {}
    }
    let (shift, ctrl, alt) = (SHIFT.load(Ordering::SeqCst), CTRL.load(Ordering::SeqCst), ALT.load(Ordering::SeqCst));
    if ctrl && !alt {
        return None; // Ctrl shortcuts (paste etc.) — not in the spike
    }
    let mut ks = [0u8; 256];
    if shift {
        ks[VK_SHIFT.0 as usize] = 0x80;
    }
    if CAPS.load(Ordering::SeqCst) {
        ks[VK_CAPITAL.0 as usize] = 0x01;
    }
    if ctrl && alt {
        ks[VK_CONTROL.0 as usize] = 0x80; // AltGr
        ks[VK_MENU.0 as usize] = 0x80;
    }
    unsafe {
        // Use the keyboard layout of the game's thread (the user may switch RU/EN in game).
        let layout = GetKeyboardLayout(GetWindowThreadProcessId(GetForegroundWindow(), None));
        let mut buf = [0u16; 8];
        // flag 0x4: don't touch the kernel keyboard state (dead keys of the game stay intact)
        let n = ToUnicodeEx(vk.0 as u32, scan, &ks, &mut buf, 0x4, Some(layout));
        if n > 0 {
            let s = String::from_utf16_lossy(&buf[..n as usize]);
            if s.chars().all(|c| !c.is_control()) {
                return Some(KeyEv::Char(s));
            }
        }
    }
    None
}

unsafe extern "system" fn kb_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let msg = wparam.0 as u32;
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let vk = VIRTUAL_KEY(kb.vkCode as u16);
        let is_modifier = matches!(
            vk,
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CONTROL | VK_LCONTROL | VK_RCONTROL | VK_MENU | VK_LMENU | VK_RMENU
        );
        match vk {
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT => SHIFT.store(down, Ordering::SeqCst),
            VK_CONTROL | VK_LCONTROL | VK_RCONTROL => CTRL.store(down, Ordering::SeqCst),
            VK_MENU | VK_LMENU | VK_RMENU => ALT.store(down, Ordering::SeqCst),
            _ => {}
        }
        if INPUT_MODE.load(Ordering::SeqCst) && down {
            if vk == VK_CAPITAL {
                CAPS.fetch_xor(true, Ordering::SeqCst);
            }
            let alt_combo = ALT.load(Ordering::SeqCst) && !CTRL.load(Ordering::SeqCst); // our Alt+Q / Alt+W
            let pass = is_modifier || alt_combo || matches!(vk, VK_LWIN | VK_RWIN | VK_CAPITAL);
            if !pass {
                if let Some(ev) = translate(vk, kb.scanCode) {
                    if let Some(tx) = KEY_TX.get() {
                        let _ = tx.lock().unwrap().send(ev);
                    }
                }
                return LRESULT(1); // swallow: the game never sees this key-down
            }
        }
        // key-ups always pass, so keys held before typing get released in the game
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn start_keyboard_hook(app: AppHandle) {
    let (tx, rx) = channel::<KeyEv>();
    KEY_TX.set(Mutex::new(tx)).ok();
    // Consumer: never block inside the hook procedure (Windows drops slow LL hooks).
    std::thread::spawn(move || {
        for ev in rx {
            let w = panel(&app);
            match ev {
                KeyEv::Esc => hide(&app),
                KeyEv::Enter => {
                    INPUT_MODE.store(false, Ordering::SeqCst); // back to the game, answer streams on top
                    let _ = w.eval("__overlay.vkey({t:'enter'}); __overlay.inputMode(false)");
                    let text = std::mem::take(&mut *TYPED.lock().unwrap());
                    if BLIND.swap(false, Ordering::SeqCst) {
                        beep(MB_OK);
                        log(format!("ENTER (blind): sent {text:?}, input released to the game"));
                        watchdog("blind enter");
                    } else {
                        log(format!("ENTER: sent {text:?}, input released to the game, panel stays visible"));
                    }
                }
                KeyEv::Back => {
                    TYPED.lock().unwrap().pop();
                    let _ = w.eval("__overlay.vkey({t:'back'})");
                }
                KeyEv::Char(s) => {
                    TYPED.lock().unwrap().push_str(&s);
                    let _ = w.eval(format!("__overlay.vkey({{t:'char',c:{}}})", js(s)));
                }
            }
        }
    });
    std::thread::spawn(|| unsafe {
        let hmod = GetModuleHandleW(None).ok().map(|h| HINSTANCE(h.0));
        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), hmod, 0) {
            Ok(_) => log("keyboard hook installed"),
            Err(e) => log(format!("keyboard hook FAILED: {e}")),
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    });
}

// ---------------------------------------------------------------- tray

const HOTKEYS: [&str; 2] = ["alt+q", "alt+w"];
const TOOLTIP: &str = "game-overlai — готов\nAlt+Q вопрос · Alt+W чат";

fn build_tray(app: &tauri::App, port: u16) -> tauri::Result<()> {
    let quick = MenuItem::with_id(app, "quick", "Быстрый вопрос\tAlt+Q", true, None::<&str>)?;
    let chat = MenuItem::with_id(app, "chat", "Открыть чат\tAlt+W", true, None::<&str>)?;
    let browser = MenuItem::with_id(app, "browser", "Открыть чат в браузере", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Пауза горячих клавиш", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&quick, &chat, &browser, &PredefinedMenuItem::separator(app)?, &pause, &PredefinedMenuItem::separator(app)?, &quit],
    )?;
    let pause_item = pause.clone();
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("icon").clone())
        .tooltip(TOOLTIP)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "quick" => show(app, Mode::Compact, false),
            "chat" => show(app, Mode::Expanded, false),
            "browser" => {
                let _ = std::process::Command::new("explorer.exe").arg(format!("http://127.0.0.1:{port}/panel")).spawn();
            }
            "pause" => {
                let paused = pause_item.is_checked().unwrap_or(false);
                let gs = app.global_shortcut();
                let res = if paused { gs.unregister_all() } else { gs.register_multiple(HOTKEYS) };
                if let Some(tray) = app.tray_by_id("main") {
                    let _ = tray.set_tooltip(Some(if paused { "game-overlai — горячие клавиши на паузе" } else { TOOLTIP }));
                }
                log(format!("hotkeys paused={paused} ({res:?})"));
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show(tray.app_handle(), Mode::Expanded, false);
            }
        })
        .build(app)?;
    log("tray icon ready (left click = chat, right click = menu)");
    Ok(())
}

// ---------------------------------------------------------------- local HTTP server

async fn serve(listener: std::net::TcpListener) {
    let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
    let router = axum::Router::new()
        .route("/panel", get(|| async { axum::response::Html(include_str!("../ui/panel.html")) }))
        .route(
            "/api/hide",
            post(|| async {
                tauri::async_runtime::spawn_blocking(|| hide(APP.get().unwrap()));
            }),
        )
        .route(
            "/api/mode",
            post(|body: String| async move {
                let app = APP.get().unwrap();
                let mode = if body == "expanded" { Mode::Expanded } else { Mode::Compact };
                apply_rect(panel_hwnd(&panel(app)), mode);
            }),
        )
        .route("/api/log", post(|body: String| async move { log(format!("page: {body}")) }))
        .route("/api/ready", post(|body: String| async move { log(format!("page ready, JS at {body} ms after navigation start")) }));
    axum::serve(listener, router).await.expect("server");
}

fn main() {
    T0.set(Instant::now()).unwrap();
    let show_on_start = std::env::args().any(|a| a == "--show");
    NOFOCUS_FORCED.store(std::env::args().any(|a| a == "--nofocus"), Ordering::SeqCst);
    WINDOW_IN_EXCLUSIVE.store(std::env::args().any(|a| a == "--window-in-exclusive"), Ordering::SeqCst);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    log(format!("UI server at http://127.0.0.1:{port}/panel (open it in a browser too)"));
    if NOFOCUS_FORCED.load(Ordering::SeqCst) {
        log("--nofocus: capture mode forced for all windows");
    }

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(HOTKEYS)
                .expect("shortcuts")
                .with_handler(|app, shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    if shortcut.matches(Modifiers::ALT, Code::KeyQ) {
                        on_hotkey(app, Mode::Compact);
                    } else if shortcut.matches(Modifiers::ALT, Code::KeyW) {
                        on_hotkey(app, Mode::Expanded);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            let handle = app.handle().clone();
            APP.set(handle.clone()).ok();
            tauri::async_runtime::spawn(serve(listener));

            let url = format!("http://127.0.0.1:{port}/panel").parse().unwrap();
            let t = Instant::now();
            let w = WebviewWindowBuilder::new(app, "panel", WebviewUrl::External(url))
                .title("game-overlai")
                .transparent(true)
                .decorations(false)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .visible(false)
                .focused(false)
                .inner_size(PANEL_W, COMPACT_H)
                .build()?;
            let hwnd = panel_hwnd(&w);
            // --capturable: debug only, lets screenshots see the panel
            let excluded = !std::env::args().any(|a| a == "--capturable")
                && unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }.is_ok();
            log(format!("panel window created in {} ms, excluded from capture: {excluded}", t.elapsed().as_millis()));
            log("hotkeys: Alt+Q = quick field, Alt+W = chat panel, Esc / ✕ = hide");
            build_tray(app, port)?;
            start_keyboard_hook(handle.clone());

            if show_on_start {
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(1500));
                    let h2 = h.clone();
                    let _ = h.run_on_main_thread(move || show(&h2, Mode::Compact, true));
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri app");
}
