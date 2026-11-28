//! WebSocket protocol between the core and the UI (overlay panel and browser tabs).
//!
//! TypeScript types are generated from these definitions: `cargo test` writes them to
//! `web/src/api/generated/` (see `.cargo/config.toml`). Don't edit the generated files.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum PanelMode {
    /// Only the input field.
    Compact,
    /// Chat history above the input field.
    Expanded,
}

/// Zoom the overlay panel one step (Ctrl + wheel, Ctrl +/-) or back to the default (Ctrl+0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ZoomStep {
    In,
    Out,
    Reset,
}

/// Edge or corner of the overlay panel the user drags to resize it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ResizeEdge {
    N,
    S,
    E,
    W,
    Ne,
    Nw,
    Se,
    Sw,
}

/// Interface language. A new language: a variant here, a dictionary web/src/locales/<code>.json,
/// a row in i18n.rs (Rust) and in LANGUAGES in web/src/i18n.ts (interface).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
#[repr(u8)]
pub enum Language {
    En,
    Ru,
    Es,
    Zh,
    Ja,
}

impl Language {
    pub const ALL: [Language; 5] = [Language::En, Language::Ru, Language::Es, Language::Zh, Language::Ja];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ClientKind {
    Overlay,
    Browser,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GameInfo {
    /// Window title, e.g. "Counter-Strike 2".
    pub title: String,
    /// Executable name, e.g. "cs2.exe".
    pub exe: String,
}

/// What the user told the agent about a game; one per executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GameProfile {
    pub exe: String,
    /// Latest window title.
    pub title: String,
    /// The user's name for the game (window titles can be generic, e.g. "UnityWndClass").
    pub name: Option<String>,
    /// Told to the agent with every chat of this game (spoilers, character, language...).
    pub instructions: String,
    /// Sites the agent should search first, one per line.
    pub sources: String,
    /// What the agent remembers about the user's progress (oldest first).
    pub progress: Vec<ProgressNote>,
    /// Hotkeys of its own while the game is in the foreground.
    #[serde(default)]
    pub hotkeys: GameHotkeys,
}

/// A game's own hotkeys (they apply while it is in the foreground): "" = the one from the
/// settings, "off" = none in this game, else a combination like "ctrl+shift+q".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct GameHotkeys {
    pub quick: String,
    pub chat: String,
    pub focus: String,
}

/// A lasting fact about the user's progress in a game (chapter, build, finished quests...),
/// saved by the agent with its MCP tools or by the user on the game's page. Every chat of the
/// game gets them in its system prompt while progress memory is on (settings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProgressNote {
    pub id: String,
    pub text: String,
    /// Unix milliseconds.
    #[ts(type = "number")]
    pub updated_at: i64,
}

// ------------------------------------------------------------------ widgets

/// A card pinned over a game's window (each game has its own). The agent makes and changes it
/// with MCP tools; the user moves it, ticks its checklist, makes it see-through or folds it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Widget {
    pub id: String,
    /// The game (process name) it belongs to.
    pub exe: String,
    pub title: String,
    /// Markdown, short.
    pub body: String,
    /// A checklist the user ticks (its state is the widget's progress).
    pub items: Vec<WidgetItem>,
    /// Countdown to this moment (unix ms).
    #[ts(type = "number | null")]
    pub timer_ends_at: Option<i64>,
    /// JSON the agent keeps for itself (counts, stages...): not shown, returned by widget_list.
    pub data: String,
    /// Top-left corner, as a fraction (0..1) of the game window.
    pub x: f64,
    pub y: f64,
    /// Logical px.
    pub width: f64,
    /// 50% opacity (the button in the corner).
    pub translucent: bool,
    /// Only the title bar.
    pub collapsed: bool,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct WidgetItem {
    pub id: String,
    pub text: String,
    pub done: bool,
}

/// Where a widget is on the widget layer (CSS px of the layer), for click-through outside widgets.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WidgetRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

// ------------------------------------------------------------------ chat

