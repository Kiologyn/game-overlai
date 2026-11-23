use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

use super::Ctx;
use crate::protocol::{ClientKind, ClientMsg, ServerMsg};
use crate::state::Target;
use crate::history::ProfileEdit;
use crate::{chat, games, lan, overlay};

#[derive(Deserialize)]
pub struct WsQuery {
    client: Option<String>,
}

pub async fn handler(ws: WebSocketUpgrade, Query(q): Query<WsQuery>, State(ctx): State<Ctx>) -> Response {
    let kind = match q.client.as_deref() {
        Some("overlay") if !ctx.lan => ClientKind::Overlay,
        _ => ClientKind::Browser,
    };
    ws.on_upgrade(move |socket| session(socket, kind, ctx))
}

async fn session(socket: WebSocket, kind: ClientKind, ctx: Ctx) {
    debug!("ws connected: {kind:?}{}", if ctx.lan { " (LAN)" } else { "" });
    let (mut sink, mut stream) = socket.split();
    let mut events = ctx.state.hub.subscribe();
    // A device on the network: counted, and dropped when access ends (off / token reset).
    let _device = ctx.lan.then(|| lan::client(&ctx.state));
    let mut lan_generation = ctx.state.lan.generation();

    let mut initial = vec![
        ServerMsg::Hello { version: env!("CARGO_PKG_VERSION").into(), client: kind },
        chat::snapshot(&ctx.state),
        chat::chat_list(&ctx.state),
        games::list(&ctx.state),
        chat::permissions(&ctx.state),
        ServerMsg::AgentStatus { status: ctx.state.agent_status.lock().unwrap().clone() },
    ];
    if !ctx.lan {
        initial.push(lan::status(&ctx.state));
    }
    if let Some(usage) = ctx.state.usage.lock().unwrap().clone() {
        initial.push(ServerMsg::AgentUsage { usage });
    }
    if kind == ClientKind::Overlay {
        let ov = ctx.state.overlay.lock().unwrap();
        if ov.visible {
            initial.push(ServerMsg::OverlayShow { mode: ov.mode, game: ov.game.clone() });
        }
    }
    for msg in initial {
        if send(&mut sink, &msg).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => handle(&ctx, kind, text.as_str()),
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(e)) => { debug!("ws error: {e}"); break; }
                Some(Ok(_)) => {}
            },
            event = events.recv() => match event {
                Ok((target, msg)) => {
                    let wanted = match target {
                        Target::All => true,
                        Target::Overlay => kind == ClientKind::Overlay,
                        Target::Local => !ctx.lan,
                    };
                    if wanted && send(&mut sink, &msg).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    warn!("ws client lagged by {n} events; resending the chat snapshot");
                    if send(&mut sink, &chat::snapshot(&ctx.state)).await.is_err() { break; }
                    if send(&mut sink, &chat::chat_list(&ctx.state)).await.is_err() { break; }
                    if send(&mut sink, &games::list(&ctx.state)).await.is_err() { break; }
                }
                Err(RecvError::Closed) => break,
            },
            _ = lan_generation.changed(), if ctx.lan => {
                debug!("ws: LAN access ended, device dropped");
                let _ = sink.send(Message::Close(None)).await;
                break;
            }
        }
    }
    debug!("ws disconnected: {kind:?}");
}

async fn send(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, msg: &ServerMsg) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}

fn handle(ctx: &Ctx, kind: ClientKind, text: &str) {
    let msg = match serde_json::from_str::<ClientMsg>(text) {
        Ok(m) => m,
        Err(e) => {
            warn!("bad message from {kind:?}: {e}");
            return;
        }
    };
    match msg {
        // Only the overlay panel controls the overlay window.
        ClientMsg::UiHide if kind == ClientKind::Overlay => overlay::run_on_main(&ctx.app, overlay::hide),
        ClientMsg::UiSetMode { mode } if kind == ClientKind::Overlay => {
            overlay::run_on_main(&ctx.app, move |app| overlay::set_mode(app, mode))
        }
        ClientMsg::UiDragStart if kind == ClientKind::Overlay => {
            debug!("ui.dragStart");
            overlay::run_on_main(&ctx.app, overlay::start_drag)
        }
        ClientMsg::UiResizeStart { edge } if kind == ClientKind::Overlay => {
            debug!("ui.resizeStart {edge:?}");
            overlay::run_on_main(&ctx.app, move |app| overlay::start_resize(app, edge))
        }
        ClientMsg::UiHide | ClientMsg::UiSetMode { .. } | ClientMsg::UiDragStart | ClientMsg::UiResizeStart { .. } => {}
        // From a phone the game is whatever runs on the PC now (the overlay may never have opened).
        ClientMsg::ChatSend { text, screenshot, new_chat } => {
            if ctx.lan {
                overlay::adopt_foreground_game(&ctx.state);
            }
            chat::send(&ctx.state, &text, screenshot, new_chat)
        }
        ClientMsg::ChatCancel => chat::cancel(&ctx.state),
        ClientMsg::ChatNew => chat::new_chat(&ctx.state),
        ClientMsg::ChatOpen { id } => chat::open(&ctx.state, &id),
        ClientMsg::ChatDelete { id } => chat::delete(&ctx.state, &id),
        ClientMsg::GameUpdate { exe, name, instructions, sources, screenshot } => {
            games::update(&ctx.state, &exe, ProfileEdit { name, instructions, sources, screenshot })
        }
        ClientMsg::PermissionRespond { id, option_id } => chat::respond_permission(&ctx.state, &id, option_id),
        // This PC's UI only: devices on the network can't change access or open things on the PC.
        ClientMsg::LanSet { .. } | ClientMsg::LanResetToken | ClientMsg::UiOpenUrl { .. } if ctx.lan => {
            warn!("LAN device sent a local-only message: ignored");
        }
        ClientMsg::LanSet { enabled } => lan::set_enabled(&ctx.state, enabled),
        ClientMsg::LanResetToken => lan::reset_token(&ctx.state),
        ClientMsg::UiOpenUrl { url } => {
            if url.starts_with("https://") || url.starts_with("http://") {
                use tauri_plugin_opener::OpenerExt;
                if let Err(e) = ctx.app.opener().open_url(&url, None::<&str>) {
                    warn!("could not open {url}: {e}");
                }
            } else {
                warn!("refused to open non-http url {url:?}");
            }
        }
    }
}
