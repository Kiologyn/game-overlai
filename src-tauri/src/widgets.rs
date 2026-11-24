//! Widgets: small cards pinned over a game's window (each game has its own set, kept in the
//! history). The agent makes and changes them with MCP tools (`mcp.rs`); the user moves and
//! resizes them, ticks their checklist, makes them see-through or folds them.
//!
//! All widgets of the game in the foreground live in one transparent, topmost window (the
//! layer, `/widgets`) laid over the game window. It never takes the focus from the game
//! (WS_EX_NOACTIVATE), and it lets the mouse through everywhere except over a widget: the page
//! reports where its widgets are (`widgets.rects`) and a watcher thread turns click-through off
//! while the cursor is over one of them (or a drag that started on one goes on).

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tracing::{debug, info, warn};
use windows::Win32::Foundation::RECT;

use crate::games;
use crate::history::{now_ms, short_id};
use crate::overlay::{self, win32};
use crate::protocol::{ServerMsg, Widget, WidgetItem, WidgetRect};
use crate::state::{AppState, Target};

pub const LABEL: &str = "widgets";
/// Per game: each is part of the game's screen.
pub const MAX_PER_GAME: usize = 8;
pub const MAX_TITLE: usize = 80;
pub const MAX_BODY: usize = 2_000;
pub const MAX_ITEMS: usize = 30;
pub const MAX_ITEM: usize = 200;
pub const MAX_DATA: usize = 4_000;
pub const DEFAULT_WIDTH: f64 = 260.0;
pub const MIN_WIDTH: f64 = 160.0;
pub const MAX_WIDTH: f64 = 640.0;
const MAX_TIMER_S: f64 = 24.0 * 3600.0;

// ------------------------------------------------------------------ the widgets

/// What the agent sets on a widget; None keeps the current value.
#[derive(Debug, Default)]
pub struct Change {
    pub title: Option<String>,
    pub body: Option<String>,
    /// The whole checklist: text, and done (None: as it was for the same text, else not done).
    pub items: Option<Vec<(String, Option<bool>)>>,
    /// Countdown from now; 0 removes it.
    pub timer_seconds: Option<f64>,
    /// Replaces the agent's data; null clears it.
    pub data: Option<serde_json::Value>,
    pub width: Option<f64>,
}

/// The widgets of one game.
pub fn for_game(state: &AppState, exe: &str) -> Vec<Widget> {
    state.history.widgets().into_iter().filter(|w| w.exe.eq_ignore_ascii_case(exe)).collect()
}

fn find(state: &AppState, id: &str) -> Result<Widget, String> {
    let id = id.trim();
    state.history.widgets().into_iter().find(|w| w.id == id).ok_or_else(|| format!("no widget with id {id}"))
}

/// Errors are for the agent (English).
pub fn create(state: &AppState, exe: &str, change: Change) -> Result<Widget, String> {
    let exe = exe.trim();
    if exe.is_empty() {
        return Err("the game is unknown".into());
    }
    let count = for_game(state, exe).len();
    if count >= MAX_PER_GAME {
        return Err(format!("this game already has {MAX_PER_GAME} widgets: update or delete one"));
    }
    if change.title.as_deref().is_none_or(|t| t.trim().is_empty()) {
        return Err("a widget needs a title".into());
    }
    // New ones go down the right edge (the page keeps them inside the game window).
    let mut w = Widget {
        id: short_id(),
        exe: exe.to_string(),
        x: 1.0,
        y: (0.08 + 0.14 * count as f64).min(0.9),
        width: DEFAULT_WIDTH,
        ..Default::default()
    };
    apply(&mut w, change)?;
    w.updated_at = now_ms();
    state.history.save_widget(&w, &games::known_title(state, exe));
    info!("widget {} for {exe}", w.id);
    changed(state);
    Ok(w)
}

pub fn update(state: &AppState, id: &str, change: Change) -> Result<Widget, String> {
    let mut w = find(state, id)?;
    apply(&mut w, change)?;
    save(state, w)
}

