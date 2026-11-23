//! One ACP connection to an agent process (agent-client-protocol v1 API).
//!
//! Rules from the crate docs that shape this code:
//! - request/notification handlers run inside the dispatch loop and must return quickly:
//!   the permission handler only registers the prompt and answers from a spawned task;
//! - requests are sent with `block_task()` from spawned tasks, never from handlers;
//! - a clean agent exit does not end `connect_with`: we watch `incoming_closed()`.

use std::path::Path;
use std::sync::Arc;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    CancelNotification, CloseSessionRequest, ContentBlock, HttpHeader, InitializeRequest,
    LoadSessionRequest, McpServer, McpServerHttp, NewSessionRequest, PermissionOptionKind, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse, ResumeSessionRequest,
    SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigSelectOptions, SessionOptionsRequest, SetSessionConfigOptionRequest,
    SessionNotification, SessionUpdate, StopReason as AcpStopReason, SystemPrompt, TextContent,
    ToolCallStatus, ToolKind,
};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client, LineDirection};
use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use tracing::{debug, info, trace, warn};

use super::{AgentSpec, Cmd, PromptBlock, Shared, StopReason, Update};
use crate::protocol::{
    AgentOption, AgentOptionKind, AgentOptionValue, AgentState, PermissionOption, PermissionPrompt, ToolInfo, ToolStatus,
    Usage,
};

/// A session config option as the agent sends it.
pub(super) type RawOption = SessionConfigOption;
use crate::proxy::ProxyEnv;

