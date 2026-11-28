//! Phase 0 / spike 1: can we drive Claude Code through ACP from Rust?
//!
//! Checks:
//!   A. initialize -> agent capabilities (image prompts, HTTP MCP, load_session)
//!   B. session/new with our own HTTP MCP server attached
//!   C. prompt with an inline image (push path)            -> should say "red / blue"
//!   D. prompt asking to call `screenshot_game` (pull path) -> should say "green / yellow"
//!   E. new connection + session/load, ask about the first image (history restore)
//!
//! Run from spikes/acp-probe:  cargo run
//! Set ACP_TRACE=1 to dump raw JSON-RPC traffic.

use std::io::{Cursor, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    ContentBlock, HttpHeader, ImageContent, InitializeRequest, LoadSessionRequest, McpServer,
    McpServerHttp, NewSessionRequest, PermissionOptionKind, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, SystemPrompt, SystemPromptMode,
    TextContent,
};
use agent_client_protocol::{AcpAgent, Agent, LineDirection};
use anyhow::Context as _;
use base64::Engine as _;
use rmcp::model::{CallToolResult, ContentBlock as McpContent};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, tool, tool_router};

// ---------------------------------------------------------------- timing / output state

static T0: OnceLock<Instant> = OnceLock::new();
static FIRST_CHUNK: AtomicBool = AtomicBool::new(false);
static PROMPT_START: Mutex<Option<Instant>> = Mutex::new(None);
/// While replaying history after session/load, don't print chunks, just count them.
static REPLAYING: AtomicBool = AtomicBool::new(false);
static REPLAYED: AtomicUsize = AtomicUsize::new(0);

fn ms() -> u128 {
    T0.get().unwrap().elapsed().as_millis()
}

fn log(msg: impl AsRef<str>) {
    eprintln!("\x1b[90m[{:>6} ms]\x1b[0m {}", ms(), msg.as_ref());
}

fn short_json(v: &impl serde::Serialize) -> String {
    let s = serde_json::to_string(v).unwrap_or_else(|e| format!("<ser err {e}>"));
    if s.chars().count() > 400 {
        format!("{}…", s.chars().take(400).collect::<String>())
    } else {
        s
    }
}

// ---------------------------------------------------------------- test images

fn png_b64(img: image::RgbImage) -> String {
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .expect("png encode");
    base64::engine::general_purpose::STANDARD.encode(buf)
}

/// Prompt image: left half red, right half blue.
fn prompt_image() -> String {
    png_b64(image::RgbImage::from_fn(800, 400, |x, _| {
        if x < 400 { image::Rgb([220, 30, 30]) } else { image::Rgb([30, 60, 220]) }
    }))
}

/// "Screenshot" served by the MCP tool: green background with a yellow square in the middle.
fn tool_image() -> String {
    png_b64(image::RgbImage::from_fn(640, 360, |x, y| {
        if (220..420).contains(&x) && (80..280).contains(&y) {
            image::Rgb([240, 220, 20])
        } else {
            image::Rgb([30, 160, 60])
        }
    }))
}

// ---------------------------------------------------------------- MCP server

#[derive(Clone)]
struct Overlai {
    calls: Arc<AtomicUsize>,
    image_b64: Arc<String>,
}

#[tool_router(server_handler)]
impl Overlai {
    #[tool(
        name = "screenshot_game",
        description = "Take a screenshot of the game the user is currently playing. Returns the current frame of the game window as an image. Use it whenever you need to see what is on the user's screen.",
        meta = always_load_meta()
    )]
    async fn screenshot_game(&self) -> Result<CallToolResult, ErrorData> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        log(format!("\x1b[33m[mcp] screenshot_game called (#{n})\x1b[0m"));
        Ok(CallToolResult::success(vec![
            McpContent::image(self.image_b64.as_str(), "image/png"),
            McpContent::text("game=TestGame window=640x360 capture=window"),
        ]))
    }
}

