//! The overlay panel: one transparent, always-on-top, frameless window that is sized exactly
//! to the panel (so no invisible part of it eats clicks meant for the game).
//!
//!   compact  — only the input field
//!   expanded — chat history above the field; the window grows upward
//!
//! The panel takes keyboard focus when shown and gives it back to the game when hidden.
//! It may stay open without focus while the user clicks into the game.
//! All functions here must run on the main thread (use `run_on_main`).

pub mod win32;

use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tracing::{debug, info, warn};

use crate::protocol::{GameInfo, PanelMode, ServerMsg};
use crate::state::{AppState, Prefetch, Target};

pub const LABEL: &str = "panel";
/// Height of the input row, logical px. Must match `--composer-h` in the UI.
pub const COMPACT_H: f64 = 56.0;
/// How long the hotkey waits for the game frame before showing the panel (~30 ms usually).
const PREFETCH_WAIT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowSource {
    /// Global hotkey: the foreground window is the game.
    Hotkey,
    /// Tray, second app instance, ...: keep the game remembered earlier.
    Other,
}

pub fn create(app: &AppHandle, state: &AppState) -> anyhow::Result<()> {
    let url = format!("{}/panel", state.ui_base).parse()?;
    let owner = win32::create_owner_window()?;
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .owner_raw(owner)
        .title("game-overlai")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .focused(false)
        .inner_size(state.config.panel.width, COMPACT_H)
        .build()?;

    let hwnd = panel_hwnd(&window);
    if std::env::var_os("OVERLAI_DEBUG_CAPTURABLE").is_some() {
        warn!("OVERLAI_DEBUG_CAPTURABLE: the panel is visible in screenshots");
    } else if !win32::exclude_from_capture(hwnd) {
        warn!("could not exclude the panel from screen capture");
    }

    // Alt+F4 on the panel hides it instead of destroying it.
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            hide(&handle);
        }
    });
    info!("panel window created ({})", state.ui_base);
    Ok(())
}

pub fn run_on_main(app: &AppHandle, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || f(&handle)) {
        warn!("run_on_main_thread failed: {e}");
    }
}

