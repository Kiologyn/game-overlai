//! Our MCP server, offered to every agent session: tools the agent can call on its own.
//! For now only `screenshot_game`. Served by the UI server at `/mcp` (streamable HTTP),
//! loopback only, with a per-run bearer token.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, tool, tool_router};
use tracing::{info, warn};

use crate::state::AppState;
use crate::{chat, shots};

pub const SERVER_NAME: &str = "overlai";
pub const SCREENSHOT_TOOL: &str = "screenshot_game";

#[derive(Clone)]
struct Overlai {
    state: Arc<AppState>,
}

#[tool_router(server_handler)]
impl Overlai {
    #[tool(
        name = "screenshot_game",
        description = "Capture the current frame of the game the user is playing (the game window, full resolution). \
            Use it when you need to see the screen: the user refers to something on screen and no frame is attached, \
            or you need to check the current game state. Returns one image.",
        meta = always_load_meta()
    )]
    async fn screenshot_game(&self) -> Result<CallToolResult, ErrorData> {
        let state = &self.state;
        let taken = match shots::for_agent(state).await {
            Ok(t) => t,
            Err(reason) => {
                info!("screenshot_game: no frame ({reason})");
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "Could not capture the game: {reason}"
                ))]));
            }
        };
        let source = taken.frame.source;
        let chat_id = state.chat.lock().unwrap().id.clone();
        let stored = match shots::store(state, &chat_id, taken.frame).await {
            Ok(s) => s,
            Err(reason) => return Ok(CallToolResult::error(vec![ContentBlock::text(format!("Could not encode the frame: {reason}"))])),
        };
        info!("screenshot_game: {}x{} {}", stored.shot.width, stored.shot.height, stored.mime);
        let game = state.overlay.lock().unwrap().game.clone();
        let mut note = match game {
            Some(g) => format!("Game window \"{}\" ({}), {}×{}", g.title, g.exe, stored.shot.width, stored.shot.height),
            None => format!("Game window, {}×{}", stored.shot.width, stored.shot.height),
        };
        if source == crate::capture::Source::Monitor {
            note.push_str(", whole monitor (the window itself could not be captured)");
        }
        match &taken.fallback {
            None => note.push_str(", captured just now."),
            Some((reason, age)) => note.push_str(&format!(
                ", captured {} s ago when the user opened the overlay; a fresh frame is not available now ({reason}).",
                age.as_secs()
            )),
        }
        chat::attach_tool_shot(state, stored.shot);
        Ok(CallToolResult::success(vec![ContentBlock::image(stored.base64, stored.mime), ContentBlock::text(note)]))
    }
}

/// Claude Code defers MCP tools behind its ToolSearch tool (an extra round trip, ~1 s);
/// `anthropic/alwaysLoad` keeps ours in the prompt. Other agents ignore the key.
fn always_load_meta() -> rmcp::model::MetaObject {
    let mut m = serde_json::Map::new();
    m.insert("anthropic/alwaysLoad".into(), serde_json::Value::Bool(true));
    rmcp::model::MetaObject(m)
}

/// `/mcp` routes; requests without the token get 401.
pub fn router(state: Arc<AppState>) -> Router {
    let token = Arc::new(format!("Bearer {}", state.mcp.token));
    let server = Overlai { state };
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    Router::new().nest_service("/mcp", service).layer(middleware::from_fn_with_state(token, require_token))
}

async fn require_token(State(expected): State<Arc<String>>, req: Request, next: Next) -> Response {
    let ok = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) == Some(expected.as_str());
    if ok {
        next.run(req).await
    } else {
        warn!("MCP request without a valid token rejected");
        StatusCode::UNAUTHORIZED.into_response()
    }
}
