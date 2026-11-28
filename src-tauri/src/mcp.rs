//! Our MCP server, offered to every agent session: tools the agent can call on its own.
//!   screenshot_game, zoom_screen, watch_screen — game frames (`shots.rs`, `capture::look`);
//!   progress_add / _update / _remove — progress memory (`progress.rs`), only while it is on;
//!   widget_list / _create / _update / _delete — cards over the game window (`widgets.rs`).
//! Served by the UI server at `/mcp` (streamable HTTP), loopback only, with a per-run bearer
//! token. Each agent session connects anew, so a settings change applies to new sessions.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::schemars::JsonSchema;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::json;
use tracing::{info, warn};

use crate::capture::look::{self, Region};
use crate::capture::{self, Frame};
use crate::state::AppState;
use crate::{chat, progress, shots, widgets};

pub const SERVER_NAME: &str = "overlai";
pub const SCREENSHOT_TOOL: &str = "screenshot_game";
/// Tools that return a game frame (shown as a thumbnail on the tool call in the chat).
pub const IMAGE_TOOLS: [&str; 3] = [SCREENSHOT_TOOL, "zoom_screen", "watch_screen"];
const PROGRESS_TOOLS: [&str; 3] = ["progress_add", "progress_update", "progress_remove"];

/// `watch_screen` limits: agents give up on a tool call after about a minute (Codex: 60 s).
const WATCH_MAX_S: u64 = 50;
const WATCH_DEFAULT_S: u64 = 30;
const WATCH_EVERY: Duration = Duration::from_millis(500);
/// Mean brightness difference (0..1) that counts as "the screen changed"; below `STILL` for
/// `SETTLED` counts as "it settled". Idle animation and particles stay well under `CHANGED`.
const CHANGED: f32 = 0.06;
const STILL: f32 = 0.015;
const SETTLED: Duration = Duration::from_millis(1500);

#[derive(Clone)]
struct Overlai {
    state: Arc<AppState>,
    tools: ToolRouter<Self>,
}

impl Overlai {
    fn new(state: Arc<AppState>) -> Self {
        let mut tools = Self::tool_router();
        if !progress::enabled(&state) {
            for name in PROGRESS_TOOLS {
                tools.remove_route(name);
            }
        }
        Self { state, tools }
    }

    /// The game the overlay was last opened over (or a phone asked about).
    fn game_exe(&self) -> Result<String, String> {
        self.state
            .overlay
            .lock()
            .unwrap()
            .game
            .as_ref()
            .map(|g| g.exe.clone())
            .filter(|e| !e.is_empty())
            .ok_or_else(|| "the game is unknown: the user should open the overlay over the game with its hotkey".into())
    }

    /// Saves the frame with the chat, shows it on the tool call, and returns it with `note`.
    async fn image(&self, frame: Arc<Frame>, size: Option<(u32, u32)>, note: impl FnOnce(&shots::Stored) -> String) -> CallToolResult {
        let state = &self.state;
        let chat_id = state.chat.lock().unwrap().id.clone();
        let stored = match size {
            Some((w, h)) => shots::store_as(state, &chat_id, frame, w, h).await,
            None => shots::store(state, &chat_id, frame).await,
        };
        let stored = match stored {
            Ok(s) => s,
            Err(reason) => return text_error(format!("Could not encode the frame: {reason}")),
        };
        let note = note(&stored);
        chat::attach_tool_shot(state, stored.shot);
        CallToolResult::success(vec![ContentBlock::image(stored.base64, stored.mime), ContentBlock::text(note)])
    }

    fn describe(&self, stored: &shots::Stored, source: capture::Source) -> String {
        let game = self.state.overlay.lock().unwrap().game.clone();
        let mut note = match game {
            Some(g) => format!("Game window \"{}\" ({}), {}×{}", g.title, g.exe, stored.shot.width, stored.shot.height),
            None => format!("Game window, {}×{}", stored.shot.width, stored.shot.height),
        };
        if (stored.sent_width, stored.sent_height) != (stored.shot.width, stored.shot.height) {
            note.push_str(&format!(" (scaled to {}×{} for you)", stored.sent_width, stored.sent_height));
        }
        if source == capture::Source::Monitor {
            note.push_str(", whole monitor (the window itself could not be captured)");
        }
        note
    }
}

