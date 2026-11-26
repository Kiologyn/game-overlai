use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};
use tracing::{info, warn};

use crate::protocol::Language;

/// User settings, stored as TOML. Missing keys fall back to defaults, so old files keep working.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Local port of the UI/WebSocket server (the chat also opens in a browser on this port).
    pub port: u16,
    /// Agent id: "claude" or "codex".
    pub agent: String,
    /// Interface language: "en", "ru", "es", "zh", "ja". Default: the system's, if the app has it.
    #[serde(deserialize_with = "lenient_language")]
    pub language: Language,
    pub hotkeys: Hotkeys,
    pub panel: PanelConfig,
    pub network: NetworkConfig,
    pub capture: CaptureConfig,
    pub lan: LanConfig,
    pub notifications: NotificationsConfig,
    pub memory: MemoryConfig,
    pub updates: UpdatesConfig,
    /// The user's model / effort choices per agent id: option id -> value, e.g.
    /// `[agent_options.claude] model = "sonnet"`, `effort = "high"`. Last: TOML tables follow values.
    pub agent_options: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// The agent saves lasting facts about the user's progress per game and gets them in every
    /// chat of that game.
    pub progress: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self { progress: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatesConfig {
    /// Look for a new version on GitHub shortly after the start and every few hours (it is only
    /// offered: installing waits for the user's click).
    pub auto_check: bool,
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self { auto_check: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationsConfig {
    /// A Windows notification when an answer to a question asked in the overlay is ready (or
    /// the agent waits for a permission) while the panel is hidden.
    pub enabled: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LanConfig {
    /// Port for phones on the local network (only while "access from the phone" is on in the
    /// tray; a free one is taken if it is busy).
    pub port: u16,
}

impl Default for LanConfig {
    fn default() -> Self {
        Self { port: 47824 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
    /// Frames sent to the agent are scaled down to this height, keeping the aspect ratio
    /// (720 = 720p; 0 = full resolution). Saved frames (thumbnails, full view) stay full size.
    pub agent_max_height: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self { agent_max_height: 720 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// "system" (Windows proxy settings, or HTTP(S)_PROXY if Windows has none), "none", or
    /// "http://host:port".
    pub proxy: String,
    /// Agents (ids: "claude", "codex") that must never connect directly: they don't start until
    /// a proxy is found. For users whose region the provider blocks. Empty by default.
    pub require_proxy_for: Vec<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self { proxy: "system".into(), require_proxy_for: vec![] }
    }
}

impl NetworkConfig {
    pub fn requires_proxy(&self, agent_id: &str) -> bool {
        self.require_proxy_for.iter().any(|a| a.trim().eq_ignore_ascii_case(agent_id))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    /// Quick question field: sending from it starts a new chat.
    pub quick: String,
    /// Chat panel with the latest chat.
    pub chat: String,
    /// Keyboard focus between the game and the open panel.
    pub focus: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    /// Logical pixels.
    pub width: f64,
    pub expanded_height: f64,
    /// Distance from the bottom edge of the screen (or of the work area if the game is windowed).
    pub bottom_margin: f64,
    /// Scale of the panel for games it hasn't been zoomed over (1.0 = 100%; 0.5..2.0).
    pub zoom: f64,
    /// The panel shows in screenshots, recordings and streams (off: Windows keeps it out).
    pub capturable: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 47823,
            agent: "claude".into(),
            language: crate::i18n::system_language(),
            hotkeys: Hotkeys::default(),
            panel: PanelConfig::default(),
            network: NetworkConfig::default(),
            capture: CaptureConfig::default(),
            lan: LanConfig::default(),
            notifications: NotificationsConfig::default(),
            memory: MemoryConfig::default(),
            updates: UpdatesConfig::default(),
            agent_options: Default::default(),
        }
    }
}

impl Default for Hotkeys {
    fn default() -> Self {
        // Ctrl+Alt: games almost never bind it (it is AltGr in many layouts); the letters are
        // under the left hand that holds Ctrl (little finger) and Alt (thumb).
        Self { quick: "ctrl+alt+q".into(), chat: "ctrl+alt+w".into(), focus: "ctrl+alt+a".into() }
    }
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self { width: 640.0, expanded_height: 540.0, bottom_margin: 64.0, zoom: 1.0, capturable: false }
    }
}

/// A language this version doesn't know (a newer version's, say) resets only the language.
fn lenient_language<'de, D: Deserializer<'de>>(d: D) -> Result<Language, D::Error> {
    let value = toml::Value::deserialize(d)?;
    Ok(Language::deserialize(value.clone()).unwrap_or_else(|_| {
        warn!("config: unknown language {value}; using the system's");
        crate::i18n::system_language()
    }))
}

const HEADER: &str = "# game-overlai settings. Delete a line to restore its default.\n\
                      # Hotkeys use the form \"ctrl+alt+q\", \"ctrl+shift+space\", ...\n\n";

/// Reads the config; creates it with defaults on first run. A broken file is left untouched
/// (so the user can fix it) and defaults are used for this run.
pub fn load(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(mut cfg) => {
                info!("config loaded from {}", path.display());
                if migrate_hotkeys(&text, &mut cfg.hotkeys) {
                    crate::hotkeys::avoid_altgr(&mut cfg.hotkeys);
                    info!("hotkeys moved to the new defaults: {:?}", cfg.hotkeys);
                    if let Err(e) = save(path, &cfg) {
                        warn!("could not save the config: {e}");
                    }
                }
                cfg
            }
            Err(e) => {
                warn!("config {} is invalid, using defaults for this run: {e}", path.display());
                Config::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut cfg = Config::default();
            crate::hotkeys::avoid_altgr(&mut cfg.hotkeys);
            if let Err(e) = save(path, &cfg) {
                warn!("could not write default config to {}: {e}", path.display());
            } else {
                info!("default config written to {}", path.display());
            }
            cfg
        }
        Err(e) => {
            warn!("could not read config {}: {e}; using defaults", path.display());
            Config::default()
        }
    }
}

/// Configs written before the focus hotkey have the old defaults Alt+Q / Alt+W (Alt is held in
/// many games: free look, item highlight): those move to the new ones; the user's own stay.
/// True: something changed (the config is saved again).
fn migrate_hotkeys(text: &str, keys: &mut Hotkeys) -> bool {
    let has_focus = toml::from_str::<toml::Table>(text)
        .ok()
        .and_then(|t| t.get("hotkeys")?.as_table()?.get("focus").cloned())
        .is_some();
    if has_focus {
        return false;
    }
    let new = Hotkeys::default();
    if keys.quick == "alt+q" {
        keys.quick = new.quick;
    }
    if keys.chat == "alt+w" {
        keys.chat = new.chat;
    }
    true
}

pub fn save(path: &Path, cfg: &Config) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, format!("{HEADER}{}", toml::to_string_pretty(cfg)?))?;
    Ok(())
}

/// "alt+q" -> "Alt+Q" for menus and hints.
pub fn pretty_hotkey(s: &str) -> String {
    s.split('+')
        .map(|part| {
            let p = part.trim();
            match p.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => "Ctrl".to_string(),
                "alt" | "option" => "Alt".to_string(),
                "shift" => "Shift".to_string(),
                "super" | "cmd" | "win" | "meta" => "Win".to_string(),
                _ => {
                    let mut c = p.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().chain(c).collect(),
                        None => String::new(),
                    }
                }
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_and_partial_files() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.port, 47823);
        let partial: Config = toml::from_str("[hotkeys]\nquick = \"ctrl+space\"\n").unwrap();
        assert_eq!(partial.hotkeys.quick, "ctrl+space");
        assert_eq!(partial.hotkeys.chat, "ctrl+alt+w");
        assert_eq!(partial.panel.width, 640.0);
        assert_eq!(partial.capture.agent_max_height, 720);
        assert_eq!(partial.lan.port, 47824);
        assert_eq!(partial.agent, "claude");
        assert!(partial.notifications.enabled);
        assert!(partial.memory.progress, "progress memory is on by default");
        assert!(!partial.network.requires_proxy("claude"), "no proxy requirement by default");
        let strict: Config = toml::from_str("[network]\nrequire_proxy_for = [\"Claude\"]\n").unwrap();
        assert!(strict.network.requires_proxy("claude") && !strict.network.requires_proxy("codex"));
        assert_eq!(strict.network.proxy, "system");
    }

    #[test]
    fn old_default_hotkeys_move_to_the_new_ones() {
        let old = "[hotkeys]
quick = \"alt+q\"
chat = \"alt+w\"
";
        let mut cfg: Config = toml::from_str(old).unwrap();
        assert!(migrate_hotkeys(old, &mut cfg.hotkeys));
        assert_eq!(cfg.hotkeys, Hotkeys::default());
        // The user's own combination stays; the focus key gets its default.
        let own = "[hotkeys]
quick = \"ctrl+shift+space\"
chat = \"alt+w\"
";
        let mut cfg: Config = toml::from_str(own).unwrap();
        assert!(migrate_hotkeys(own, &mut cfg.hotkeys));
        assert_eq!((cfg.hotkeys.quick.as_str(), cfg.hotkeys.chat.as_str()), ("ctrl+shift+space", "ctrl+alt+w"));
        assert_eq!(cfg.hotkeys.focus, "ctrl+alt+a");
        // Written by this version: never touched again, even if it is Alt+Q.
        let new = "[hotkeys]
quick = \"alt+q\"
chat = \"alt+w\"
focus = \"alt+e\"
";
        let mut cfg: Config = toml::from_str(new).unwrap();
        assert!(!migrate_hotkeys(new, &mut cfg.hotkeys));
        assert_eq!(cfg.hotkeys.quick, "alt+q");
    }

    #[test]
    fn language_is_read_leniently() {
        let ja: Config = toml::from_str("language = \"ja\"\nport = 1\n").unwrap();
        assert_eq!((ja.language, ja.port), (Language::Ja, 1));
        // A newer version's language resets only the language.
        let unknown: Config = toml::from_str("language = \"de\"\nport = 2\n").unwrap();
        assert_eq!((unknown.language, unknown.port), (crate::i18n::system_language(), 2));
        let written = toml::to_string_pretty(&Config { language: Language::Zh, ..Config::default() }).unwrap();
        assert!(written.contains("language = \"zh\""), "{written}");
    }

    #[test]
    fn pretty_hotkeys() {
        assert_eq!(pretty_hotkey("alt+q"), "Alt+Q");
        assert_eq!(pretty_hotkey("ctrl+shift+space"), "Ctrl+Shift+Space");
    }
}
