//! The overlay panel: one transparent, always-on-top, frameless window that is sized exactly
//! to the panel (so no invisible part of it eats clicks meant for the game).
//!
//!   compact  — only the input field
//!   expanded — chat history above the field; the window grows upward
//!
//! The user can move the panel (header / grip) and resize it (edges) within the game window;
//! where it ends up is remembered per game, relative to the game window (`history::PanelLayout`).
//! Ctrl + wheel / Ctrl +/-/0 zoom it: the content and the window scale together, the bottom
//! edge's centre stays put; the zoom is remembered per game too.
//!
//! The panel takes keyboard focus when shown and gives it back to the game when hidden.
//! It may stay open without focus while the user clicks into the game; once focus goes to
//! another app (neither the game nor the panel), it hides.
//! All functions here must run on the main thread (use `run_on_main`).

pub mod win32;

use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use windows::Win32::Foundation::RECT;
use tracing::{debug, info, warn};

use crate::history::PanelLayout;
use crate::protocol::{GameInfo, PanelMode, ResizeEdge, ServerMsg, ZoomStep};
use crate::state::{AppState, Prefetch, Target};

pub const LABEL: &str = "panel";
/// Height of the input row, logical px. Must match `--composer-h` in the UI.
pub const COMPACT_H: f64 = 56.0;
/// Smallest panel the user can resize to, logical px.
const MIN_W: f64 = 360.0;
const MIN_H: f64 = 220.0;
/// Zoom steps, as in a browser.
const ZOOM_STEPS: [f64; 11] = [0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

/// The zoom step nearest to `z` (config values, the settings page).
pub fn snap_zoom(z: f64) -> f64 {
    let z = if z.is_finite() { z } else { 1.0 };
    ZOOM_STEPS.into_iter().min_by(|a, b| (a - z).abs().total_cmp(&(b - z).abs())).unwrap_or(1.0)
}

fn next_zoom(current: f64, step: ZoomStep, default: f64) -> f64 {
    const EPS: f64 = 1e-6;
    match step {
        ZoomStep::In => ZOOM_STEPS.into_iter().find(|s| *s > current + EPS).unwrap_or(ZOOM_STEPS[ZOOM_STEPS.len() - 1]),
        ZoomStep::Out => ZOOM_STEPS.into_iter().rev().find(|s| *s < current - EPS).unwrap_or(ZOOM_STEPS[0]),
        ZoomStep::Reset => default,
    }
}

/// Focus elsewhere this long hides the panel (not a window passing by on the way back).
const FOCUS_AWAY: Duration = Duration::from_millis(250);

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
        .inner_size(state.config.read().unwrap().panel.width, COMPACT_H)
        .build()?;

    let hwnd = panel_hwnd(&window);
    apply_capturable(state, hwnd);

    // Moves and resizes by the user stay inside the game window; the result is remembered.
    let on_moved = app.clone();
    if !win32::limit_moves(hwnd, move || run_on_main(&on_moved, save_layout)) {
        warn!("could not limit panel moves to the game window");
    }

    // Focus went to another app: hide the panel (checked again after a moment).
    let on_focus = app.clone();
    let watching = win32::watch_foreground(move |fg| {
        if fg.is_invalid() || win32::is_own(fg) || !self::state(&on_focus).overlay.lock().unwrap().visible {
            return;
        }
        let app = on_focus.clone();
        std::thread::spawn(move || {
            std::thread::sleep(FOCUS_AWAY);
            run_on_main(&app, hide_if_focus_away);
        });
    });
    if !watching {
        warn!("could not watch the focus: the panel won't hide when another app gets it");
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

/// The settings changed: show or hide the panel in screen captures.
pub fn update_capturable(app: &AppHandle) {
    if let Some(w) = panel(app) {
        apply_capturable(&state(app), panel_hwnd(&w));
    }
}

/// The panel in screenshots, recordings and streams: hidden unless the user lets it in
/// (`[panel] capturable`, or OVERLAI_DEBUG_CAPTURABLE for tests). Frames for the agent never
/// show it (`capture::wgc` hides it for monitor captures).
fn apply_capturable(st: &AppState, hwnd: windows::Win32::Foundation::HWND) {
    let forced = std::env::var_os("OVERLAI_DEBUG_CAPTURABLE").is_some();
    let on = forced || st.config.read().unwrap().panel.capturable;
    if win32::set_capturable(hwnd, on) {
        info!("panel {} in screen captures{}", if on { "visible" } else { "hidden" }, if forced { " (OVERLAI_DEBUG_CAPTURABLE)" } else { "" });
    } else {
        warn!("could not make the panel {} in screen captures", if on { "visible" } else { "hidden" });
    }
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

/// A question from a phone: the game is the window in the foreground on the PC right now (the
/// panel may never have been opened over it). Our own windows and the desktop keep the game
/// known so far.
pub fn adopt_foreground_game(st: &AppState) {
    let fg = win32::foreground();
    if !win32::is_alive(fg) || win32::is_shell_window(fg) {
        return;
    }
    let exe = win32::process_exe(fg).unwrap_or_default();
    let own = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .is_some_and(|me| me.eq_ignore_ascii_case(&exe));
    if own {
        return;
    }
    let mut ov = st.overlay.lock().unwrap();
    if ov.game_hwnd == win32::raw(fg) {
        return;
    }
    let game = GameInfo { title: win32::window_title(fg), exe };
    debug!("question from a device on the network: game is {game:?}");
    ov.game_hwnd = win32::raw(fg);
    ov.game = Some(game);
    ov.prefetch = None;
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
    hide_panel(app, true);
}

/// Focus is neither on the panel nor on the game (another app, the desktop, Start...): hide
/// the panel, leaving the focus where the user put it.
fn hide_if_focus_away(app: &AppHandle) {
    let st = state(app);
    let (visible, game_hwnd) = {
        let ov = st.overlay.lock().unwrap();
        (ov.visible, ov.game_hwnd)
    };
    let fg = win32::foreground();
    if !visible || fg.is_invalid() || win32::is_own(fg) {
        return;
    }
    let game = win32::hwnd(game_hwnd);
    if win32::is_alive(game) && (fg == game || win32::process_id(fg) == win32::process_id(game)) {
        return; // the user clicked into the game: the panel stays
    }
    debug!(
        "focus went to {:?} ({}): hiding the panel",
        win32::window_title(fg),
        win32::process_exe(fg).unwrap_or_default()
    );
    hide_panel(app, false);
}

/// `give_focus_back`: activate the game (the user closed the panel), not when the focus is
/// already elsewhere.
fn hide_panel(app: &AppHandle, give_focus_back: bool) {
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
    if give_focus_back && win32::is_alive(game) && !win32::is_minimized(game) {
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

/// Panel layouts are kept per game (exe); "" = no game.
fn layout_key(st: &AppState) -> String {
    st.overlay.lock().unwrap().game.as_ref().map(|g| g.exe.to_lowercase()).unwrap_or_default()
}

/// Where the user left the panel over this game, or bottom-centre of it by default. The bottom
/// edge stays put when the panel expands. Always inside the game window (`win32::panel_bounds`).
/// Goes through Tauri so tao's view of the window stays in sync.
fn place(st: &AppState, window: &WebviewWindow, mode: PanelMode, game_hwnd: isize) {
    let game = win32::hwnd(game_hwnd);
    let (b, scale) = win32::panel_bounds(win32::is_alive(game).then_some(game));
    let saved = st.history.panel_layout(&layout_key(st));
    let cfg = st.config.read().unwrap().panel.clone();
    // Saved sizes already are at the saved zoom; the defaults are at 100%.
    let zoom = snap_zoom(saved.map_or(cfg.zoom, |l| l.zoom));
    st.overlay.lock().unwrap().zoom = zoom;
    let (bw, bh) = (b.right - b.left, b.bottom - b.top);
    let px = |v: f64| (v * scale).round() as i32;
    let (min_w, min_h) = (px(MIN_W * zoom).min(bw), px(MIN_H * zoom).min(bh));
    let w = px(saved.map_or(cfg.width * zoom, |l| l.width)).clamp(min_w, bw);
    let h = match mode {
        PanelMode::Compact => px(COMPACT_H * zoom).min(bh),
        PanelMode::Expanded => px(saved.map_or(cfg.expanded_height * zoom, |l| l.height)).clamp(min_h, bh),
    };
    let center = saved.map_or(b.left + bw / 2, |l| b.left + (l.center_x * bw as f64).round() as i32);
    let bottom = saved.map_or(b.bottom - px(cfg.bottom_margin), |l| b.top + (l.bottom * bh as f64).round() as i32);
    let mut r = RECT { left: center - w / 2, top: bottom - h, right: center - w / 2 + w, bottom };
    win32::clamp_move(&mut r, &b);
    win32::set_move_limits(win32::MoveLimits {
        bounds: b,
        min_w,
        min_h,
        fixed_h: (mode == PanelMode::Compact).then_some(h),
    });
    debug!(
        "place {mode:?}: bounds=({},{})-({},{}) scale={scale} zoom={zoom} saved={} -> {w}x{h} at ({},{})",
        b.left,
        b.top,
        b.right,
        b.bottom,
        saved.is_some(),
        r.left,
        r.top
    );
    if let Err(e) = window.set_zoom(zoom) {
        warn!("panel zoom: {e}");
    }
    let _ = window.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = window.set_position(PhysicalPosition::new(r.left, r.top));
}

/// Ctrl + wheel / Ctrl +/-/0 on the panel: scale its content and the window together around
/// the centre of the bottom edge. Scales the remembered layout (not the window as it is: that
/// may be cut by the game window's edges) and places the panel from it, so zooming in and back
/// out returns the panel to its size and place.
pub fn zoom(app: &AppHandle, step: ZoomStep) {
    let Some(w) = panel(app) else { return };
    if !w.is_visible().unwrap_or(false) {
        return;
    }
    let st = state(app);
    let cfg = st.config.read().unwrap().panel.clone();
    let (mode, game_hwnd, old) = {
        let ov = st.overlay.lock().unwrap();
        (ov.mode, ov.game_hwnd, ov.zoom)
    };
    let new = next_zoom(old, step, snap_zoom(cfg.zoom));
    if (new - old).abs() < 1e-6 {
        st.hub.send(Target::Overlay, ServerMsg::OverlayZoom { zoom: new }); // at the limit: show it
        return;
    }
    let key = layout_key(&st);
    let layout = match st.history.panel_layout(&key) {
        Some(l) => l,
        // Never moved over this game: the default size, where the panel is now.
        None => {
            let game = win32::hwnd(game_hwnd);
            let (b, _) = win32::panel_bounds(win32::is_alive(game).then_some(game));
            let (bw, bh) = (b.right - b.left, b.bottom - b.top);
            if bw <= 0 || bh <= 0 {
                return;
            }
            let r = win32::window_rect(panel_hwnd(&w));
            PanelLayout {
                width: cfg.width * old,
                height: cfg.expanded_height * old,
                center_x: ((r.left + (r.right - r.left) / 2 - b.left) as f64 / bw as f64).clamp(0.0, 1.0),
                bottom: ((r.bottom - b.top) as f64 / bh as f64).clamp(0.0, 1.0),
                zoom: old,
            }
        }
    };
    let k = new / old;
    let layout = PanelLayout { width: layout.width * k, height: layout.height * k, zoom: new, ..layout };
    debug!("panel zoom {old} -> {new}: {layout:?}");
    st.history.save_panel_layout(&key, &layout);
    place(&st, &w, mode, game_hwnd);
    st.hub.send(Target::Overlay, ServerMsg::OverlayZoom { zoom: new });
}

/// The UI's drag area was pressed: move the panel with the mouse (native move loop; the bounds
/// are enforced in `win32::limit_moves`).
pub fn start_drag(app: &AppHandle) {
    if let Some(w) = panel(app) {
        debug!("panel move started");
        let _ = w.start_dragging();
    }
}

/// A resize handle was pressed. The window is resizable only during the drag, so Windows can't
/// resize it otherwise (snap, maximise).
pub fn start_resize(app: &AppHandle, edge: ResizeEdge) {
    let Some(w) = panel(app) else { return };
    let w = w.as_ref().window(); // resize dragging is on the Window, not the WebviewWindow
    let compact = state(app).overlay.lock().unwrap().mode == PanelMode::Compact;
    if compact && !matches!(edge, ResizeEdge::E | ResizeEdge::W) {
        return; // the compact panel only gets wider or narrower
    }
    use tauri_runtime::ResizeDirection as D;
    let direction = match edge {
        ResizeEdge::N => D::North,
        ResizeEdge::S => D::South,
        ResizeEdge::E => D::East,
        ResizeEdge::W => D::West,
        ResizeEdge::Ne => D::NorthEast,
        ResizeEdge::Nw => D::NorthWest,
        ResizeEdge::Se => D::SouthEast,
        ResizeEdge::Sw => D::SouthWest,
    };
    debug!("panel resize started: {edge:?}");
    let _ = w.set_resizable(true);
    if let Err(e) = w.start_resize_dragging(direction) {
        let _ = w.set_resizable(false);
        warn!("resize: {e}");
    }
}

/// A move or resize by the user ended: remember where the panel is over this game.
fn save_layout(app: &AppHandle) {
    let Some(w) = panel(app) else { return };
    let _ = w.set_resizable(false);
    let st = state(app);
    let (mode, game_hwnd) = {
        let ov = st.overlay.lock().unwrap();
        (ov.mode, ov.game_hwnd)
    };
    let game = win32::hwnd(game_hwnd);
    let (b, scale) = win32::panel_bounds(win32::is_alive(game).then_some(game));
    let (bw, bh) = (b.right - b.left, b.bottom - b.top);
    if bw <= 0 || bh <= 0 {
        return;
    }
    let r = win32::window_rect(panel_hwnd(&w));
    let (pw, ph) = (r.right - r.left, r.bottom - r.top);
    let key = layout_key(&st);
    let zoom = st.overlay.lock().unwrap().zoom;
    let height = match mode {
        PanelMode::Expanded => ph as f64 / scale,
        // The compact panel only changes width: keep the expanded height.
        PanelMode::Compact => st
            .history
            .panel_layout(&key)
            .map_or(st.config.read().unwrap().panel.expanded_height * zoom, |l| l.height),
    };
    let layout = PanelLayout {
        width: pw as f64 / scale,
        height,
        center_x: ((r.left + pw / 2 - b.left) as f64 / bw as f64).clamp(0.0, 1.0),
        bottom: ((r.bottom - b.top) as f64 / bh as f64).clamp(0.0, 1.0),
        zoom,
    };
    debug!("panel layout for {:?}: {layout:?}", if key.is_empty() { "-" } else { key.as_str() });
    st.history.save_panel_layout(&key, &layout);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps() {
        assert_eq!(snap_zoom(1.24), 1.25);
        assert_eq!(snap_zoom(9.0), 2.0);
        assert_eq!(snap_zoom(f64::NAN), 1.0);
        assert_eq!(next_zoom(1.0, ZoomStep::In, 1.0), 1.1);
        assert_eq!(next_zoom(1.0, ZoomStep::Out, 1.0), 0.9);
        assert_eq!(next_zoom(1.3, ZoomStep::In, 1.0), 1.5, "off-step values go to the next step");
        assert_eq!(next_zoom(2.0, ZoomStep::In, 1.0), 2.0);
        assert_eq!(next_zoom(0.5, ZoomStep::Out, 1.0), 0.5);
        assert_eq!(next_zoom(1.75, ZoomStep::Reset, 1.25), 1.25, "back to the default");
    }
}