fn text_error(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text.into())])
}

fn text_ok(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text.into())])
}

fn json_ok(value: serde_json::Value) -> CallToolResult {
    text_ok(serde_json::to_string_pretty(&value).unwrap_or_default())
}

// ------------------------------------------------------------------ parameters

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct ZoomParams {
    /// Left edge of the region, in pixels of the last screenshot as you saw it.
    x: f64,
    /// Top edge, same pixels.
    y: f64,
    width: f64,
    height: f64,
    /// Crop a fresh frame instead of the last screenshot (same coordinates). Default false.
    #[serde(default)]
    fresh: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "lowercase")]
enum Until {
    /// The screen (or region) becomes clearly different from now.
    Change,
    /// The screen stops changing (for about 1.5 s).
    Stable,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct WatchParams {
    /// What to wait for: "change" (default) or "stable".
    until: Option<Until>,
    /// Give up after this many seconds (5..50, default 30); a frame is returned either way.
    timeout_seconds: Option<u64>,
    /// Watch only this region (pixels of the last screenshot as you saw it): x, y, width, height.
    region: Option<[f64; 4]>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct ProgressAddParams {
    /// One short, self-contained fact, e.g. "Reached Chapter 3 (Liurnia)" or "Plays an INT mage, Moonveil".
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct ProgressUpdateParams {
    /// The note's id (from the list in your instructions or progress_add).
    id: String,
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct IdParams {
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct ItemParam {
    text: String,
    /// Ticked. Omit to keep the tick of an existing item with the same text (new items: not done).
    done: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct WidgetParams {
    /// Required for widget_update; ignored by widget_create.
    id: Option<String>,
    /// Short title (required to create).
    title: Option<String>,
    /// Short Markdown shown under the title (a few lines).
    body: Option<String>,
    /// The whole checklist, replacing the current one. The user ticks items; ticks are kept by text.
    items: Option<Vec<ItemParam>>,
    /// Start a countdown of this many seconds (0 removes it).
    timer_seconds: Option<f64>,
    /// Any JSON you want to keep with the widget (counts, stages...). Not shown to the user;
    /// replaces the previous data, null clears it.
    data: Option<serde_json::Value>,
    /// Width in px (160..640, default 260).
    width: Option<f64>,
}

impl WidgetParams {
    fn change(self) -> widgets::Change {
        widgets::Change {
            title: self.title,
            body: self.body,
            items: self.items.map(|items| items.into_iter().map(|i| (i.text, i.done)).collect()),
            timer_seconds: self.timer_seconds,
            data: self.data,
            width: self.width,
        }
    }
}

/// A widget as the agent reads it.
fn widget_json(w: &crate::protocol::Widget) -> serde_json::Value {
    let left = w.timer_ends_at.map(|end| ((end - crate::history::now_ms()).max(0) as f64 / 1000.0).round());
    json!({
        "id": w.id,
        "title": w.title,
        "body": w.body,
        "items": w.items.iter().map(|i| json!({ "text": i.text, "done": i.done })).collect::<Vec<_>>(),
        "timer_seconds_left": left,
        "data": serde_json::from_str::<serde_json::Value>(&w.data).unwrap_or(serde_json::Value::Null),
        "folded_by_user": w.collapsed,
    })
}

// ------------------------------------------------------------------ tools

#[tool_router]
impl Overlai {
    #[tool(
        name = "screenshot_game",
        description = "Capture a frame of the game the user is playing (the game window). Call it whenever the answer \
            may depend on what is on screen: the user refers to something on screen, asks about their current situation, \
            or you need to check the game state. The first call after the user opens the overlay returns the frame from \
            that moment; later calls take a fresh one. Returns one image.",
        meta = always_load_meta()
    )]
    async fn screenshot_game(&self) -> Result<CallToolResult, ErrorData> {
        let taken = match shots::for_agent(&self.state).await {
            Ok(t) => t,
            Err(reason) => {
                info!("screenshot_game: no frame ({reason})");
                return Ok(text_error(format!("Could not capture the game: {reason}")));
            }
        };
        let (frame, source) = (taken.frame.clone(), taken.frame.source);
        let result = self
            .image(frame.clone(), None, |stored| {
                shots::set_seen(frame.clone(), stored.sent_width, stored.sent_height);
                info!("screenshot_game: {}x{} sent as {}x{}", stored.shot.width, stored.shot.height, stored.sent_width, stored.sent_height);
                let mut note = self.describe(stored, source);
                match &taken.when {
                    shots::Taken::Now => note.push_str(", captured just now."),
                    shots::Taken::AtHotkey { age } => note.push_str(&format!(
                        ", captured {} s ago at the moment the user opened the overlay (what they are asking about). \
                         Call the tool again for a fresh frame.",
                        age.as_secs()
                    )),
                    shots::Taken::Fallback { reason, age } => note.push_str(&format!(
                        ", captured {} s ago when the user opened the overlay; a fresh frame is not available now ({reason}).",
                        age.as_secs()
                    )),
                }
                note
            })
            .await;
        Ok(result)
    }

    #[tool(
        name = "zoom_screen",
        description = "Look closer at part of the last screenshot: returns that region at the game's full resolution, \
            enlarged if small. Use it for small text you can't read in the screenshot (item stats, tooltips, quest text, \
            chat, map labels, numbers). Coordinates are pixels of the last screenshot as you saw it. Call screenshot_game \
            first. Set fresh=true to take the region from a new frame instead.",
        meta = always_load_meta()
    )]
    async fn zoom_screen(&self, Parameters(p): Parameters<ZoomParams>) -> Result<CallToolResult, ErrorData> {
        let Some(seen) = shots::seen() else {
            return Ok(text_error("No screenshot yet: call screenshot_game first, then zoom into a region of it."));
        };
        let base = if p.fresh {
            match shots::grab_now(&self.state).await {
                Ok(f) => Arc::new(f),
                Err(reason) => return Ok(text_error(format!("Could not capture the game: {reason}"))),
            }
        } else {
            seen.frame.clone()
        };
        let Some(region) = Region::from_seen(p.x, p.y, p.width, p.height, (seen.width, seen.height), (base.width, base.height))
        else {
            return Ok(text_error(format!(
                "The region is outside the screenshot ({}×{} as you saw it).",
                seen.width, seen.height
            )));
        };
        let crop = Arc::new(look::crop(&base, region));
        let size = look::zoom_size(crop.width, crop.height);
        info!("zoom_screen: {region:?} of {}x{}, shown at {}x{}", base.width, base.height, size.0, size.1);
        let fresh = p.fresh;
        Ok(self
            .image(crop, Some(size), move |stored| {
                format!(
                    "Region x={} y={} {}×{} of the {} at full resolution ({}×{} px of {}×{}), shown at {}×{}.",
                    p.x.round(),
                    p.y.round(),
                    p.width.round(),
                    p.height.round(),
                    if fresh { "game now" } else { "last screenshot" },
                    region.width,
                    region.height,
                    base.width,
                    base.height,
                    stored.sent_width,
                    stored.sent_height
                )
            })
            .await)
    }

    #[tool(
        name = "watch_screen",
        description = "Wait until the game screen changes (until=\"change\") or settles (until=\"stable\"), then return \
            a fresh frame. Use it when the user asks you to wait for something on screen and then react: a loading screen \
            to end, a dialog or result to appear, an animation to finish. Optionally watch only a region (pixels of the \
            last screenshot). Waits at most timeout_seconds (≤ 50); the user waits for your answer meanwhile, so use it \
            only when asked or clearly needed, and look at the frame to check it is what you waited for.",
        meta = always_load_meta()
    )]
    async fn watch_screen(&self, Parameters(p): Parameters<WatchParams>) -> Result<CallToolResult, ErrorData> {
        let until = p.until.unwrap_or(Until::Change);
        let timeout = Duration::from_secs(p.timeout_seconds.unwrap_or(WATCH_DEFAULT_S).clamp(5, WATCH_MAX_S));
        let started = Instant::now();
        let first = match shots::grab_now(&self.state).await {
            Ok(f) => f,
            Err(reason) => return Ok(text_error(format!("Could not capture the game: {reason}"))),
        };
        let region = match p.region {
            Some([x, y, w, h]) => {
                let seen = shots::seen().map(|s| (s.width, s.height)).unwrap_or((first.width, first.height));
                match Region::from_seen(x, y, w, h, seen, (first.width, first.height)) {
                    Some(r) => r,
                    None => return Ok(text_error("The region is outside the screen.")),
                }
            }
            None => Region { x: 0, y: 0, width: first.width, height: first.height },
        };
        let base = look::fingerprint(&first, region);
        let (mut previous, mut still_since, mut last) = (base.clone(), Instant::now(), first);
        let mut outcome = None;
        while started.elapsed() < timeout {
            tokio::time::sleep(WATCH_EVERY).await;
            let frame = match shots::grab_now(&self.state).await {
                Ok(f) if (f.width, f.height) == (last.width, last.height) => f,
                Ok(f) => {
                    // The window changed size (resolution, mode): that is a change.
                    last = f;
                    outcome = Some("the game window changed size");
                    break;
                }
                Err(reason) => {
                    warn!("watch_screen: {reason}");
                    outcome = Some("the game can't be captured now");
                    break;
                }
            };
            let now = look::fingerprint(&frame, region);
            let met = match until {
                Until::Change => now.difference(&base) > CHANGED,
                Until::Stable => {
                    if now.difference(&previous) > STILL {
                        still_since = Instant::now();
                    }
                    still_since.elapsed() >= SETTLED
                }
            };
            previous = now;
            last = frame;
            if met {
                outcome = Some(match until {
                    Until::Change => "the screen changed",
                    Until::Stable => "the screen settled",
                });
                break;
            }
        }
        let waited = started.elapsed().as_secs_f32();
        let outcome = outcome.map(str::to_owned).unwrap_or_else(|| {
            format!("timed out: no {} within {} s", if matches!(until, Until::Change) { "change" } else { "settling" }, timeout.as_secs())
        });
        info!("watch_screen ({until:?}): {outcome} after {waited:.1} s");
        let source = last.source;
        let frame = Arc::new(last);
        Ok(self
            .image(frame.clone(), None, |stored| {
                shots::set_seen(frame.clone(), stored.sent_width, stored.sent_height);
                format!("{} after {waited:.1} s. {}, captured just now.", capitalize(&outcome), self.describe(stored, source))
            })
            .await)
    }

