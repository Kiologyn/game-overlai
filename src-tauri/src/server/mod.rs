//! Local HTTP server: serves the UI (overlay and browser use the same pages) and the
//! WebSocket the UI talks to the core through. Listens on 127.0.0.1 only for now
//! (LAN access with a token comes in phase 8).

mod static_files;
mod ws;

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tauri::AppHandle;
use tracing::{error, info, warn};

use crate::state::AppState;

#[derive(Clone)]
pub struct Ctx {
    pub app: AppHandle,
    pub state: Arc<AppState>,
    allowed_hosts: Arc<Vec<String>>,
    allowed_origins: Arc<Vec<String>>,
}

/// Binds the configured port, or a free one if it is taken.
pub fn bind(preferred: u16) -> anyhow::Result<(std::net::TcpListener, u16)> {
    let listener = match std::net::TcpListener::bind(("127.0.0.1", preferred)) {
        Ok(l) => l,
        Err(e) => {
            warn!("port {preferred} is busy ({e}); using a random free port");
            std::net::TcpListener::bind(("127.0.0.1", 0))?
        }
    };
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

pub async fn run(listener: std::net::TcpListener, app: AppHandle, state: Arc<AppState>) {
    let port = state.port;
    let allowed_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let mut allowed_origins = vec![format!("http://127.0.0.1:{port}"), format!("http://localhost:{port}")];
    if !state.ui_base.contains(&format!(":{port}")) {
        allowed_origins.push(state.ui_base.trim_end_matches('/').to_string()); // Vite dev server
    }
    let mcp = crate::mcp::router(state.clone());
    let ctx = Ctx { app, state, allowed_hosts: Arc::new(allowed_hosts), allowed_origins: Arc::new(allowed_origins) };

    let router = Router::new()
        .route("/ws", get(ws::handler))
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/shots/{id}", get(shot))
        .fallback(static_files::serve)
        .with_state(ctx.clone())
        .merge(mcp)
        .layer(middleware::from_fn_with_state(ctx, guard));

    let listener = match tokio::net::TcpListener::from_std(listener) {
        Ok(l) => l,
        Err(e) => {
            error!("UI server: {e}");
            return;
        }
    };
    info!("UI server on http://127.0.0.1:{port}");
    if let Err(e) = axum::serve(listener, router).await {
        error!("UI server stopped: {e}");
    }
}

/// A game frame of a chat (thumbnails and the full view in the UI).
async fn shot(State(ctx): State<Ctx>, Path(id): Path<String>) -> Response {
    let Some((bytes, mime)) = crate::shots::load(&ctx.state, &id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut res = bytes.into_response();
    let headers = res.headers_mut();
    if let Ok(mime) = HeaderValue::from_str(&mime) {
        headers.insert(header::CONTENT_TYPE, mime);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=86400, immutable"));
    res
}

/// Rejects requests whose Host/Origin is not ours. Protects the local server from web pages
/// in the user's browser (DNS rebinding, cross-site WebSocket).
async fn guard(State(ctx): State<Ctx>, req: Request, next: Next) -> Response {
    let host = header_value(&req, header::HOST);
    let origin = header_value(&req, header::ORIGIN);
    let host_ok = host.as_deref().is_some_and(|h| ctx.allowed_hosts.iter().any(|a| a == h));
    let origin_ok = origin.as_deref().is_none_or(|o| ctx.allowed_origins.iter().any(|a| a == o));
    if host_ok && origin_ok {
        return next.run(req).await;
    }
    warn!("rejected request {} (host {host:?}, origin {origin:?})", req.uri().path());
    StatusCode::FORBIDDEN.into_response()
}

fn header_value(req: &Request, name: header::HeaderName) -> Option<String> {
    req.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}