pub fn delete(state: &AppState, id: &str) -> Result<(), String> {
    let w = find(state, id)?;
    state.history.delete_widget(&w.id);
    info!("widget {} deleted", w.id);
    changed(state);
    Ok(())
}

/// The user ticked or unticked a checklist item.
pub fn toggle_item(state: &AppState, id: &str, item_id: &str) {
    let Ok(mut w) = find(state, id) else { return };
    let Some(item) = w.items.iter_mut().find(|i| i.id == item_id) else { return };
    item.done = !item.done;
    let _ = save(state, w);
}

/// The user moved, resized, folded the widget or made it see-through.
pub fn set_view(
    state: &AppState,
    id: &str,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    translucent: Option<bool>,
    collapsed: Option<bool>,
) {
    let Ok(mut w) = find(state, id) else { return };
    if let Some(x) = x.filter(|v| v.is_finite()) {
        w.x = x.clamp(0.0, 1.0);
    }
    if let Some(y) = y.filter(|v| v.is_finite()) {
        w.y = y.clamp(0.0, 1.0);
    }
    if let Some(width) = width {
        w.width = clamp_width(width);
    }
    if let Some(t) = translucent {
        w.translucent = t;
    }
    if let Some(c) = collapsed {
        w.collapsed = c;
    }
    let _ = save(state, w);
}

fn save(state: &AppState, mut w: Widget) -> Result<Widget, String> {
    w.updated_at = now_ms();
    state.history.save_widget(&w, "");
    changed(state);
    Ok(w)
}

fn apply(w: &mut Widget, c: Change) -> Result<(), String> {
    if let Some(title) = c.title {
        let title = one_line(&title);
        if title.is_empty() {
            return Err("a widget needs a title".into());
        }
        w.title = games::cut(&title, MAX_TITLE);
    }
    if let Some(body) = c.body {
        w.body = games::cut(body.trim(), MAX_BODY);
    }
    if let Some(items) = c.items {
        if items.len() > MAX_ITEMS {
            return Err(format!("at most {MAX_ITEMS} checklist items"));
        }
        // Items with the same text keep their id and (unless told) their tick.
        let mut old = std::mem::take(&mut w.items);
        for (text, done) in items {
            let text = games::cut(&one_line(&text), MAX_ITEM);
            if text.is_empty() {
                continue;
            }
            let prev = old.iter().position(|o| o.text == text).map(|i| old.remove(i));
            w.items.push(WidgetItem {
                id: prev.as_ref().map(|p| p.id.clone()).unwrap_or_else(short_id),
                done: done.or(prev.map(|p| p.done)).unwrap_or(false),
                text,
            });
        }
    }
    if let Some(s) = c.timer_seconds {
        w.timer_ends_at = (s.is_finite() && s > 0.0).then(|| now_ms() + (s.min(MAX_TIMER_S) * 1000.0) as i64);
    }
    if let Some(data) = c.data {
        let text = if data.is_null() { String::new() } else { data.to_string() };
        if text.len() > MAX_DATA {
            return Err(format!("data is over {MAX_DATA} characters as JSON"));
        }
        w.data = text;
    }
    if let Some(width) = c.width {
        w.width = clamp_width(width);
    }
    Ok(())
}

