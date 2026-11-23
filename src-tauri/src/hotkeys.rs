use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tracing::{info, warn};

use crate::overlay;
use crate::protocol::PanelMode;
use crate::state::{AppState, HotkeySet};

/// The settings page is recording a new hotkey: ours are off meanwhile.
static CAPTURING: AtomicBool = AtomicBool::new(false);

/// The plugin dispatches every registered shortcut to this handler (runs on the main thread).
pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let st = app.state::<Arc<AppState>>();
            let (quick, chat) = {
                let keys = st.hotkeys.lock().unwrap();
                (keys.quick.is_some_and(|s| s.id() == shortcut.id()), keys.chat.is_some_and(|s| s.id() == shortcut.id()))
            };
            if quick {
                overlay::on_hotkey(app, PanelMode::Compact);
            } else if chat {
                overlay::on_hotkey(app, PanelMode::Expanded);
            }
        })
        .build()
}

/// Registers the configured hotkeys. Returns the ones another program already holds.
pub fn register(app: &AppHandle) -> Vec<String> {
    let st = app.state::<Arc<AppState>>();
    let keys = {
        let set = st.hotkeys.lock().unwrap();
        [("quick", set.quick), ("chat", set.chat)]
    };
    let mut failed = Vec::new();
    for (name, shortcut) in keys {
        let Some(sc) = shortcut else { continue };
        match app.global_shortcut().register(sc) {
            Ok(()) => info!("hotkey {name}: {sc}"),
            Err(e) => {
                warn!("hotkey {name} ({sc}) could not be registered, probably used by another program: {e}");
                failed.push(sc.to_string());
            }
        }
    }
    failed
}

fn active(st: &AppState) -> bool {
    !st.hotkeys_paused.load(Ordering::SeqCst) && !CAPTURING.load(Ordering::SeqCst)
}

/// Registers the hotkeys again (new ones from the settings, or after a pause).
pub fn reload(app: &AppHandle) {
    let st = app.state::<Arc<AppState>>();
    if let Err(e) = app.global_shortcut().unregister_all() {
        warn!("could not unregister hotkeys: {e}");
    }
    *st.hotkeys.lock().unwrap() = HotkeySet::parse(&st.cfg().hotkeys);
    let failed = if active(&st) { register(app) } else { vec![] };
    *st.failed_hotkeys.lock().unwrap() = failed;
    crate::tray::refresh_hotkeys(app);
}

pub fn set_paused(app: &AppHandle, paused: bool) {
    let st = app.state::<Arc<AppState>>();
    st.hotkeys_paused.store(paused, Ordering::SeqCst);
    reload(app);
    info!("hotkeys {}", if paused { "paused" } else { "resumed" });
}

/// The settings page started/stopped recording a hotkey.
pub fn set_capturing(app: &AppHandle, capturing: bool) {
    if CAPTURING.swap(capturing, Ordering::SeqCst) != capturing {
        reload(app);
    }
}
