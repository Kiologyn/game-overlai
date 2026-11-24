//! Tray icon: left click opens the chat, right click shows the menu.
//! Settings (hotkeys, agent, proxy, autostart...) are on the /settings page in the browser.

use std::sync::{Arc, OnceLock};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tracing::warn;

use crate::config::pretty_hotkey;
use crate::hotkeys;
use crate::lan;
use crate::overlay::{self, ShowSource};
use crate::protocol::{AgentState, PanelMode, Usage};
use crate::state::AppState;

pub const TRAY_ID: &str = "main";

/// The menu items, renamed when the hotkeys or the language change; the "access from the phone"
/// check mark follows changes made on the /phone page.
struct Items {
    quick: MenuItem<tauri::Wry>,
    chat: MenuItem<tauri::Wry>,
    browser: MenuItem<tauri::Wry>,
    lan: CheckMenuItem<tauri::Wry>,
    phone: MenuItem<tauri::Wry>,
    pause: CheckMenuItem<tauri::Wry>,
    settings: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

static ITEMS: OnceLock<Items> = OnceLock::new();

fn quick_text(key: &str) -> String {
    tr!("tray_quick", key = pretty_hotkey(key))
}

fn chat_text(key: &str) -> String {
    tr!("tray_chat", key = pretty_hotkey(key))
}

pub fn build(app: &tauri::App) -> tauri::Result<()> {
    let st = app.state::<Arc<AppState>>().inner().clone();
    let keys = st.cfg().hotkeys;
    let quick = MenuItem::with_id(app, "quick", quick_text(&keys.quick), true, None::<&str>)?;
    let chat = MenuItem::with_id(app, "chat", chat_text(&keys.chat), true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", tr!("tray_settings"), true, None::<&str>)?;
    let browser = MenuItem::with_id(app, "browser", tr!("tray_browser"), true, None::<&str>)?;
    let lan_on = CheckMenuItem::with_id(app, "lan", tr!("tray_lan"), true, lan::enabled(&st), None::<&str>)?;
    let phone = MenuItem::with_id(app, "phone", tr!("tray_phone"), true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", tr!("tray_pause"), true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", tr!("tray_quit"), true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &quick,
            &chat,
            &browser,
            &PredefinedMenuItem::separator(app)?,
            &lan_on,
            &phone,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let pause_item = pause.clone();
    let lan_item = lan_on.clone();
    let _ = ITEMS.set(Items {
        quick: quick.clone(),
        chat: chat.clone(),
        browser: browser.clone(),
        lan: lan_on.clone(),
        phone: phone.clone(),
        pause: pause.clone(),
        settings: settings.clone(),
        quit: quit.clone(),
    });
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip(tooltip(&st))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "quick" => overlay::show(app, PanelMode::Compact, ShowSource::Other),
            "chat" => overlay::show(app, PanelMode::Expanded, ShowSource::Other),
            "browser" => open_in_browser(app),
            "lan" => {
                let on = lan_item.is_checked().unwrap_or(false);
                let st = app.state::<Arc<AppState>>().inner().clone();
                lan::set_enabled(&st, on);
                if on {
                    open_phone_page(app); // the QR code is what the user needs next
                }
            }
            "phone" => open_phone_page(app),
            "settings" => open_page(app, "settings"),
            "pause" => {
                let paused = pause_item.is_checked().unwrap_or(false);
                hotkeys::set_paused(app, paused);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                overlay::show(tray.app_handle(), PanelMode::Expanded, ShowSource::Other);
            }
        })
        .build(app)?;
    Ok(())
}

/// Windows truncates tray tooltips at 127 characters: keep every line short.
fn tooltip(st: &AppState) -> String {
    let agent = st.agent_status.lock().unwrap().clone();
    let state = match agent.state {
        AgentState::Ready => tr!("agent_ready"),
        AgentState::Starting => tr!("agent_starting"),
        AgentState::Installing => tr!("agent_installing"),
        AgentState::Error => tr!("agent_error"),
    };
    let agent_line = tr!("agent_state_line", agent = agent.agent, state = state);
    let mut lines = vec![format!("game-overlai · {agent_line}")];
    if let Some(Usage { five_hour: Some(five), seven_day, .. }) = st.usage.lock().unwrap().clone() {
        let mut line = tr!("tray_usage", percent = format!("{:.0}", five * 100.0));
        if let Some(w) = seven_day {
            line = format!("{line} · {}", tr!("tray_usage_week", percent = format!("{:.0}", w * 100.0)));
        }
        lines.push(line);
    }
    if lan::enabled(st) {
        lines.push(tr!("tray_lan_on"));
    }
    if st.hotkeys_paused.load(std::sync::atomic::Ordering::SeqCst) {
        lines.push(tr!("tray_paused"));
    } else {
        let failed = st.failed_hotkeys.lock().unwrap();
        if failed.is_empty() {
            let keys = st.cfg().hotkeys;
            lines.push(tr!("tray_keys", quick = pretty_hotkey(&keys.quick), chat = pretty_hotkey(&keys.chat)));
        } else {
            lines.push(tr!("tray_keys_taken", keys = failed.join(", ")));
        }
    }
    let mut s = lines.join("\n");
    s.truncate(s.char_indices().nth(120).map(|(i, _)| i).unwrap_or(s.len()));
    s
}

/// Re-renders the tooltip (agent status, usage, hotkey state changed).
pub fn refresh_tooltip(app: &AppHandle) {
    let st = app.state::<Arc<AppState>>();
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip(&st)));
    }
}

pub fn open_in_browser(app: &AppHandle) {
    let url = app.state::<Arc<AppState>>().browser_url();
    open_url(app, &url);
}

/// The /phone page on this PC: QR code, addresses, access on/off.
pub fn open_phone_page(app: &AppHandle) {
    open_page(app, "phone");
}

/// A page of the browser UI on this PC, e.g. "settings".
pub fn open_page(app: &AppHandle, page: &str) {
    let url = format!("{}{page}", app.state::<Arc<AppState>>().browser_url());
    open_url(app, &url);
}

/// Hotkeys changed in the settings (or were paused): menu labels and tooltip.
pub fn refresh_hotkeys(app: &AppHandle) {
    refresh_menu(app);
}

/// Every menu label and the tooltip again: the hotkeys or the language changed.
pub fn refresh_menu(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let keys = handle.state::<Arc<AppState>>().cfg().hotkeys;
        if let Some(items) = ITEMS.get() {
            let _ = items.quick.set_text(quick_text(&keys.quick));
            let _ = items.chat.set_text(chat_text(&keys.chat));
            let _ = items.browser.set_text(tr!("tray_browser"));
            let _ = items.lan.set_text(tr!("tray_lan"));
            let _ = items.phone.set_text(tr!("tray_phone"));
            let _ = items.pause.set_text(tr!("tray_pause"));
            let _ = items.settings.set_text(tr!("tray_settings"));
            let _ = items.quit.set_text(tr!("tray_quit"));
        }
        refresh_tooltip(&handle);
    });
}

fn open_url(app: &AppHandle, url: &str) {
    if let Err(e) = app.opener().open_url(url, None::<&str>) {
        warn!("could not open {url}: {e}");
    }
}

/// Syncs the "access from the phone" check mark (and the tooltip) with the actual state.
pub fn refresh_lan(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let st = handle.state::<Arc<AppState>>();
        if let Some(items) = ITEMS.get() {
            let _ = items.lan.set_checked(lan::enabled(&st));
        }
        refresh_tooltip(&handle);
    });
}