fn clamp_width(width: f64) -> f64 {
    if width.is_finite() { width.round().clamp(MIN_WIDTH, MAX_WIDTH) } else { DEFAULT_WIDTH }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn snapshot(state: &AppState) -> ServerMsg {
    ServerMsg::Widgets { widgets: state.history.widgets() }
}

/// Every UI gets the new list; the layer may have to appear or go.
fn changed(state: &AppState) {
    state.hub.send(Target::All, snapshot(state));
    if let Some(app) = state.app.get() {
        overlay::run_on_main(app, refresh);
    }
}

// ------------------------------------------------------------------ the layer window

/// Which game window the layer lies over now.
#[derive(Debug, Clone)]
struct Shown {
    game: isize,
    exe: String,
    bounds: RECT,
    scale: f64,
}

struct Layer {
    shown: Option<Shown>,
    /// Where the page's widgets are (CSS px of the layer).
    rects: Vec<WidgetRect>,
    /// The layer takes the mouse (the cursor is over a widget).
    interactive: bool,
}

static LAYER: Mutex<Layer> = Mutex::new(Layer { shown: None, rects: Vec::new(), interactive: false });

/// How often the watcher checks the cursor (click-through), and every how many checks the game
/// window (moved, resized, minimised, closed).
const CURSOR_EVERY: Duration = Duration::from_millis(40);
const GAME_EVERY: u32 = 10;

pub fn create_layer(app: &AppHandle, state: &AppState) -> anyhow::Result<()> {
    let url = format!("{}/widgets", state.ui_base).parse()?;
    let owner = win32::create_owner_window()?;
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .owner_raw(owner)
        .title("game-overlai widgets")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .focused(false)
        .focusable(false)
        .build()?;
    let _ = window.set_ignore_cursor_events(true);
    win32::register_layer(window.hwnd().unwrap_or_default());
    window.on_window_event(|event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
        }
    });
    let watcher = app.clone();
    std::thread::Builder::new().name("widgets".into()).spawn(move || watch(watcher))?;
    refresh(app);
    info!("widget layer created");
    Ok(())
}

/// The page reported where its widgets are.
pub fn set_rects(rects: Vec<WidgetRect>) {
    LAYER.lock().unwrap().rects = rects;
}

/// Which game's widgets are shown now (for a page that connects later).
pub fn shown_msg() -> ServerMsg {
    ServerMsg::WidgetsShown { exe: LAYER.lock().unwrap().shown.as_ref().map(|s| s.exe.clone()) }
}

/// Main thread. Lays the layer over the game in the foreground if it has widgets, else hides
/// it. Our own windows (the panel) in the foreground keep the game it is over.
pub fn refresh(app: &AppHandle) {
    let Some(window) = app.get_webview_window(LABEL) else { return };
    let st = app.state::<std::sync::Arc<AppState>>().inner().clone();
    let fg = win32::foreground();
    let current = LAYER.lock().unwrap().shown.clone();
    let candidate = if fg.is_invalid() || win32::is_own(fg) {
        current.as_ref().map(|s| (win32::hwnd(s.game), s.exe.clone()))
    } else if win32::is_shell_window(fg) {
        None
    } else {
        win32::process_exe(fg).map(|exe| (fg, exe))
    };
    let target = candidate.filter(|(h, exe)| {
        win32::is_alive(*h) && !win32::is_minimized(*h) && !for_game(&st, exe).is_empty()
    });

    let Some((game, exe)) = target else {
        if current.is_some() {
            let _ = window.hide();
            let mut layer = LAYER.lock().unwrap();
            layer.shown = None;
            layer.interactive = false;
            drop(layer);
            let _ = window.set_ignore_cursor_events(true);
            st.hub.send(Target::Local, ServerMsg::WidgetsShown { exe: None });
            debug!("widget layer hidden");
        }
        return;
    };
    let (bounds, scale) = win32::panel_bounds(Some(game));
    let next = Shown { game: win32::raw(game), exe: exe.clone(), bounds, scale };
    let moved = current.as_ref().is_none_or(|c| c.bounds != bounds);
    if moved {
        let _ = window.set_position(PhysicalPosition::new(bounds.left, bounds.top));
        let _ = window.set_size(PhysicalSize::new((bounds.right - bounds.left) as u32, (bounds.bottom - bounds.top) as u32));
    }
    if current.is_none() {
        let _ = window.show(); // not focusable: shown without taking the focus
        let _ = window.set_always_on_top(true);
        overlay::keep_panel_on_top(app);
    }
    let new_game = current.as_ref().is_none_or(|c| !c.exe.eq_ignore_ascii_case(&exe));
    LAYER.lock().unwrap().shown = Some(next);
    if new_game {
        st.hub.send(Target::Local, ServerMsg::WidgetsShown { exe: Some(exe.clone()) });
        debug!("widget layer over {exe} at {:?}", (bounds.left, bounds.top, bounds.right, bounds.bottom));
    }
}