/// A game frame kept by the core; the image is at `/api/shots/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Shot {
    pub id: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum MessageStatus {
    /// The assistant is still answering.
    Streaming,
    Done,
    /// Stopped by the user.
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ToolStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolInfo {
    pub id: String,
    /// Human-readable, e.g. "Web search: elden ring margit weakness".
    pub title: String,
    /// search | fetch | read | think | other ...
    pub kind: String,
    pub status: ToolStatus,
    /// The game frame our `screenshot_game` tool returned to the agent.
    pub shot: Option<Shot>,
}

/// A piece of an assistant message, in arrival order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "lowercase")]
#[ts(export)]
pub enum Part {
    /// Markdown answer text.
    Text { text: String },
    /// The model's reasoning (shown collapsed).
    Thought { text: String },
    Tool { tool: ToolInfo },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AppendKind {
    Text,
    Thought,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatMessage {
    pub id: String,
    pub role: Role,
    pub parts: Vec<Part>,
    /// Game frame attached to the user's message (📷).
    pub screenshot: Option<Shot>,
    pub status: MessageStatus,
    pub error: Option<String>,
}

impl ChatMessage {
    /// Appends streamed text: extends the last part if it has the same kind, otherwise starts
    /// a new part. The UI applies exactly the same rule to `chat.append`.
    pub fn append(&mut self, kind: AppendKind, text: &str) {
        match (self.parts.last_mut(), kind) {
            (Some(Part::Text { text: t }), AppendKind::Text) | (Some(Part::Thought { text: t }), AppendKind::Thought) => {
                t.push_str(text)
            }
            (_, AppendKind::Text) => self.parts.push(Part::Text { text: text.to_string() }),
            (_, AppendKind::Thought) => self.parts.push(Part::Thought { text: text.to_string() }),
        }
    }

    /// Inserts or replaces a tool part by id (an attached frame is kept) and returns the
    /// merged tool, which is what `chat.tool` sends.
    pub fn upsert_tool(&mut self, mut tool: ToolInfo) -> ToolInfo {
        match self.parts.iter_mut().find(|p| matches!(p, Part::Tool { tool: t } if t.id == tool.id)) {
            Some(Part::Tool { tool: old }) => {
                if tool.shot.is_none() {
                    tool.shot = old.shot.take();
                }
                *old = tool.clone();
            }
            _ => self.parts.push(Part::Tool { tool: tool.clone() }),
        }
        tool
    }
}

/// A chat in the history list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatSummary {
    pub id: String,
    /// Title given by the agent, or the start of the first question.
    pub title: String,
    pub game: Option<GameInfo>,
    /// Unix milliseconds of the last question.
    #[ts(type = "number")]
    pub updated_at: i64,
}

// ------------------------------------------------------------------ agent

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AgentState {
    /// Installing the agent adapter (first run).
    Installing,
    Starting,
    Ready,
    /// Not usable; `detail` says why and what to do.
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentStatus {
    pub agent: String,
    pub state: AgentState,
    pub detail: Option<String>,
}

/// Subscription usage reported by the agent, 0.0..=1.0.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Usage {
    pub five_hour: Option<f64>,
    pub seven_day: Option<f64>,
    /// Unix seconds.
    #[ts(type = "number | null")]
    pub five_hour_resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    /// allow_once | allow_always | reject_once | reject_always
    pub kind: String,
}

/// The agent asks before doing something (e.g. fetching a page).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PermissionPrompt {
    pub id: String,
    pub title: String,
    pub options: Vec<PermissionOption>,
}

// ------------------------------------------------------------------ phone (LAN)

/// One address of this PC on the local network, with the sign-in link for a phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LanAddress {
    /// Network adapter, e.g. "Wi-Fi".
    pub adapter: String,
    pub ip: String,
    /// A private address of a real adapter; `false` = virtual (WSL, VPN...) or public.
    pub recommended: bool,
    /// Link with the access token (opens the chat and signs the phone in).
    pub url: String,
    /// QR code of `url` (SVG markup made by the core).
    pub qr_svg: String,
}

/// Access to the chat from other devices on the local network (sent to this PC's UI only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LanStatus {
    /// Turned on by the user (stays on after a restart).
    pub enabled: bool,
    /// Port the LAN server listens on; `null` while it is off or failed to start.
    pub port: Option<u16>,
    /// Best address first.
    pub addresses: Vec<LanAddress>,
    /// Adapter the QR code points at (the user's choice); `null` = automatic (the first address).
    pub chosen_adapter: Option<String>,
    /// Devices connected right now.
    pub clients: u32,
    pub error: Option<String>,
}