pub(super) async fn run(
    spec: &'static AgentSpec,
    entry: &Path,
    proxy: &ProxyEnv,
    shared: Arc<Shared>,
    commands: Arc<AsyncMutex<mpsc::UnboundedReceiver<Cmd>>>,
    first: Option<Cmd>,
) -> Result<(), String> {
    // Proxy variables reach node and the claude.exe it starts (they inherit the environment).
    let config = proxy
        .vars()
        .into_iter()
        .fold(AcpAgentConfig::new("node").args([entry.display().to_string()]), |c, (k, v)| c.env(k, v));
    let agent = AcpAgent::new(config).with_debug(|line, direction| {
            if direction == LineDirection::Stderr {
                trace!(target: "agent", "{line}");
            }
        });

    let on_update = shared.clone();
    let on_permission = shared.clone();
    let main = shared.clone();

    Client
        .v1()
        .name("game-overlai")
        .on_receive_notification(
            async move |n: SessionNotification, _cx| {
                on_notification(&on_update, n);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest, responder, cx| {
                let prompt = permission_prompt(&req);
                let answer = on_permission.register_permission(req.session_id.0.to_string(), prompt);
                // Wait for the user outside the dispatch loop.
                cx.spawn(async move {
                    let outcome = match answer.await.ok().flatten() {
                        Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option)),
                        None => RequestPermissionOutcome::Cancelled,
                    };
                    responder.respond(RequestPermissionResponse::new(outcome))
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, async move |cx: ConnectionTo<Agent>| {
            let init = cx.send_request(InitializeRequest::new(ProtocolVersion::V1)).block_task().await?;
            let caps = &init.agent_capabilities;
            info!(
                "agent {} ready: {:?}, image={} resume={} load={} mcp_http={}",
                spec.id,
                init.agent_info.as_ref().map(|i| format!("{} {}", i.name, i.version)),
                caps.prompt_capabilities.image,
                caps.session_capabilities.resume.is_some(),
                caps.load_session,
                caps.mcp_capabilities.http
            );
            let restore = if caps.session_capabilities.resume.is_some() {
                Restore::Resume
            } else if caps.load_session {
                Restore::Load
            } else {
                Restore::Unsupported
            };
            // Our MCP server goes into every session (over HTTP; stdio-only agents get none).
            let mcp_servers: Vec<McpServer> = match &main.mcp {
                Some(mcp) if caps.mcp_capabilities.http => vec![McpServer::Http(
                    McpServerHttp::new(mcp.name, &mcp.url)
                        .headers(vec![HttpHeader::new("Authorization", format!("Bearer {}", mcp.token))]),
                )],
                Some(_) => {
                    warn!("agent {} has no HTTP MCP support: no screenshot tool", spec.id);
                    vec![]
                }
                None => vec![],
            };
            main.status(AgentState::Ready, None);
            // The model / effort choices before any session exists (`session/options`).
            let list_options = caps.session_capabilities.options.is_some();
            main.options.lock().unwrap().clear();
            dispatch(&cx, &main, spec, &mcp_servers, restore, list_options, Cmd::ApplyOptions { session_id: None })?;

            let mut first = first;
            loop {
                let cmd = match first.take() {
                    Some(cmd) => cmd,
                    None => {
                        let mut rx = commands.lock().await;
                        tokio::select! {
                            cmd = rx.recv() => match cmd {
                                Some(cmd) => cmd,
                                None => break,
                            },
                            _ = cx.incoming_closed() => {
                                info!("agent {} exited", spec.id);
                                break;
                            }
                        }
                    }
                };
                if let Cmd::Restart = cmd {
                    info!("agent {} restart requested", spec.id);
                    break;
                }
                dispatch(&cx, &main, spec, &mcp_servers, restore, list_options, cmd)?;
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())
}

/// How this agent continues an earlier session.
#[derive(Debug, Clone, Copy)]
enum Restore {
    /// `session/resume`: no history replay.
    Resume,
    /// `session/load`: the agent replays the history first (we drop it, see `replaying`).
    Load,
    Unsupported,
}

fn dispatch(
    cx: &ConnectionTo<Agent>,
    shared: &Arc<Shared>,
    spec: &'static AgentSpec,
    mcp_servers: &[McpServer],
    restore: Restore,
    list_options: bool,
    cmd: Cmd,
) -> Result<(), agent_client_protocol::Error> {
    let conn = cx.clone();
    let mcp_servers = mcp_servers.to_vec();
    match cmd {
        Cmd::NewSession { cwd, system_prompt, reply } => {
            let shared = shared.clone();
            cx.spawn(async move {
                let request = NewSessionRequest::new(cwd)
                    .mcp_servers(mcp_servers)
                    .system_prompt(SystemPrompt::new(vec![text(&system_prompt)]).mode(spec.system_prompt_mode.clone()))
                    .meta((spec.session_meta)());
                let result = match conn.send_request(request).block_task().await {
                    Ok(r) => {
                        let id = r.session_id.0.to_string();
                        // The user's model / effort before the first prompt.
                        apply_prefs(&conn, &shared, &id, r.config_options.unwrap_or_default()).await;
                        Ok(id)
                    }
                    Err(e) => Err(e.to_string()),
                };
                let _ = reply.send(result);
                Ok(())
            })
        }
        Cmd::RestoreSession { session_id, cwd, system_prompt, reply } => {
            let shared = shared.clone();
            let prompt = SystemPrompt::new(vec![text(&system_prompt)]).mode(spec.system_prompt_mode.clone());
            cx.spawn(async move {
                let result = match restore {
                    Restore::Resume => {
                        let request = ResumeSessionRequest::new(session_id.clone(), cwd)
                            .mcp_servers(mcp_servers)
                            .system_prompt(prompt)
                            .meta((spec.session_meta)());
                        match conn.send_request(request).block_task().await {
                            Ok(r) => {
                                apply_prefs(&conn, &shared, &session_id, r.config_options.unwrap_or_default()).await;
                                Ok(())
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                    Restore::Load => {
                        shared.replaying.lock().unwrap().insert(session_id.clone());
                        let request = LoadSessionRequest::new(session_id.clone(), cwd)
                            .mcp_servers(mcp_servers)
                            .system_prompt(prompt)
                            .meta((spec.session_meta)());
                        let result = conn.send_request(request).block_task().await;
                        shared.replaying.lock().unwrap().remove(&session_id);
                        match result {
                            Ok(r) => {
                                apply_prefs(&conn, &shared, &session_id, r.config_options.unwrap_or_default()).await;
                                Ok(())
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                    Restore::Unsupported => Err(format!("{} не умеет продолжать сессии", spec.name)),
                };
                let _ = reply.send(result);
                Ok(())
            })
        }
        Cmd::Prompt { session_id, blocks, reply } => cx.spawn(async move {
            let blocks = blocks
                .into_iter()
                .map(|b| match b {
                    PromptBlock::Text(t) => text(&t),
                })
                .collect();
            let result = conn.send_request(PromptRequest::new(session_id, blocks)).block_task().await;
            let _ = reply.send(result.map(|r| stop_reason(r.stop_reason)).map_err(|e| e.to_string()));
            Ok(())
        }),
        Cmd::Cancel { session_id } => {
            debug!("cancel {session_id}");
            cx.send_notification(CancelNotification::new(session_id))
        }
        Cmd::ApplyOptions { session_id: Some(id) } => {
            let shared = shared.clone();
            cx.spawn(async move {
                let options = shared.options.lock().unwrap().clone();
                apply_prefs(&conn, &shared, &id, options).await;
                Ok(())
            })
        }
        Cmd::ApplyOptions { session_id: None } if list_options => {
            let shared = shared.clone();
            cx.spawn(async move {
                let request = SessionOptionsRequest::new(std::env::temp_dir()).meta((spec.session_meta)());
                match conn.send_request(request).block_task().await {
                    Ok(r) => {
                        // Not a session: show the defaults with the user's choices over them.
                        let mut options = public_options(&r.config_options.unwrap_or_default());
                        let prefs = shared.prefs.lock().unwrap().clone();
                        for o in &mut options {
                            if let Some(want) = prefs.get(&o.id).filter(|w| o.values.iter().any(|v| &v.value == *w)) {
                                o.current = want.clone();
                            }
                        }
                        shared.emit(super::AgentEvent::Options(options));
                    }
                    Err(e) => debug!("session/options: {e}"),
                }
                Ok(())
            })
        }
        Cmd::ApplyOptions { session_id: None } => Ok(()), // shown once a session exists
        Cmd::Restart => Ok(()), // handled by the command loop
        Cmd::CloseSession { session_id } => cx.spawn(async move {
            match conn.send_request(CloseSessionRequest::new(session_id.clone())).block_task().await {
                Ok(_) => debug!("closed session {session_id}"),
                Err(e) => debug!("close session {session_id}: {e}"),
            }
            Ok(())
        }),
    }
}

/// Sets the user's choices the session doesn't have yet, in the order the agent lists its
/// options (the model decides which effort levels exist), and reports the result.
async fn apply_prefs(conn: &ConnectionTo<Agent>, shared: &Shared, session_id: &str, mut options: Vec<RawOption>) {
    let prefs = shared.prefs.lock().unwrap().clone();
    let order: Vec<String> = options.iter().map(|o| o.id.0.to_string()).collect();
    for id in order {
        let Some(want) = prefs.get(&id) else { continue };
        let Some(option) = options.iter().find(|o| *o.id.0 == *id) else { continue };
        let Some((current, values)) = select_values(option) else { continue };
        if current == *want || !values.iter().any(|v| v.value == *want) {
            continue; // already so, or not offered (e.g. an effort level this model lacks)
        }
        let request = SetSessionConfigOptionRequest::new(session_id.to_string(), id.clone(), want.as_str());
        match conn.send_request(request).block_task().await {
            Ok(r) => {
                info!("session {session_id}: {id} = {want}");
                options = r.config_options;
            }
            Err(e) => warn!("session {session_id}: can't set {id} = {want}: {e}"),
        }
    }
    report_options(shared, options);
}

fn report_options(shared: &Shared, options: Vec<RawOption>) {
    let public = public_options(&options);
    *shared.options.lock().unwrap() = options;
    shared.emit(super::AgentEvent::Options(public));
}

/// The options the user picks from: the model and the reasoning effort (selects only).
fn public_options(options: &[RawOption]) -> Vec<AgentOption> {
    options
        .iter()
        .filter_map(|o| {
            let kind = match o.category.as_ref()? {
                SessionConfigOptionCategory::Model => AgentOptionKind::Model,
                SessionConfigOptionCategory::ThoughtLevel => AgentOptionKind::Effort,
                _ => return None,
            };
            let (current, values) = select_values(o)?;
            Some(AgentOption { id: o.id.0.to_string(), kind, name: o.name.clone(), current, values })
        })
        .collect()
}

fn select_values(o: &RawOption) -> Option<(String, Vec<AgentOptionValue>)> {
    let SessionConfigKind::Select(select) = &o.kind else { return None };
    let flat: Vec<_> = match &select.options {
        SessionConfigSelectOptions::Ungrouped(list) => list.iter().collect(),
        SessionConfigSelectOptions::Grouped(groups) => groups.iter().flat_map(|g| g.options.iter()).collect(),
        #[allow(unreachable_patterns)]
        _ => return None,
    };
    let values = flat
        .into_iter()
        .map(|v| AgentOptionValue { value: v.value.0.to_string(), name: v.name.clone(), description: v.description.clone() })
        .collect();
    Some((select.current_value.0.to_string(), values))
}

fn text(s: &str) -> ContentBlock {
    ContentBlock::Text(TextContent::new(s))
}

fn stop_reason(r: AcpStopReason) -> StopReason {
    match r {
        AcpStopReason::EndTurn => StopReason::EndTurn,
        AcpStopReason::Cancelled => StopReason::Cancelled,
        AcpStopReason::Refusal => StopReason::Refusal,
        AcpStopReason::MaxTokens | AcpStopReason::MaxTurnRequests => StopReason::MaxTokens,
        #[allow(unreachable_patterns)]
        other => StopReason::Other(format!("{other:?}")),
    }
}

// ------------------------------------------------------------------ session updates

fn on_notification(shared: &Shared, n: SessionNotification) {
    let session_id = n.session_id.0.to_string();
    if shared.replaying.lock().unwrap().contains(&session_id) {
        return; // history replay of session/load: we already have these messages
    }
    let update = match n.update {
        SessionUpdate::AgentMessageChunk(chunk) => match chunk.content {
            ContentBlock::Text(t) => Update::Text(t.text),
            _ => return,
        },
        SessionUpdate::AgentThoughtChunk(chunk) => match chunk.content {
            ContentBlock::Text(t) => Update::Thought(t.text),
            _ => return,
        },
        SessionUpdate::ToolCall(tc) => Update::Tool(ToolInfo {
            id: tc.tool_call_id.0.to_string(),
            title: tool_title(&tc.title),
            kind: tool_kind(&tc.kind),
            status: tool_status(&tc.status),
            shot: None,
        }),
        SessionUpdate::ToolCallUpdate(u) => Update::ToolPatch {
            id: u.tool_call_id.0.to_string(),
            title: u.fields.title.as_deref().map(tool_title),
            kind: u.fields.kind.as_ref().map(tool_kind),
            status: u.fields.status.as_ref().map(tool_status),
        },
        SessionUpdate::SessionInfoUpdate(info) => {
            match serde_json::to_value(&info).ok().and_then(|v| v.get("title")?.as_str().map(str::to_owned)) {
                Some(title) if !title.is_empty() => Update::Title(title),
                _ => return,
            }
        }
        SessionUpdate::ConfigOptionUpdate(u) => {
            report_options(shared, u.config_options);
            return;
        }
        SessionUpdate::UsageUpdate(u) => {
            if let Some(usage) = serde_json::to_value(&u).ok().and_then(|v| parse_usage(&v)) {
                shared.emit(super::AgentEvent::Usage(usage));
            }
            return;
        }
        _ => return,
    };
    shared.emit(super::AgentEvent::Update { session_id, update });
}

/// Claude Code puts subscription limits into `_meta["_claude/rateLimit"].unifiedWindows`
/// (other agents don't: no badge then).
fn parse_usage(v: &Value) -> Option<Usage> {
    let windows = v.get("_meta")?.get("_claude/rateLimit")?.get("unifiedWindows")?;
    let five = windows.get("five_hour");
    let seven = windows.get("seven_day");
    Some(Usage {
        five_hour: five.and_then(|w| w.get("utilization")?.as_f64()),
        seven_day: seven.and_then(|w| w.get("utilization")?.as_f64()),
        five_hour_resets_at: five.and_then(|w| w.get("resetsAt")?.as_i64()),
    })
}

/// "mcp__overlai__screenshot_game" -> "screenshot_game"; other titles unchanged.
fn tool_title(title: &str) -> String {
    match title.strip_prefix("mcp__") {
        Some(rest) => rest.split_once("__").map(|(_, tool)| tool).unwrap_or(rest).to_string(),
        None => title.to_string(),
    }
}

fn tool_kind(kind: &ToolKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "other".into())
}

fn tool_status(status: &ToolCallStatus) -> ToolStatus {
    match status {
        ToolCallStatus::Pending => ToolStatus::Pending,
        ToolCallStatus::InProgress => ToolStatus::Running,
        ToolCallStatus::Completed => ToolStatus::Done,
        ToolCallStatus::Failed => ToolStatus::Failed,
        #[allow(unreachable_patterns)]
        _ => ToolStatus::Running,
    }
}

fn permission_prompt(req: &RequestPermissionRequest) -> PermissionPrompt {
    let options = req
        .options
        .iter()
        .map(|o| PermissionOption {
            id: o.option_id.0.to_string(),
            name: o.name.clone(),
            kind: match o.kind {
                PermissionOptionKind::AllowOnce => "allow_once",
                PermissionOptionKind::AllowAlways => "allow_always",
                PermissionOptionKind::RejectOnce => "reject_once",
                PermissionOptionKind::RejectAlways => "reject_always",
                #[allow(unreachable_patterns)]
                _ => "other",
            }
            .into(),
        })
        .collect();
    PermissionPrompt {
        id: uuid::Uuid::new_v4().simple().to_string(),
        title: req.tool_call.fields.title.as_deref().map(tool_title).unwrap_or_else(|| "Действие агента".into()),
        options,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_from_claude_meta() {
        let v: Value = serde_json::from_str(
            r#"{"_meta":{"_claude/rateLimit":{"status":"allowed","unifiedWindows":{
                "five_hour":{"utilization":0.13,"resetsAt":1795141200},
                "seven_day":{"utilization":0.23,"resetsAt":1795395600}}}}}"#,
        )
        .unwrap();
        let u = parse_usage(&v).unwrap();
        assert_eq!(u.five_hour, Some(0.13));
        assert_eq!(u.seven_day, Some(0.23));
        assert_eq!(u.five_hour_resets_at, Some(1795141200));
        assert!(parse_usage(&serde_json::json!({"used": 5})).is_none());
    }

    #[test]
    fn tool_titles() {
        assert_eq!(tool_title("mcp__overlai__screenshot_game"), "screenshot_game");
        assert_eq!(tool_title("Web search"), "Web search");
    }
}