/// Claude Code defers MCP tools behind ToolSearch by default (one extra round trip).
/// `anthropic/alwaysLoad` on the tool keeps it in the prompt; other agents ignore it.
fn always_load_meta() -> rmcp::model::MetaObject {
    let mut m = serde_json::Map::new();
    m.insert("anthropic/alwaysLoad".into(), serde_json::Value::Bool(true));
    rmcp::model::MetaObject(m)
}

fn lean_mode() -> bool {
    std::env::args().any(|a| a == "--lean")
}

const LEAN_SYSTEM_PROMPT: &str = "You are an in-game assistant inside an overlay. The user is playing a video game right now \
and glances at your answers between actions, so answer briefly and concretely, in the user's language. \
When you need to see what is on the screen, call the `screenshot_game` tool. \
Use web search for game facts you are not sure about.";

/// Claude-specific session options (claude-agent-acp `_meta.claudeCode.options`):
/// don't load the user's global settings/plugins/skills/MCP servers, and keep only the tools we want.
fn lean_session_meta() -> serde_json::Map<String, serde_json::Value> {
    let v = serde_json::json!({
        "claudeCode": { "options": {
            "settingSources": [],
            "strictMcpConfig": true,
            "tools": ["WebSearch", "WebFetch"]
        }}
    });
    v.as_object().unwrap().clone()
}

async fn start_mcp(token: String, calls: Arc<AtomicUsize>) -> anyhow::Result<String> {
    let server = Overlai { calls, image_b64: Arc::new(tool_image()) };
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let expected = format!("Bearer {token}");
    let app = axum::Router::new().nest_service("/mcp", service).layer(
        axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
            let ok = req
                .headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v == expected);
            async move {
                if ok {
                    next.run(req).await
                } else {
                    log("\x1b[31m[mcp] rejected request without valid token\x1b[0m");
                    axum::response::IntoResponse::into_response(axum::http::StatusCode::UNAUTHORIZED)
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://127.0.0.1:{}/mcp", listener.local_addr()?.port());
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log(format!("[mcp] server error: {e}"));
        }
    });
    Ok(url)
}

// ---------------------------------------------------------------- ACP helpers

fn spawn_agent(adapter: &Path) -> anyhow::Result<AcpAgent> {
    let trace = std::env::var("ACP_TRACE").is_ok();
    Ok(AcpAgent::from_args(["node".to_string(), adapter.display().to_string()])?.with_debug(
        move |line, dir| match dir {
            LineDirection::Stderr => log(format!("\x1b[35m[agent stderr]\x1b[0m {line}")),
            LineDirection::Stdin if trace => log(format!("\x1b[34m-> {}\x1b[0m", truncate(line))),
            LineDirection::Stdout if trace => log(format!("\x1b[36m<- {}\x1b[0m", truncate(line))),
            _ => {}
        },
    ))
}

fn truncate(s: &str) -> String {
    if s.len() > 600 { format!("{}…[{} bytes]", &s[..s.floor_char_boundary(600)], s.len()) } else { s.to_string() }
}

fn mcp_servers(url: &str, token: &str) -> Vec<McpServer> {
    vec![McpServer::Http(
        McpServerHttp::new("overlai", url)
            .headers(vec![HttpHeader::new("Authorization", format!("Bearer {token}"))]),
    )]
}

async fn on_notification(n: SessionNotification) {
    if REPLAYING.load(Ordering::SeqCst) {
        REPLAYED.fetch_add(1, Ordering::SeqCst);
        return;
    }
    match n.update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            if !FIRST_CHUNK.swap(true, Ordering::SeqCst) {
                let started = PROMPT_START.lock().unwrap().unwrap_or_else(Instant::now);
                log(format!("\x1b[32mfirst text chunk after {} ms\x1b[0m", started.elapsed().as_millis()));
            }
            match chunk.content {
                ContentBlock::Text(t) => {
                    print!("{}", t.text);
                    let _ = std::io::stdout().flush();
                }
                other => log(format!("[non-text chunk] {}", short_json(&other))),
            }
        }
        SessionUpdate::AgentThoughtChunk(_) => {
            eprint!("\x1b[90m·\x1b[0m");
        }
        SessionUpdate::ToolCall(tc) => log(format!(
            "\x1b[33m[tool_call]\x1b[0m title={:?} name={:?} kind={:?} status={:?}",
            tc.title, tc.name, tc.kind, tc.status
        )),
        SessionUpdate::ToolCallUpdate(u) => log(format!("[tool_call_update] {}", short_json(&u))),
        other => log(format!("[update] {}", short_json(&other))),
    }
}

