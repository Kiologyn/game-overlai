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
    #[serde(rename = "chat.send", rename_all = "camelCase")]
    ChatSend {
        text: String,
        screenshot: bool,
        /// Start a new chat with this message (sent from the quick field).
        new_chat: bool,
    },
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
    /// `optionId: null` = dismiss (the agent treats it as cancelled).
    #[serde(rename = "permission.respond", rename_all = "camelCase")]
    PermissionRespond { id: String, option_id: Option<String> },
    /// Open an http(s) link in the default browser (links must not navigate the overlay).
    #[serde(rename = "ui.openUrl")]
    UiOpenUrl { url: String },
}

/// core -> UI.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ServerMsg {
    #[serde(rename = "hello")]
    Hello { version: String, client: ClientKind },
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
    /// Pending permission prompts for the current chat (replaces the previous list).
    #[serde(rename = "permissions")]
    Permissions { prompts: Vec<PermissionPrompt> },
    #[serde(rename = "agent.status")]
    AgentStatus { status: AgentStatus },
    #[serde(rename = "agent.usage")]
    AgentUsage { usage: Usage },
    /// The overlay panel was shown by a hotkey/tray (overlay only).
    #[serde(rename = "overlay.show")]
    OverlayShow { mode: PanelMode, game: Option<GameInfo> },
    #[serde(rename = "overlay.hidden")]
    OverlayHidden,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format() {
        let m: ClientMsg =
            serde_json::from_str(r#"{"type":"chat.send","text":"hi","screenshot":true,"newChat":true}"#).unwrap();
        assert!(matches!(m, ClientMsg::ChatSend { new_chat: true, screenshot: true, .. }));
        let m: ClientMsg = serde_json::from_str(r#"{"type":"ui.setMode","mode":"expanded"}"#).unwrap();
        assert!(matches!(m, ClientMsg::UiSetMode { mode: PanelMode::Expanded }));
        let m: ClientMsg = serde_json::from_str(r#"{"type":"ui.resizeStart","edge":"nw"}"#).unwrap();
        assert!(matches!(m, ClientMsg::UiResizeStart { edge: ResizeEdge::Nw }));
        let m: ClientMsg = serde_json::from_str(r#"{"type":"chat.open","id":"c1"}"#).unwrap();
        assert!(matches!(m, ClientMsg::ChatOpen { id } if id == "c1"));
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