/// Click-through outside the widgets, and following the game window.
fn watch(app: AppHandle) {
    let mut tick = 0u32;
    loop {
        std::thread::sleep(CURSOR_EVERY);
        tick = tick.wrapping_add(1);
        let (shown, over, interactive) = {
            let layer = LAYER.lock().unwrap();
            let Some(shown) = layer.shown.clone() else { continue };
            let (cx, cy) = win32::cursor_pos();
            let x = (cx - shown.bounds.left) as f64 / shown.scale;
            let y = (cy - shown.bounds.top) as f64 / shown.scale;
            let over = layer
                .rects
                .iter()
                .any(|r| x >= r.x - 2.0 && x <= r.x + r.width + 2.0 && y >= r.y - 2.0 && y <= r.y + r.height + 2.0);
            (shown, over, layer.interactive)
        };
        if tick.is_multiple_of(GAME_EVERY) {
            let game = win32::hwnd(shown.game);
            if !win32::is_alive(game) || win32::is_minimized(game) || win32::panel_bounds(Some(game)).0 != shown.bounds {
                overlay::run_on_main(&app, refresh);
                continue;
            }
        }
        // A drag that started on a widget goes on even if the page hasn't reported the new place yet.
        let want = over || (interactive && win32::left_button_down());
        if want != interactive {
            LAYER.lock().unwrap().interactive = want;
            match app.get_webview_window(LABEL) {
                Some(w) => {
                    if let Err(e) = w.set_ignore_cursor_events(!want) {
                        warn!("widget layer click-through: {e}");
                    }
                }
                None => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> Widget {
        Widget { id: "w".into(), exe: "game.exe".into(), title: "T".into(), width: DEFAULT_WIDTH, ..Default::default() }
    }

    #[test]
    fn checklists_keep_ticks_by_text() {
        let mut w = widget();
        let items = |list: &[(&str, Option<bool>)]| Change {
            items: Some(list.iter().map(|(t, d)| (t.to_string(), *d)).collect()),
            ..Default::default()
        };
        apply(&mut w, items(&[("Iron", None), ("Wood", Some(true)), ("  ", None)])).unwrap();
        assert_eq!(w.items.iter().map(|i| (i.text.as_str(), i.done)).collect::<Vec<_>>(), [("Iron", false), ("Wood", true)]);
        let wood = w.items[1].id.clone();
        apply(&mut w, items(&[("Stone", None), ("Wood", None)])).unwrap();
        assert_eq!(w.items[1].id, wood, "same text, same item");
        assert!(w.items[1].done, "its tick stays");
        assert!(!w.items[0].done);
        // Two items with the same text get two ids.
        apply(&mut w, items(&[("Wood", None), ("Wood", None)])).unwrap();
        assert_ne!(w.items[0].id, w.items[1].id);
    }

    #[test]
    fn values_are_cleaned() {
        let mut w = widget();
        let c = Change {
            title: Some("  Smokes \n on Mirage ".into()),
            width: Some(5000.0),
            timer_seconds: Some(90.0),
            data: Some(serde_json::json!({"kills": 3})),
            ..Default::default()
        };
        apply(&mut w, c).unwrap();
        assert_eq!((w.title.as_str(), w.width), ("Smokes on Mirage", MAX_WIDTH));
        let left = w.timer_ends_at.unwrap() - now_ms();
        assert!((89_000..=90_000).contains(&left), "{left}");
        assert_eq!(w.data, r#"{"kills":3}"#);
        apply(&mut w, Change { timer_seconds: Some(0.0), data: Some(serde_json::Value::Null), ..Default::default() }).unwrap();
        assert_eq!((w.timer_ends_at, w.data.as_str()), (None, ""));
        assert!(apply(&mut w, Change { title: Some(" ".into()), ..Default::default() }).is_err());
        let big = serde_json::Value::String("x".repeat(MAX_DATA));
        assert!(apply(&mut w, Change { data: Some(big), ..Default::default() }).is_err());
    }
}