    #[tool(
        name = "progress_add",
        description = "Remember a lasting fact about the user's progress in the current game for future chats (chapter or \
            area reached, character, class or build, bosses beaten, quests done, important choices, goals). One short \
            fact per call. Returns the note's id.",
        meta = always_load_meta()
    )]
    async fn progress_add(&self, Parameters(p): Parameters<ProgressAddParams>) -> Result<CallToolResult, ErrorData> {
        let result = self.game_exe().and_then(|exe| progress::add(&self.state, &exe, &p.text));
        Ok(match result {
            Ok(note) => {
                chat::progress_changed_by_agent(&self.state);
                text_ok(format!("Saved as {}.", note.id))
            }
            Err(e) => text_error(e),
        })
    }

    #[tool(
        name = "progress_update",
        description = "Replace the text of a saved progress note (by id) when the fact changed, e.g. a later chapter.",
        meta = always_load_meta()
    )]
    async fn progress_update(&self, Parameters(p): Parameters<ProgressUpdateParams>) -> Result<CallToolResult, ErrorData> {
        Ok(match progress::edit(&self.state, &p.id, &p.text) {
            Ok(()) => {
                chat::progress_changed_by_agent(&self.state);
                text_ok("Updated.")
            }
            Err(e) => text_error(e),
        })
    }

    #[tool(
        name = "progress_remove",
        description = "Forget a saved progress note (by id): wrong, outdated, or the user asked to forget it.",
        meta = always_load_meta()
    )]
    async fn progress_remove(&self, Parameters(p): Parameters<IdParams>) -> Result<CallToolResult, ErrorData> {
        Ok(match progress::delete(&self.state, &p.id) {
            Ok(()) => {
                chat::progress_changed_by_agent(&self.state);
                text_ok("Removed.")
            }
            Err(e) => text_error(e),
        })
    }

    #[tool(
        name = "widget_list",
        description = "List the widgets pinned over the current game's window, with their checklist ticks (the user \
            ticks items), countdowns and your saved data. Read them before changing a widget.",
        meta = always_load_meta()
    )]
    async fn widget_list(&self) -> Result<CallToolResult, ErrorData> {
        Ok(match self.game_exe() {
            Ok(exe) => json_ok(json!(widgets::for_game(&self.state, &exe).iter().map(widget_json).collect::<Vec<_>>())),
            Err(e) => text_error(e),
        })
    }

    #[tool(
        name = "widget_create",
        description = "Pin a small card over the current game's window, kept for this game across chats and restarts: \
            a title, short Markdown, an optional checklist the user ticks, an optional countdown, and hidden data you can \
            keep progress in. Create one when the user asks to pin, track or keep something in view (quest steps, a build, \
            materials to collect, a timer, a counter), or offer it when it would clearly help. Keep it short: it covers \
            part of the game. The user can move, resize, fold it and make it see-through. Returns the widget.",
        meta = always_load_meta()
    )]
    async fn widget_create(&self, Parameters(p): Parameters<WidgetParams>) -> Result<CallToolResult, ErrorData> {
        let result = self.game_exe().and_then(|exe| widgets::create(&self.state, &exe, p.change()));
        Ok(match result {
            Ok(w) => json_ok(widget_json(&w)),
            Err(e) => text_error(e),
        })
    }

    #[tool(
        name = "widget_update",
        description = "Change a widget (by id): only the fields you give change. `items` replaces the whole checklist \
            (ticks are kept for items with the same text unless you set `done`), `data` replaces your data. Use it to \
            record progress, tick or add steps, restart a countdown. Returns the widget.",
        meta = always_load_meta()
    )]
    async fn widget_update(&self, Parameters(p): Parameters<WidgetParams>) -> Result<CallToolResult, ErrorData> {
        let Some(id) = p.id.clone() else {
            return Ok(text_error("Give the widget's id (see widget_list)."));
        };
        Ok(match widgets::update(&self.state, &id, p.change()) {
            Ok(w) => json_ok(widget_json(&w)),
            Err(e) => text_error(e),
        })
    }

    #[tool(
        name = "widget_delete",
        description = "Remove a widget (by id) when it is no longer needed or the user asks.",
        meta = always_load_meta()
    )]
    async fn widget_delete(&self, Parameters(p): Parameters<IdParams>) -> Result<CallToolResult, ErrorData> {
        Ok(match widgets::delete(&self.state, &p.id) {
            Ok(()) => text_ok("Deleted."),
            Err(e) => text_error(e),
        })
    }
}

#[tool_handler(router = self.tools)]
impl rmcp::ServerHandler for Overlai {}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
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
    let service = StreamableHttpService::new(
        move || Ok(Overlai::new(state.clone())),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_have_schemas_and_always_load() {
        let tools = Overlai::tool_router().list_all();
        let names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
        for name in IMAGE_TOOLS.iter().chain(&PROGRESS_TOOLS).chain(&["widget_list", "widget_create", "widget_update", "widget_delete"]) {
            assert!(names.iter().any(|n| n == name), "{name} missing in {names:?}");
        }
        for t in &tools {
            assert!(t.meta.as_ref().is_some_and(|m| m.0.contains_key("anthropic/alwaysLoad")), "{} not always loaded", t.name);
        }
        let zoom = tools.iter().find(|t| t.name == "zoom_screen").unwrap();
        let props = zoom.input_schema.get("properties").and_then(|p| p.as_object()).unwrap();
        assert!(["x", "y", "width", "height", "fresh"].iter().all(|k| props.contains_key(*k)), "{props:?}");
    }
}