// ------------------------------------------------------------------ agent options

/// What an option of the agent's session sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AgentOptionKind {
    Model,
    /// Reasoning effort ("thought level" in ACP).
    Effort,
}

/// A choice the agent offers for its sessions (model, reasoning effort), as it reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentOption {
    /// The agent's id for the option, e.g. "model", "effort".
    pub id: String,
    pub kind: AgentOptionKind,
    pub name: String,
    pub current: String,
    pub values: Vec<AgentOptionValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentOptionValue {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
}

// ------------------------------------------------------------------ settings

/// What the settings page edits (saved to config.toml).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SettingsValues {
    /// e.g. "ctrl+alt+q"
    pub hotkey_quick: String,
    pub hotkey_chat: String,
    /// Focus between the game and the open panel.
    pub hotkey_focus: String,
    /// "claude" | "codex"
    pub agent: String,
    /// "system" | "none" | "http://host:port"
    pub proxy: String,
    /// Agent ids that must not connect without a proxy.
    pub require_proxy_for: Vec<String>,
    /// Default panel size (logical px, at 100%) for games the user hasn't placed it over yet.
    pub panel_width: f64,
    pub panel_height: f64,
    /// Default panel zoom (1.0 = 100%) for games it hasn't been zoomed over.
    pub panel_zoom: f64,
    /// The panel shows in screenshots, recordings and streams.
    pub panel_capturable: bool,
    /// Height of frames sent to the agent (0 = full).
    pub agent_max_height: u32,
    pub lan_port: u16,
    pub notifications: bool,
    /// The agent remembers the user's progress per game (`[memory] progress`).
    pub progress_memory: bool,
    /// Look for new versions at the start and every few hours (`[updates] auto_check`).
    pub update_check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentChoice {
    pub id: String,
    pub name: String,
    /// How to sign in to it (shown under the choice).
    pub login_hint: String,
}

/// The settings page's state (this PC's UI only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Settings {
    pub values: SettingsValues,
    pub agents: Vec<AgentChoice>,
    /// Saved changes that take effect after a restart (agent, proxy, ports).
    pub restart_needed: bool,
    /// Hotkeys another program holds.
    pub hotkeys_taken: Vec<String>,
    /// Starts with Windows (this exe).
    pub autostart: bool,
    /// Autostart points at another copy of the app (e.g. an older build): its path.
    pub autostart_other: Option<String>,
    pub version: String,
    pub config_file: String,
    pub data_dir: String,
    /// The last save was refused: why.
    pub error: Option<String>,
}

// ------------------------------------------------------------------ updates

/// Where looking for a new version stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateState {
    /// Not checked yet.
    Idle,
    Checking,
    UpToDate,
    /// A newer version is out (`UpdateStatus::version`).
    Available,
    /// Downloading it; `percent` once the size is known.
    Downloading { percent: Option<u8> },
    /// The installer starts: the app closes and comes back updated.
    Installing,
    /// The check or the update failed (the version found, if any, can be tried again).
    Error { message: String },
}

/// New versions (this PC's UI only: only it can install).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateStatus {
    pub state: UpdateState,
    /// This copy's version.
    pub current: String,
    /// The newer version found.
    pub version: Option<String>,
    /// Its release page ("What's new").
    pub notes_url: Option<String>,
    /// A copy without the installer (portable exe, dev build): "Update" opens the release page.
    pub portable: bool,
    /// "Later" in the chat header: its button hides until the next start or a newer version.
    pub dismissed: bool,
}

// ------------------------------------------------------------------ messages

