//! Supported agents. Each one is an ACP adapter published on npm and started with Node.
//! Adding Gemini later = another `AgentSpec`.

use agent_client_protocol::v1::schema::SystemPromptMode;
use serde_json::{Map, Value, json};

pub struct AgentSpec {
    pub id: &'static str,
    /// Shown in the UI.
    pub name: &'static str,
    pub npm_package: &'static str,
    /// Pinned: adapters change quickly.
    pub npm_version: &'static str,
    /// Script inside the package that speaks ACP over stdio.
    pub entry: &'static str,
    /// Agent-specific `_meta` for session/new and session/resume|load.
    pub session_meta: fn() -> Map<String, Value>,
    /// How our system prompt is applied: Claude replaces its coding prompt with ours
    /// (`Override`); codex-acp accepts only `Append`.
    pub system_prompt_mode: SystemPromptMode,
    /// Hint shown when the agent reports it is not signed in.
    /// How to sign in: the code of a string in web/src/locales (see i18n.rs).
    pub login_hint: &'static str,
}

pub const CLAUDE: AgentSpec = AgentSpec {
    id: "claude",
    name: "Claude",
    npm_package: "@agentclientprotocol/claude-agent-acp",
    npm_version: "0.109.0",
    entry: "dist/index.js",
    session_meta: claude_meta,
    system_prompt_mode: SystemPromptMode::Override,
    login_hint: "login_hint_claude",
};

pub const CODEX: AgentSpec = AgentSpec {
    id: "codex",
    name: "Codex",
    npm_package: "@agentclientprotocol/codex-acp",
    npm_version: "2.8.0",
    entry: "dist/index.js",
    session_meta: codex_meta,
    system_prompt_mode: SystemPromptMode::Append,
    login_hint: "login_hint_codex",
};

/// Agents the user can choose in the settings.
pub const ALL: [&AgentSpec; 2] = [&CLAUDE, &CODEX];

/// The spec for a configured agent id (Claude if unknown).
pub fn by_id(id: &str) -> &'static AgentSpec {
    ALL.into_iter().find(|s| s.id.eq_ignore_ascii_case(id.trim())).unwrap_or(&CLAUDE)
}

/// "Lean" Claude Code session (measured in phase 0: ~3K instead of ~41K tokens per question,
/// first token ~1.3 s instead of ~2.8 s): don't load the user's global settings, plugins,
/// skills and MCP servers; keep only web tools. Our MCP server is passed separately, and its
/// tools run without a permission prompt (they read the game window, keep progress notes and
/// widgets of the app; nothing outside it).
fn claude_meta() -> Map<String, Value> {
    let v = json!({
        "claudeCode": { "options": {
            "settingSources": [],
            "strictMcpConfig": true,
            "tools": ["WebSearch", "WebFetch"],
            "allowedTools": [format!("mcp__{}", crate::mcp::SERVER_NAME)]
        }}
    });
    v.as_object().cloned().unwrap_or_default()
}

/// codex-acp uses the normal signed-in Codex CLI account; MCP and images go through plain ACP.
fn codex_meta() -> Map<String, Value> {
    Map::new()
}
