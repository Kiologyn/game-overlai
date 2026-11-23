//! Tray icon: left click opens the chat, right click shows the menu.
//! Later phases add: agent choice, settings, autostart.

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

/// "Access from the phone" item: kept in sync when access is changed on the /phone page.
static LAN_ITEM: OnceLock<CheckMenuItem<tauri::Wry>> = OnceLock::new();

pub fn build(app: &tauri::App) -> tauri::Result<()> {
    let st = app.state::<Arc<AppState>>().inner().clone();
    let quick_key = pretty_hotkey(&st.config.hotkeys.quick);
    let chat_key = pretty_hotkey(&st.config.hotkeys.chat);

    let quick = MenuItem::with_id(app, "quick", format!("Быстрый вопрос\t{quick_key}"), true, None::<&str>)?;
    let chat = MenuItem::with_id(app, "chat", format!("Открыть чат\t{chat_key}"), true, None::<&str>)?;
    let browser = MenuItem::with_id(app, "browser", "Открыть чат в браузере", true, None::<&str>)?;
    let lan_on = CheckMenuItem::with_id(app, "lan", "Доступ с телефона (Wi-Fi)", true, lan::enabled(&st), None::<&str>)?;
    let phone = MenuItem::with_id(app, "phone", "Телефон: QR-код…", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Пауза горячих клавиш", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
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
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let pause_item = pause.clone();
    let lan_item = lan_on.clone();
    let _ = LAN_ITEM.set(lan_on.clone());
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
            "pause" => {
                let paused = pause_item.is_checked().unwrap_or(false);
                hotkeys::set_paused(app, paused);
                refresh_tooltip(app);
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
    let agent_line = match agent.state {
        AgentState::Ready => format!("{}: готов", agent.agent),
        AgentState::Starting => format!("{}: запускается…", agent.agent),
        AgentState::Installing => format!("{}: установка…", agent.agent),
        AgentState::Error => format!("{}: недоступен", agent.agent),
    };
    let mut lines = vec![format!("game-overlai · {agent_line}")];
    if let Some(Usage { five_hour: Some(five), seven_day, .. }) = st.usage.lock().unwrap().clone() {
        let week = seven_day.map(|w| format!(" · неделя {:.0}%", w * 100.0)).unwrap_or_default();
        lines.push(format!("Лимит 5 ч: {:.0}%{week}", five * 100.0));
    }
    if lan::enabled(st) {
        lines.push("Доступ с телефона включён".into());
    }
    if st.hotkeys_paused.load(std::sync::atomic::Ordering::SeqCst) {
        lines.push("Горячие клавиши на паузе".into());
    } else {
        let failed = st.failed_hotkeys.lock().unwrap();
        if failed.is_empty() {
            lines.push(format!(
                "{} вопрос · {} чат",
                pretty_hotkey(&st.config.hotkeys.quick),
                pretty_hotkey(&st.config.hotkeys.chat)
            ));
        } else {
            lines.push(format!("Заняты: {}", failed.join(", ")));
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
    let url = format!("{}phone", app.state::<Arc<AppState>>().browser_url());
    open_url(app, &url);
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
        if let Some(item) = LAN_ITEM.get() {
            let _ = item.set_checked(lan::enabled(&st));
        }
        refresh_tooltip(&handle);
    });
}
