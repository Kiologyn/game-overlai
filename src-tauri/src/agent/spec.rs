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
    login_hint: "Войдите в Claude Code: откройте терминал, выполните `claude` и команду `/login`.",
};

/// Not used by default; kept working for when the agent becomes selectable (phase 9).
#[allow(dead_code)]
pub const CODEX: AgentSpec = AgentSpec {
    id: "codex",
    name: "Codex",
    npm_package: "@agentclientprotocol/codex-acp",
    npm_version: "2.8.0",
    entry: "dist/index.js",
    session_meta: codex_meta,
    system_prompt_mode: SystemPromptMode::Append,
    login_hint: "Войдите в Codex в терминале командой `codex login`, затем перезапустите приложение.",
};

/// "Lean" Claude Code session (measured in phase 0: ~3K instead of ~41K tokens per question,
/// first token ~1.3 s instead of ~2.8 s): don't load the user's global settings, plugins,
/// skills and MCP servers; keep only web tools. Our MCP server is passed separately, and its
/// screenshot tool runs without a permission prompt (it only reads the game window).
fn claude_meta() -> Map<String, Value> {
    let v = json!({
        "claudeCode": { "options": {
            "settingSources": [],
            "strictMcpConfig": true,
            "tools": ["WebSearch", "WebFetch"],
            "allowedTools": [format!("mcp__{}__{}", crate::mcp::SERVER_NAME, crate::mcp::SCREENSHOT_TOOL)]
        }}
    });
    v.as_object().cloned().unwrap_or_default()
}

/// codex-acp uses the normal signed-in Codex CLI account; MCP and images go through plain ACP.
fn codex_meta() -> Map<String, Value> {
    Map::new()
}