async fn prompt(cx: &ConnectionTo<Agent>, sid: &agent_client_protocol::v1::schema::SessionId, label: &str, blocks: Vec<ContentBlock>) -> agent_client_protocol::Result<()> {
    log(format!("\x1b[1m=== {label}\x1b[0m"));
    FIRST_CHUNK.store(false, Ordering::SeqCst);
    let start = Instant::now();
    *PROMPT_START.lock().unwrap() = Some(start);
    let resp = cx.send_request(PromptRequest::new(sid.clone(), blocks)).block_task().await?;
    println!();
    log(format!("done in {} ms, stop_reason={:?}", start.elapsed().as_millis(), resp.stop_reason));
    Ok(())
}

fn text(s: &str) -> ContentBlock {
    ContentBlock::Text(TextContent::new(s))
}

// ---------------------------------------------------------------- main

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    T0.set(Instant::now()).unwrap();
    let root = std::env::current_dir()?;
    let adapter = root.join("node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js");
    anyhow::ensure!(adapter.exists(), "adapter not found at {} (run npm install)", adapter.display());

    let profile = root.join("profile");
    prepare_profile(&profile)?;

    let token = uuid::Uuid::new_v4().simple().to_string();
    let calls = Arc::new(AtomicUsize::new(0));
    let mcp_url = start_mcp(token.clone(), calls.clone()).await?;
    log(format!("MCP server at {mcp_url}"));

    let session_id = tokio::time::timeout(
        Duration::from_secs(300),
        run_first_connection(&adapter, &profile, &mcp_url, &token),
    )
    .await
    .context("first connection timed out")??;

    log(format!("screenshot_game was called {} time(s)", calls.load(Ordering::SeqCst)));

    if let Some(sid) = session_id {
        tokio::time::timeout(
            Duration::from_secs(180),
            run_load_connection(&adapter, &profile, &mcp_url, &token, sid),
        )
        .await
        .context("load connection timed out")??;
    }
    log("probe finished");
    Ok(())
}

fn prepare_profile(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let agents_md = dir.join("AGENTS.md");
    if !agents_md.exists() {
        std::fs::write(
            &agents_md,
            "# Game assistant\n\nYou are an assistant that helps the user while they play a video game.\n\
             Answer briefly, in the user's language.\n\
             When you need to see the screen, call the `screenshot_game` tool.\n",
        )?;
    }
    let claude_md = dir.join("CLAUDE.md");
    if !claude_md.exists() {
        std::fs::write(&claude_md, "@AGENTS.md\n")?;
    }
    Ok(())
}

/// Builds the client: notification printing + auto-approve permission requests (logged).
macro_rules! client {
    () => {
        agent_client_protocol::Client
            .v1()
            .name("game-overlai-probe")
            .on_receive_notification(
                async move |n: SessionNotification, _cx| {
                    on_notification(n).await;
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_request(
                async move |req: RequestPermissionRequest, responder, _cx| {
                    log(format!("\x1b[31m[permission request]\x1b[0m {}", short_json(&req.tool_call)));
                    let id = [PermissionOptionKind::AllowOnce, PermissionOptionKind::AllowAlways]
                        .iter()
                        .find_map(|k| req.options.iter().find(|o| o.kind == *k).map(|o| o.option_id.clone()));
                    responder.respond(RequestPermissionResponse::new(match id {
                        Some(id) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id)),
                        None => RequestPermissionOutcome::Cancelled,
                    }))
                },
                agent_client_protocol::on_receive_request!(),
            )
    };
}