/// UI -> core.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ClientMsg {
    /// Hide the overlay panel and give focus back to the game (overlay only).
    #[serde(rename = "ui.hide")]
    UiHide,
    /// The panel switched between compact and expanded: resize the window (overlay only).
    #[serde(rename = "ui.setMode")]
    UiSetMode { mode: PanelMode },
    /// The mouse went down on the panel's drag area: the core starts a native move (overlay only).
    #[serde(rename = "ui.dragStart")]
    UiDragStart,
    /// The mouse went down on a resize handle (overlay only).
    #[serde(rename = "ui.resizeStart")]
    UiResizeStart { edge: ResizeEdge },
    /// Scale the panel (content and window together) one step (overlay only).
    #[serde(rename = "ui.zoom")]
    UiZoom { step: ZoomStep },
    #[serde(rename = "chat.send", rename_all = "camelCase")]
    ChatSend {
        text: String,
        /// Start a new chat with this message (sent from the quick field).
        new_chat: bool,
    },
    /// Pick a model / reasoning effort: applies to the current chat now and to later chats.
    #[serde(rename = "agent.setOption")]
    AgentSetOption { id: String, value: String },
    /// Stop the answer being generated.
    #[serde(rename = "chat.cancel")]
    ChatCancel,
    /// Start an empty new chat.
    #[serde(rename = "chat.new")]
    ChatNew,
    /// Make a chat from the history the current one.
    #[serde(rename = "chat.open")]
    ChatOpen { id: String },
    /// Delete a chat with its frames (the current one is replaced by an empty chat).
    #[serde(rename = "chat.delete")]
    ChatDelete { id: String },
    /// Save a game's profile (applies to its chats from their next question).
    #[serde(rename = "game.update")]
    GameUpdate {
        exe: String,
        name: Option<String>,
        instructions: String,
        sources: String,
    },
    /// A game's own hotkeys (this PC's UI only: recording a combination needs its keyboard).
    #[serde(rename = "game.hotkeys")]
    GameSetHotkeys { exe: String, hotkeys: GameHotkeys },
    /// `optionId: null` = dismiss (the agent treats it as cancelled).
    #[serde(rename = "permission.respond", rename_all = "camelCase")]
    PermissionRespond { id: String, option_id: Option<String> },
    /// Turn chat access from the local network on/off (this PC's UI only).
    #[serde(rename = "lan.set")]
    LanSet { enabled: bool },
    /// New access token: devices signed in so far are logged out (this PC's UI only).
    #[serde(rename = "lan.resetToken")]
    LanResetToken,
    /// Which adapter's address the QR code shows; `null` = automatic (this PC's UI only).
    #[serde(rename = "lan.chooseAdapter")]
    LanChooseAdapter { adapter: Option<String> },
    /// Save the settings (this PC's UI only).
    #[serde(rename = "settings.save")]
    SettingsSave { values: SettingsValues },
    #[serde(rename = "settings.autostart")]
    SettingsAutostart { enabled: bool },
    /// Applies at once (no Save): every UI, the tray and notifications switch.
    #[serde(rename = "settings.language")]
    SettingsLanguage { language: Language },
    /// The settings page records a hotkey: global hotkeys are off meanwhile, so pressing the
    /// current one doesn't open the panel.
    #[serde(rename = "settings.capturing")]
    SettingsCapturing { active: bool },
    /// Forget where the panel was placed over each game.
    #[serde(rename = "settings.resetPanelPlaces")]
    SettingsResetPanelPlaces,
    /// Restart the app (applies agent/proxy/port changes).
    #[serde(rename = "app.restart")]
    AppRestart,
    /// Look for a new version now (this PC's UI only, as the three below).
    #[serde(rename = "update.check")]
    UpdateCheck,
    /// Download the version found and install it (the app restarts).
    #[serde(rename = "update.install")]
    UpdateInstall,
    /// "Later": hide the chat header's update button until the next start.
    #[serde(rename = "update.dismiss")]
    UpdateDismiss,
    /// Open an http(s) link in the default browser (links must not navigate the overlay).
    #[serde(rename = "ui.openUrl")]
    UiOpenUrl { url: String },
    /// The user ticked or unticked a widget's checklist item.
    #[serde(rename = "widget.toggleItem", rename_all = "camelCase")]
    WidgetToggleItem { id: String, item_id: String },
    /// The user moved, resized, folded or made a widget see-through (only the given fields change).
    #[serde(rename = "widget.set")]
    WidgetSet {
        id: String,
        #[serde(default)]
        #[ts(optional)]
        x: Option<f64>,
        #[serde(default)]
        #[ts(optional)]
        y: Option<f64>,
        #[serde(default)]
        #[ts(optional)]
        width: Option<f64>,
        #[serde(default)]
        #[ts(optional)]
        translucent: Option<bool>,
        #[serde(default)]
        #[ts(optional)]
        collapsed: Option<bool>,
    },
    #[serde(rename = "widget.delete")]
    WidgetDelete { id: String },
    /// The widget layer page: where its widgets are, so clicks elsewhere go to the game.
    #[serde(rename = "widgets.rects")]
    WidgetRects { rects: Vec<WidgetRect> },
    /// The game page: the user edits what the agent remembers about the game.
    #[serde(rename = "progress.add")]
    ProgressAdd { exe: String, text: String },
    #[serde(rename = "progress.edit")]
    ProgressEdit { id: String, text: String },
    #[serde(rename = "progress.delete")]
    ProgressDelete { id: String },
}

