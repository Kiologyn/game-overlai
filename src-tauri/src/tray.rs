//! Tray icon: left click opens the chat, right click shows the menu.
//! Later phases add: agent status and subscription limits in the tooltip, agent choice,
//! phone QR code, settings, autostart.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tracing::warn;

use crate::config::pretty_hotkey;
use crate::hotkeys;
use crate::overlay::{self, ShowSource};
use crate::protocol::{AgentState, PanelMode, Usage};
use crate::state::AppState;

pub const TRAY_ID: &str = "main";

pub fn build(app: &tauri::App) -> tauri::Result<()> {
    let st = app.state::<Arc<AppState>>().inner().clone();
    let quick_key = pretty_hotkey(&st.config.hotkeys.quick);
    let chat_key = pretty_hotkey(&st.config.hotkeys.chat);

    let quick = MenuItem::with_id(app, "quick", format!("Быстрый вопрос\t{quick_key}"), true, None::<&str>)?;
    let chat = MenuItem::with_id(app, "chat", format!("Открыть чат\t{chat_key}"), true, None::<&str>)?;
    let browser = MenuItem::with_id(app, "browser", "Открыть чат в браузере", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Пауза горячих клавиш", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &quick,
            &chat,
            &browser,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let pause_item = pause.clone();
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip(tooltip(&st))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "quick" => overlay::show(app, PanelMode::Compact, ShowSource::Other),
            "chat" => overlay::show(app, PanelMode::Expanded, ShowSource::Other),
            "browser" => open_in_browser(app),
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
    if let Err(e) = app.opener().open_url(&url, None::<&str>) {
        warn!("could not open {url}: {e}");
    }
}