async fn run_first_connection(
    adapter: &PathBuf,
    profile: &PathBuf,
    mcp_url: &str,
    token: &str,
) -> anyhow::Result<Option<agent_client_protocol::v1::schema::SessionId>> {
    let agent = spawn_agent(adapter)?;
    let (profile, mcp_url, token) = (profile.clone(), mcp_url.to_string(), token.to_string());
    let spawned = Instant::now();
    let sid = client!()
        .connect_with(agent, async move |cx: ConnectionTo<Agent>| {
            let init = cx.send_request(InitializeRequest::new(ProtocolVersion::V1)).block_task().await?;
            log(format!("initialize took {} ms (incl. node startup)", spawned.elapsed().as_millis()));
            println!("--- agent_info: {}", short_json(&init.agent_info));
            println!("--- agent_capabilities:\n{}", serde_json::to_string_pretty(&init.agent_capabilities)?);
            println!("--- auth_methods: {}", short_json(&init.auth_methods));
            let caps = &init.agent_capabilities;
            let can_load = caps.load_session;

            let t = Instant::now();
            let mut req = NewSessionRequest::new(profile.clone()).mcp_servers(mcp_servers(&mcp_url, &token));
            if lean_mode() {
                log("\x1b[1mLEAN mode: override system prompt, no user settings, tools=[WebSearch, WebFetch]\x1b[0m");
                req = req
                    .system_prompt(SystemPrompt::new(vec![text(LEAN_SYSTEM_PROMPT)]).mode(SystemPromptMode::Override))
                    .meta(lean_session_meta());
            }
            let new = cx.send_request(req).block_task().await?;
            log(format!("session/new took {} ms, id={}", t.elapsed().as_millis(), new.session_id));
            println!("--- session/new response: {}", short_json(&new));
            let sid = new.session_id;

            prompt(
                &cx,
                &sid,
                "C: inline image prompt",
                vec![
                    text("Какие два цвета на этой картинке и где каждый? Ответь одним коротким предложением."),
                    ContentBlock::Image(ImageContent::new(prompt_image(), "image/png")),
                ],
            )
            .await?;

            prompt(
                &cx,
                &sid,
                "D: MCP screenshot tool",
                vec![text(
                    "Сделай скриншот игры инструментом screenshot_game и опиши, что на нём, одним коротким предложением.",
                )],
            )
            .await?;

            Ok(can_load.then_some(sid))
        })
        .await?;
    Ok(sid)
}

async fn run_load_connection(
    adapter: &PathBuf,
    profile: &PathBuf,
    mcp_url: &str,
    token: &str,
    sid: agent_client_protocol::v1::schema::SessionId,
) -> anyhow::Result<()> {
    log("\x1b[1m=== E: new process + session/load\x1b[0m");
    let agent = spawn_agent(adapter)?;
    let (profile, mcp_url, token) = (profile.clone(), mcp_url.to_string(), token.to_string());
    client!()
        .connect_with(agent, async move |cx: ConnectionTo<Agent>| {
            cx.send_request(InitializeRequest::new(ProtocolVersion::V1)).block_task().await?;
            REPLAYING.store(true, Ordering::SeqCst);
            let t = Instant::now();
            let mut req = LoadSessionRequest::new(sid.clone(), profile.clone()).mcp_servers(mcp_servers(&mcp_url, &token));
            if lean_mode() {
                req = req.meta(lean_session_meta());
            }
            cx.send_request(req).block_task().await?;
            REPLAYING.store(false, Ordering::SeqCst);
            log(format!(
                "session/load took {} ms, replayed {} history updates",
                t.elapsed().as_millis(),
                REPLAYED.load(Ordering::SeqCst)
            ));
            prompt(
                &cx,
                &sid,
                "E: question about earlier context",
                vec![text("Какого цвета была левая половина самой первой картинки в этом чате? Одно слово.")],
            )
            .await?;
            Ok(())
        })
        .await?;
    Ok(())
}