fn panel(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

fn panel_hwnd(w: &WebviewWindow) -> windows::Win32::Foundation::HWND {
    w.hwnd().unwrap_or_default()
}

fn state(app: &AppHandle) -> Arc<AppState> {
    app.state::<Arc<AppState>>().inner().clone()
}

/// Hotkey semantics:
///   hidden                          -> show in `mode`
///   visible, unfocused              -> focus it (in `mode`)
///   visible, focused, other mode    -> switch to `mode`
///   visible, focused, same mode     -> hide
pub fn on_hotkey(app: &AppHandle, mode: PanelMode) {
    let Some(w) = panel(app) else { return };
    let me = panel_hwnd(&w);
    let visible = w.is_visible().unwrap_or(false);
    let focused = win32::foreground() == me;
    let current = state(app).overlay.lock().unwrap().mode;
    if visible && focused && current == mode {
        hide(app);
    } else {
        show(app, mode, ShowSource::Hotkey);
    }
}

pub fn show(app: &AppHandle, mode: PanelMode, source: ShowSource) {
    let Some(w) = panel(app) else { return };
    let st = state(app);
    let me = panel_hwnd(&w);
    let fg = win32::foreground();

    if source == ShowSource::Hotkey && fg != me && win32::is_alive(fg) && !win32::is_shell_window(fg) {
        let game = GameInfo { title: win32::window_title(fg), exe: win32::process_exe(fg).unwrap_or_default() };
        // Grab the frame BEFORE the panel takes focus: it is what the user is asking about, and
        // some games pause, hide tooltips or (exclusive fullscreen) minimise once unfocused.
        // Kept in memory only; used if the message goes out with 📷 on.
        let started = Instant::now();
        let prefetch = match st.capture.grab_blocking(win32::raw(fg), PREFETCH_WAIT) {
            Ok(frame) => {
                debug!("prefetched {frame:?} in {} ms", started.elapsed().as_millis());
                Some(Prefetch { frame: Arc::new(frame), used: false })
            }
            Err(e) => {
                debug!("no prefetched frame: {e}");
                None
            }
        };
        let mut ov = st.overlay.lock().unwrap();
        ov.game_hwnd = win32::raw(fg);
        ov.game = Some(game);
        ov.prefetch = prefetch;
    }
    let (game_hwnd, game) = {
        let mut ov = st.overlay.lock().unwrap();
        if !win32::is_alive(win32::hwnd(ov.game_hwnd)) {
            ov.game_hwnd = 0;
            ov.game = None;
            ov.prefetch = None;
        }
        ov.mode = mode;
        ov.visible = true;
        (ov.game_hwnd, ov.game.clone())
    };

    // Alt+W over a game shows that game's latest chat (chats are bound to games).
    if source == ShowSource::Hotkey && mode == PanelMode::Expanded
        && let Some(g) = &game
    {
        crate::chat::follow_game(&st, g);
    }
    place(&st, &w, mode, game_hwnd);
    let _ = w.set_always_on_top(true); // re-assert: some games make themselves topmost
    let _ = w.show();
    let how = win32::force_foreground(me);
    let _ = w.set_focus();
    let r = win32::window_rect(me);
    st.hub.send(Target::Overlay, ServerMsg::OverlayShow { mode, game: game.clone() });
    debug!(
        "panel shown: {mode:?} at ({},{}) over {:?} ({:?}), focus: {how}",
        r.left,
        r.top,
        game.as_ref().map(|g| g.title.as_str()).unwrap_or("-"),
        source
    );
}

pub fn hide(app: &AppHandle) {
    let Some(w) = panel(app) else { return };
    let st = state(app);
    let game_hwnd = {
        let mut ov = st.overlay.lock().unwrap();
        ov.visible = false;
        ov.prefetch = None; // the next question is about a later moment
        ov.game_hwnd
    };
    let _ = w.hide();
    st.hub.send(Target::Overlay, ServerMsg::OverlayHidden);

    let game = win32::hwnd(game_hwnd);
    if win32::is_alive(game) && !win32::is_minimized(game) {
        // Usually Windows activates the game by itself once we hide; this makes sure.
        let _ = win32::set_foreground(game);
    }
    debug!("panel hidden");
}

/// The UI switched compact <-> expanded (e.g. first message sent from the quick field).
pub fn set_mode(app: &AppHandle, mode: PanelMode) {
    let Some(w) = panel(app) else { return };
    let st = state(app);
    let game_hwnd = {
        let mut ov = st.overlay.lock().unwrap();
        if ov.mode == mode {
            return;
        }
        ov.mode = mode;
        ov.game_hwnd
    };
    place(&st, &w, mode, game_hwnd);
}

/// Bottom-centre of the game's monitor; the bottom edge stays put when the panel expands.
/// Goes through Tauri so tao's view of the window stays in sync.
fn place(st: &AppState, window: &WebviewWindow, mode: PanelMode, game_hwnd: isize) {
    let game = win32::hwnd(game_hwnd);
    let (area, scale) = win32::placement_area(win32::is_alive(game).then_some(game));
    let cfg = &st.config.panel;
    let margin = (cfg.bottom_margin * scale).round() as i32;
    let max_h = (area.bottom - area.top - 2 * margin).max(1);
    let w = ((cfg.width * scale).round() as i32).min(area.right - area.left);
    let h = match mode {
        PanelMode::Compact => (COMPACT_H * scale).round() as i32,
        PanelMode::Expanded => ((cfg.expanded_height * scale).round() as i32).min(max_h),
    };
    let x = area.left + (area.right - area.left - w) / 2;
    let y = area.bottom - margin - h;
    debug!(
        "place {mode:?}: area=({},{})-({},{}) scale={scale} -> {w}x{h} at ({x},{y})",
        area.left, area.top, area.right, area.bottom
    );
    let _ = window.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}
