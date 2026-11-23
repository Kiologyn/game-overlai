use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tauri_plugin_global_shortcut::Shortcut;
use tokio::sync::broadcast;
use tracing::warn;

use crate::agent::{AgentHandle, McpEndpoint};
use crate::capture::{Capturer, Frame};
use crate::chat::ChatState;
use crate::config::Config;
use crate::history::History;
use crate::lan::Lan;
use crate::protocol::{AgentState, AgentStatus, GameInfo, PanelMode, ServerMsg, Usage};

pub struct AppState {
    pub config: Config,
    /// Port the UI server actually listens on (config.port, or a free one if that was taken).
    pub port: u16,
    /// Base URL the overlay window loads the UI from (the core itself, or the Vite dev server).
    pub ui_base: String,
    pub profiles_dir: std::path::PathBuf,
    pub history: History,
    /// Chat access for phones on the local network.
    pub lan: Lan,
    pub hub: Hub,
    pub chat: Mutex<ChatState>,
    pub overlay: Mutex<OverlayState>,
    pub hotkeys: HotkeySet,
    pub hotkeys_paused: AtomicBool,
    pub agent: AgentHandle,
    /// Our MCP server as offered to the agent (the `/mcp` route checks its token).
    pub mcp: McpEndpoint,
    pub capture: Capturer,
    pub agent_status: Mutex<AgentStatus>,
    pub usage: Mutex<Option<Usage>>,
    /// Hotkeys another program holds (shown in the tray tooltip).
    pub failed_hotkeys: Mutex<Vec<String>>,
    /// Set once Tauri is up; lets background tasks reach the tray.
    pub app: std::sync::OnceLock<tauri::AppHandle>,
}

impl AppState {
    pub fn new(
        config: Config,
        port: u16,
        ui_base: String,
        profiles_dir: std::path::PathBuf,
        history_dir: std::path::PathBuf,
        agent: AgentHandle,
        mcp: McpEndpoint,
    ) -> Self {
        let hotkeys = HotkeySet::parse(&config);
        let agent_status = AgentStatus { agent: agent.spec.name.into(), state: AgentState::Starting, detail: None };
        let history = History::open(&history_dir);
        let lan = Lan::load(&history_dir, config.lan.port);
        let chat = history
            .latest()
            .and_then(|id| history.load(&id))
            .map(|stored| ChatState::from_stored(stored, agent.spec.id))
            .unwrap_or_else(|| ChatState::empty(None));
        Self {
            config,
            port,
            ui_base,
            profiles_dir,
            history,
            lan,
            hub: Hub::new(),
            chat: Mutex::new(chat),
            overlay: Mutex::new(OverlayState::default()),
            hotkeys,
            hotkeys_paused: AtomicBool::new(false),
            agent,
            mcp,
            capture: Capturer::start(),
            agent_status: Mutex::new(agent_status),
            usage: Mutex::new(None),
            failed_hotkeys: Mutex::new(Vec::new()),
            app: std::sync::OnceLock::new(),
        }
    }

    /// Address for opening the chat in a browser on this PC.
    pub fn browser_url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }
}

#[derive(Debug)]
pub struct OverlayState {
    /// HWND (as isize, HWND is not Send) of the window that had focus when the panel was
    /// opened by a hotkey: the game. Focus goes back to it when the panel hides.
    pub game_hwnd: isize,
    pub game: Option<GameInfo>,
    pub mode: PanelMode,
    pub visible: bool,
    /// Frame of the game grabbed right before the hotkey showed the panel; dropped on hide.
    pub prefetch: Option<Prefetch>,
}

#[derive(Debug)]
pub struct Prefetch {
    pub frame: Arc<Frame>,
    /// Already attached to a message (only the first message after the hotkey gets it).
    pub used: bool,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self { game_hwnd: 0, game: None, mode: PanelMode::Compact, visible: false, prefetch: None }
    }
}

pub struct HotkeySet {
    pub quick: Option<Shortcut>,
    pub chat: Option<Shortcut>,
}

impl HotkeySet {
    fn parse(cfg: &Config) -> Self {
        let parse = |name: &str, s: &str| match s.parse::<Shortcut>() {
            Ok(sc) => Some(sc),
            Err(e) => {
                warn!("hotkey {name} = {s:?} is invalid ({e}); it is disabled");
                None
            }
        };
        Self { quick: parse("quick", &cfg.hotkeys.quick), chat: parse("chat", &cfg.hotkeys.chat) }
    }
}

/// Who receives a server message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Every connected UI (overlay + browser tabs).
    All,
    /// Only overlay panel connections.
    Overlay,
    /// UIs on this PC (overlay + local browser tabs), not devices on the network.
    Local,
}

/// Fan-out of server messages to all WebSocket sessions.
pub struct Hub {
    tx: broadcast::Sender<(Target, std::sync::Arc<ServerMsg>)>,
}

impl Hub {
    fn new() -> Self {
        Self { tx: broadcast::channel(256).0 }
    }

    pub fn send(&self, target: Target, msg: ServerMsg) {
        // No receivers is fine (no UI connected yet).
        let _ = self.tx.send((target, std::sync::Arc::new(msg)));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(Target, std::sync::Arc<ServerMsg>)> {
        self.tx.subscribe()
    }
}
