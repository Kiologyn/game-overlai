use std::sync::Arc;
use std::sync::atomic::Ordering;

use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tracing::{info, warn};

use crate::overlay;
use crate::protocol::PanelMode;
use crate::state::AppState;

/// The plugin dispatches every registered shortcut to this handler (runs on the main thread).
pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let st = app.state::<Arc<AppState>>();
            if st.hotkeys.quick.as_ref().is_some_and(|s| s.id() == shortcut.id()) {
                overlay::on_hotkey(app, PanelMode::Compact);
            } else if st.hotkeys.chat.as_ref().is_some_and(|s| s.id() == shortcut.id()) {
                overlay::on_hotkey(app, PanelMode::Expanded);
            }
        })
        .build()
}

/// Registers the configured hotkeys. Returns the ones another program already holds.
pub fn register(app: &AppHandle) -> Vec<String> {
    let st = app.state::<Arc<AppState>>();
    let mut failed = Vec::new();
    for (name, shortcut) in [("quick", &st.hotkeys.quick), ("chat", &st.hotkeys.chat)] {
        let Some(sc) = shortcut else { continue };
        match app.global_shortcut().register(*sc) {
            Ok(()) => info!("hotkey {name}: {sc}"),
            Err(e) => {
                warn!("hotkey {name} ({sc}) could not be registered, probably used by another program: {e}");
                failed.push(sc.to_string());
            }
        }
    }
    failed
}

pub fn set_paused(app: &AppHandle, paused: bool) {
    let st = app.state::<Arc<AppState>>();
    st.hotkeys_paused.store(paused, Ordering::SeqCst);
    if paused {
        if let Err(e) = app.global_shortcut().unregister_all() {
            warn!("could not unregister hotkeys: {e}");
        }
        info!("hotkeys paused");
    } else {
        *st.failed_hotkeys.lock().unwrap() = register(app);
        info!("hotkeys resumed");
    }
}
