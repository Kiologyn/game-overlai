//! Windows notifications for questions asked in the overlay: the user hid the panel to keep
//! playing, and the answer is ready (or the agent waits for a permission). Not for questions
//! from the browser or the phone: the user is looking at that screen. Off in the settings.

use tauri_plugin_notification::NotificationExt;
use tracing::{debug, warn};

use crate::config::pretty_hotkey;
use crate::state::AppState;

/// Identifier of the app (tauri.conf.json); Windows shows notifications under it.
const APP_ID: &str = "dev.gameoverlai.app";

/// Notifications go out only when the panel is hidden and the user wants them.
fn wanted(state: &AppState) -> bool {
    state.cfg().notifications.enabled && !state.overlay.lock().unwrap().visible
}

pub fn answer_ready(state: &AppState, title: Option<&str>, answer: &str) {
    if !wanted(state) {
        return;
    }
    let key = pretty_hotkey(&state.cfg().hotkeys.chat);
    let body = format!("{}\n{}", excerpt(answer, 140), tr!("notify_open_chat", key = key));
    show(state, title.unwrap_or(crate::i18n::text("notify_answer_ready")), &body);
}

pub fn answer_failed(state: &AppState, error: &str) {
    if wanted(state) {
        show(state, crate::i18n::text("notify_answer_failed"), &excerpt(error, 160));
    }
}

pub fn permission(state: &AppState, what: &str) {
    if !wanted(state) {
        return;
    }
    let key = pretty_hotkey(&state.cfg().hotkeys.chat);
    let body = format!("{}\n{}", excerpt(what, 140), tr!("notify_respond", key = key));
    show(state, crate::i18n::text("notify_permission"), &body);
}

fn show(state: &AppState, title: &str, body: &str) {
    let Some(app) = state.app.get() else { return };
    debug!("notification: {title}");
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        warn!("notification not shown: {e}");
    }
}

/// First words of `text` in one line, Markdown marks dropped.
fn excerpt(text: &str, max: usize) -> String {
    let marks: String = text.chars().filter(|c| !matches!(c, '*' | '#' | '`' | '_' | '>')).collect();
    let plain = marks.split_whitespace().collect::<Vec<_>>().join(" ");
    let plain = plain.as_str();
    match plain.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", plain[..i].trim_end()),
        None => plain.to_string(),
    }
}

/// A copy of the app run from anywhere but cargo's target dir sets its own app id on
/// notifications (tauri-plugin-notification); Windows shows those only for an app id that is
/// registered: the installer's shortcut does it, this covers the portable exe too.
pub fn register_app_id() {
    let Ok(exe) = std::env::current_exe() else { return };
    let dir = exe.parent().map(|d| d.display().to_string().to_lowercase()).unwrap_or_default();
    if dir.ends_with(r"\target\debug") || dir.ends_with(r"\target\release") {
        return; // the plugin uses PowerShell's id there
    }
    let key = format!(r"Software\Classes\AppUserModelId\{APP_ID}");
    let icon = std::env::var_os("LOCALAPPDATA")
        .map(|d| std::path::PathBuf::from(d).join("game-overlai").join("icon.png"))
        .filter(|p| std::fs::write(p, include_bytes!("../icons/128x128.png")).is_ok());
    let mut result = crate::settings::autostart::set_string(&key, "DisplayName", "game-overlai");
    if let Some(icon) = icon {
        result = result.and(crate::settings::autostart::set_string(&key, "IconUri", &icon.display().to_string()));
    }
    if let Err(e) = result {
        warn!("notifications: app id not registered: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpts() {
        assert_eq!(excerpt("**Ответ:**\n\n# Железо\nнайти в `болоте`", 100), "Ответ: Железо найти в болоте");
        assert_eq!(excerpt("один два три", 4), "один…");
    }
}