/// core -> UI.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ServerMsg {
    #[serde(rename = "hello")]
    Hello { version: String, client: ClientKind },
    /// The interface language: after hello, and to every UI when the user changes it.
    #[serde(rename = "language")]
    Language { language: Language },
    /// Full state of the current chat (on connect, when a new chat starts or another one opens).
    /// `chatId` of an empty new chat is not in the history until its first message.
    #[serde(rename = "chat.snapshot", rename_all = "camelCase")]
    ChatSnapshot { chat_id: String, messages: Vec<ChatMessage>, game: Option<GameInfo>, title: Option<String> },
    /// A new message (user's, or an empty streaming assistant message), or a new version of
    /// an existing one with the same id (e.g. its game frame is ready).
    #[serde(rename = "chat.message")]
    ChatMessage { message: ChatMessage },
    /// Streamed text for a message (see `ChatMessage::append`).
    #[serde(rename = "chat.append", rename_all = "camelCase")]
    ChatAppend { message_id: String, kind: AppendKind, text: String },
    /// A tool call started or changed: replaces the tool part with the same id.
    #[serde(rename = "chat.tool", rename_all = "camelCase")]
    ChatTool { message_id: String, tool: ToolInfo },
    #[serde(rename = "chat.status", rename_all = "camelCase")]
    ChatStatus { message_id: String, status: MessageStatus, error: Option<String> },
    #[serde(rename = "chat.title")]
    ChatTitle { title: String },
    /// The whole history list, newest first (on connect).
    #[serde(rename = "chats.snapshot")]
    ChatsSnapshot { chats: Vec<ChatSummary> },
    /// A chat was added to the history or changed (title, last question): replaces the one
    /// with the same id; the list stays sorted by `updatedAt`, newest first.
    #[serde(rename = "chats.upsert")]
    ChatsUpsert { chat: ChatSummary },
    #[serde(rename = "chats.remove")]
    ChatsRemove { id: String },
    /// All game profiles, most recently played first (on connect).
    #[serde(rename = "games.snapshot")]
    GamesSnapshot { games: Vec<GameProfile> },
    /// A game was added or its profile changed: replaces the one with the same exe.
    #[serde(rename = "games.upsert")]
    GamesUpsert { game: GameProfile },
    /// Pending permission prompts for the current chat (replaces the previous list).
    #[serde(rename = "permissions")]
    Permissions { prompts: Vec<PermissionPrompt> },
    #[serde(rename = "agent.status")]
    AgentStatus { status: AgentStatus },
    #[serde(rename = "agent.usage")]
    AgentUsage { usage: Usage },
    /// The agent's model / effort choices with their current values (empty: it offers none).
    #[serde(rename = "agent.options")]
    AgentOptions { options: Vec<AgentOption> },
    /// Every game's widgets: on connect and after any change.
    #[serde(rename = "widgets")]
    Widgets { widgets: Vec<Widget> },
    /// Which game's widgets the widget layer shows now (None: hidden).
    #[serde(rename = "widgets.shown")]
    WidgetsShown { exe: Option<String> },
    /// The overlay panel was shown by a hotkey/tray (overlay only).
    #[serde(rename = "overlay.show")]
    OverlayShow { mode: PanelMode, game: Option<GameInfo> },
    #[serde(rename = "overlay.hidden")]
    OverlayHidden,
    /// The panel's zoom changed (or hit its limit): the panel shows it briefly (overlay only).
    #[serde(rename = "overlay.zoom")]
    OverlayZoom { zoom: f64 },
    /// Phone access over the local network (this PC's UI only: it carries the token).
    #[serde(rename = "lan.status")]
    LanStatus { status: LanStatus },
    /// Settings page state (this PC's UI only).
    #[serde(rename = "settings")]
    Settings { settings: Box<Settings> },
    /// New versions: on connect and on every change (this PC's UI only).
    #[serde(rename = "update")]
    Update { update: UpdateStatus },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format() {
        let m: ClientMsg =
            serde_json::from_str(r#"{"type":"chat.send","text":"hi","newChat":true}"#).unwrap();
        assert!(matches!(m, ClientMsg::ChatSend { new_chat: true, .. }));
        let m: ClientMsg = serde_json::from_str(r#"{"type":"ui.setMode","mode":"expanded"}"#).unwrap();
        assert!(matches!(m, ClientMsg::UiSetMode { mode: PanelMode::Expanded }));
        let m: ClientMsg = serde_json::from_str(r#"{"type":"ui.resizeStart","edge":"nw"}"#).unwrap();
        assert!(matches!(m, ClientMsg::UiResizeStart { edge: ResizeEdge::Nw }));
        let m: ClientMsg = serde_json::from_str(r#"{"type":"lan.set","enabled":true}"#).unwrap();
        assert!(matches!(m, ClientMsg::LanSet { enabled: true }));
        let m: ClientMsg = serde_json::from_str(r#"{"type":"chat.open","id":"c1"}"#).unwrap();
        assert!(matches!(m, ClientMsg::ChatOpen { id } if id == "c1"));
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"game.update","exe":"cs2.exe","name":null,"instructions":"x","sources":""}"#,
        )
        .unwrap();
        assert!(matches!(m, ClientMsg::GameUpdate { name: None, .. }));
        let m: ClientMsg = serde_json::from_str(r#"{"type":"permission.respond","id":"p1","optionId":null}"#).unwrap();
        assert!(matches!(m, ClientMsg::PermissionRespond { option_id: None, .. }));
        let s = serde_json::to_string(&ServerMsg::OverlayShow { mode: PanelMode::Compact, game: None }).unwrap();
        assert_eq!(s, r#"{"type":"overlay.show","mode":"compact","game":null}"#);
        let s = serde_json::to_string(&ServerMsg::ChatSnapshot {
            chat_id: "c1".into(),
            messages: vec![],
            game: None,
            title: None,
        })
        .unwrap();
        assert_eq!(s, r#"{"type":"chat.snapshot","chatId":"c1","messages":[],"game":null,"title":null}"#);
        let s = serde_json::to_string(&Part::Text { text: "x".into() }).unwrap();
        assert_eq!(s, r#"{"type":"text","text":"x"}"#);
    }

    #[test]
    fn append_and_upsert() {
        let mut m = ChatMessage {
            id: "a".into(),
            role: Role::Assistant,
            parts: vec![],
            screenshot: None,
            status: MessageStatus::Streaming,
            error: None,
        };
        m.append(AppendKind::Thought, "hmm ");
        m.append(AppendKind::Thought, "ok");
        m.append(AppendKind::Text, "Hel");
        m.append(AppendKind::Text, "lo");
        let tool =
            |status| ToolInfo { id: "t1".into(), title: "Search".into(), kind: "search".into(), status, shot: None };
        let shot = Shot { id: "s1".into(), width: 1920, height: 1080 };
        m.upsert_tool(tool(ToolStatus::Running));
        m.upsert_tool(ToolInfo { shot: Some(shot.clone()), ..tool(ToolStatus::Running) });
        let merged = m.upsert_tool(tool(ToolStatus::Done)); // a later update without the frame keeps it
        m.append(AppendKind::Text, "!");
        let done = ToolInfo { shot: Some(shot), ..tool(ToolStatus::Done) };
        assert_eq!(merged, done);
        assert_eq!(m.parts.len(), 4);
        assert_eq!(m.parts[0], Part::Thought { text: "hmm ok".into() });
        assert_eq!(m.parts[1], Part::Text { text: "Hello".into() });
        assert_eq!(m.parts[2], Part::Tool { tool: done });
        assert_eq!(m.parts[3], Part::Text { text: "!".into() });
    }
}
