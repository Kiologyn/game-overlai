// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod capture;
mod chat;
mod config;
mod hotkeys;
mod history;
mod logging;
mod mcp;
mod overlay;
mod paths;
mod protocol;
mod proxy;
mod server;
mod shots;
mod state;
mod tray;

use std::sync::Arc;
use std::time::Duration;

use tauri::RunEvent;
use tracing::{error, info};

use crate::overlay::ShowSource;
use crate::protocol::PanelMode;
use crate::state::AppState;

fn main() {
    let paths = paths::Paths::resolve();
    let _log_guard = logging::init(&paths.log_dir);
    info!("game-overlai {} starting", env!("CARGO_PKG_VERSION"));

    let config = config::load(&paths.config_file);
    let (listener, port) = match server::bind(config.port) {
        Ok(v) => v,
        Err(e) => {
            error!("cannot start the UI server: {e}");
            return;
        }
    };
    // Dev: OVERLAI_UI_DEV=http://localhost:5173 loads the UI from Vite (hot reload).
    let ui_base = std::env::var("OVERLAI_UI_DEV")
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
    // Our MCP server lives on the UI server; a fresh token per run keeps other programs out.
    let mcp = agent::McpEndpoint {
        name: mcp::SERVER_NAME,
        url: format!("http://127.0.0.1:{port}/mcp"),
        token: uuid::Uuid::new_v4().simple().to_string(),
    };
    // The agent starts right away (install on first run, then a warm process).
    let (agent_events, agent_events_rx) = tokio::sync::mpsc::unbounded_channel();
    let spec = &agent::spec::CLAUDE;
    let agent = agent::AgentHandle::start(
        spec,
        paths.agents_dir.clone(),
        config.network.proxy.clone(),
        config.network.requires_proxy(spec.id),
        Some(mcp.clone()),
        agent_events,
    );
    let state = Arc::new(AppState::new(
        config,
        port,
        ui_base,
        paths.profiles_dir.clone(),
        paths.history_dir.clone(),
        agent,
        mcp,
    ));
    chat::spawn_event_loop(state.clone(), agent_events_rx);

    let app = tauri::Builder::default()
        // Must be first: a second launch just opens the chat in the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            overlay::show(app, PanelMode::Expanded, ShowSource::Other);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(hotkeys::plugin())
        .manage(state.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            let _ = state.app.set(handle.clone());
            tauri::async_runtime::spawn(server::run(listener, handle.clone(), state.clone()));
            overlay::create(&handle, &state)?;
            *state.failed_hotkeys.lock().unwrap() = hotkeys::register(&handle);
            tray::build(app)?;
            debug_show_on_start(&handle);
            info!("ready: {} = quick question, {} = chat", state.config.hotkeys.quick, state.config.hotkeys.chat);
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            error!("failed to start: {e}");
            return;
        }
    };
    app.run(|app, event| match event {
        // The app lives in the tray: hidden windows must not end it. Only "Выход" (exit(0)) does.
        RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
        // An answer still streaming is saved as far as it got.
        RunEvent::Exit => {
            use tauri::Manager;
            chat::flush(&app.state::<Arc<AppState>>());
        }
        _ => {}
    });
}

/// OVERLAI_DEBUG_SHOW=compact|expanded opens the panel 1.5 s after start (for UI checks).
fn debug_show_on_start(app: &tauri::AppHandle) {
    let mode = match std::env::var("OVERLAI_DEBUG_SHOW").as_deref() {
        Ok("compact") => PanelMode::Compact,
        Ok("expanded") => PanelMode::Expanded,
        _ => return,
    };
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        overlay::run_on_main(&app, move |app| overlay::show(app, mode, ShowSource::Hotkey));
        if std::env::var_os("OVERLAI_DEBUG_RESHOW").is_some() {
            std::thread::sleep(Duration::from_millis(1000));
            overlay::run_on_main(&app, overlay::hide);
            std::thread::sleep(Duration::from_millis(1000));
            overlay::run_on_main(&app, move |app| overlay::show(app, mode, ShowSource::Other));
        }
    });
}
