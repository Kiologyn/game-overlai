use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

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
    /// Current settings (the settings page changes them; some apply only after a restart).
    pub config: RwLock<Config>,
    /// The settings this run started with: agent, proxy and ports changed since need a restart.
    pub started_with: Config,
    pub config_file: std::path::PathBuf,
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
    pub hotkeys: Mutex<HotkeySet>,
    pub hotkeys_paused: AtomicBool,
    pub agent: AgentHandle,
    /// Our MCP server as offered to the agent (the `/mcp` route checks its token).
    pub mcp: McpEndpoint,
    pub capture: Capturer,
    pub agent_status: Mutex<AgentStatus>,
    pub usage: Mutex<Option<Usage>>,
    /// The agent's model / effort choices as last reported.
    pub agent_options: Mutex<Vec<crate::protocol::AgentOption>>,
    /// Hotkeys another program holds (shown in the tray tooltip).
    pub failed_hotkeys: Mutex<Vec<String>>,
    /// Set once Tauri is up; lets background tasks reach the tray.
    pub app: std::sync::OnceLock<tauri::AppHandle>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: Config,
        config_file: std::path::PathBuf,
        port: u16,
        ui_base: String,
        profiles_dir: std::path::PathBuf,
        history_dir: std::path::PathBuf,
        agent: AgentHandle,
        mcp: McpEndpoint,
    ) -> Self {
        let hotkeys = HotkeySet::parse(&config.hotkeys);
        let agent_status = AgentStatus { agent: agent.spec.name.into(), state: AgentState::Starting, detail: None };
        let history = History::open(&history_dir);
        let lan = Lan::load(&history_dir, config.lan.port);
        let chat = history
            .latest()
            .and_then(|id| history.load(&id))
            .map(|stored| ChatState::from_stored(stored, agent.spec.id))
            .unwrap_or_else(|| ChatState::empty(None));
        Self {
            started_with: config.clone(),
            config: RwLock::new(config),
            config_file,
            port,
            ui_base,
            profiles_dir,
            history,
            lan,
            hub: Hub::new(),
            chat: Mutex::new(chat),
            overlay: Mutex::new(OverlayState::default()),
            hotkeys: Mutex::new(hotkeys),
            hotkeys_paused: AtomicBool::new(false),
            agent,
            mcp,
            capture: Capturer::start(),
            agent_status: Mutex::new(agent_status),
            usage: Mutex::new(None),
            agent_options: Mutex::new(Vec::new()),
            failed_hotkeys: Mutex::new(Vec::new()),
            app: std::sync::OnceLock::new(),
        }
    }

    /// A copy of the current settings.
    pub fn cfg(&self) -> Config {
        self.config.read().unwrap().clone()
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
    /// Open (until Esc / ✕ / the hotkey closes it).
    pub visible: bool,
    /// Open, but put aside while another window is in front of the game.
    pub parked: bool,
    /// Frame of the game grabbed right before the hotkey showed the panel; dropped on hide.
    pub prefetch: Option<Prefetch>,
    /// The panel's current zoom (set when it is placed over a game).
    pub zoom: f64,
}

#[derive(Debug)]
pub struct Prefetch {
    pub frame: Arc<Frame>,
    /// Already given to the agent (only its first screenshot after the hotkey gets it).
    pub used: bool,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self { game_hwnd: 0, game: None, mode: PanelMode::Compact, visible: false, parked: false, prefetch: None, zoom: 1.0 }
    }
}

pub struct HotkeySet {
    pub quick: Option<Shortcut>,
    pub chat: Option<Shortcut>,
    pub focus: Option<Shortcut>,
}

impl HotkeySet {
    pub fn parse(keys: &crate::config::Hotkeys) -> Self {
        let parse = |name: &str, s: &str| match s.parse::<Shortcut>() {
            _ if s.is_empty() || s == "off" => None, // off in this game
            Ok(sc) => Some(sc),
            Err(e) => {
                warn!("hotkey {name} = {s:?} is invalid ({e}); it is disabled");
                None
            }
        };
        Self {
            quick: parse("quick", &keys.quick),
            chat: parse("chat", &keys.chat),
            focus: parse("focus", &keys.focus),
        }
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
