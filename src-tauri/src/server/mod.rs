//! HTTP servers: serve the UI (overlay and browser use the same pages) and the WebSocket the
//! UI talks to the core through.
//!
//! - Local (127.0.0.1, always on): the overlay, browser tabs on this PC, our MCP server.
//! - LAN (all interfaces, own port, only while the user has it on; `lan.rs`): the browser chat
//!   for a phone. Needs the access token; no MCP, no overlay or LAN controls.

mod static_files;
mod ws;

use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
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
    /// The LAN server (devices on the network), not the local one.
    pub lan: bool,
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
    let ctx =
        Ctx { app, state, allowed_hosts: Arc::new(allowed_hosts), allowed_origins: Arc::new(allowed_origins), lan: false };

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

/// The LAN server; ends when `stop` fires (access turned off).
pub async fn run_lan(
    listener: std::net::TcpListener,
    app: AppHandle,
    state: Arc<AppState>,
    stop: tokio::sync::oneshot::Receiver<()>,
) {
    let ctx = Ctx { app, state, allowed_hosts: Arc::default(), allowed_origins: Arc::default(), lan: true };
    let router = Router::new()
        .route("/ws", get(ws::handler))
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/shots/{id}", get(shot))
        .fallback(static_files::serve)
        .with_state(ctx.clone())
        .layer(middleware::from_fn_with_state(ctx, lan_guard));
    let listener = match tokio::net::TcpListener::from_std(listener) {
        Ok(l) => l,
        Err(e) => {
            error!("LAN server: {e}");
            return;
        }
    };
    let service = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
    if let Err(e) = axum::serve(listener, service)
        .with_graceful_shutdown(async {
            let _ = stop.await;
        })
        .await
    {
        error!("LAN server stopped: {e}");
    }
}

/// LAN requests: same-origin only, and signed in with the access token. The QR link carries
/// the token once (`?t=`); it is swapped for a cookie and a URL without it.
async fn lan_guard(State(ctx): State<Ctx>, req: Request, next: Next) -> Response {
    let peer = req.extensions().get::<ConnectInfo<std::net::SocketAddr>>().map(|c| c.0.ip().to_string());
    let peer = peer.unwrap_or_default();
    let host = header_value(&req, header::HOST).unwrap_or_default();
    if let Some(origin) = header_value(&req, header::ORIGIN)
        && origin != format!("http://{host}")
    {
        warn!("LAN: rejected {} from {peer}: origin {origin:?}", req.uri().path());
        return StatusCode::FORBIDDEN.into_response();
    }
    let lan = &ctx.state.lan;
    let path = req.uri().path().to_string();
    let reason = if let Some(token) = query_param(req.uri().query().unwrap_or(""), "t") {
        if lan.token_ok(token) {
            let agent = header_value(&req, header::USER_AGENT).unwrap_or_default();
            info!("LAN: device {peer} signed in ({agent})");
            // Lax, not Strict: a QR scanner (e.g. Yandex) opens the link from its own site, and
            // Chromium then doesn't send a Strict cookie on the redirect to "/" (the device got
            // "no access" right after signing in). Other sites still can't use it: they would
            // need a WebSocket or a fetch, and those carry their Origin, refused above.
            let cookie = format!("{}={token}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax", crate::lan::COOKIE);
            let mut res = Redirect::to("/").into_response();
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                res.headers_mut().insert(header::SET_COOKIE, v);
            }
            return res;
        }
        "an outdated link"
    } else {
        match cookie(&req, crate::lan::COOKIE) {
            Some(t) if lan.token_ok(t) => return next.run(req).await,
            Some(_) => "an outdated cookie (access was reset)",
            None => "no cookie",
        }
    };
    warn!("LAN: {path} from {peer}: no access, {reason}");
    (StatusCode::UNAUTHORIZED, Html(no_access_page())).into_response()
}

/// The page a phone without a valid sign-in gets, in the app's language.
fn no_access_page() -> String {
    let lang = serde_json::to_value(crate::i18n::language()).ok().and_then(|v| v.as_str().map(str::to_owned));
    format!(
        r#"<!doctype html><html lang="{}"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>game-overlai</title>
<style>body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#0e0e0e;color:#ececec;
font:15px/1.5 system-ui,sans-serif}}main{{max-width:340px;padding:24px}}h1{{font-size:18px;margin:0 0 8px}}
p{{color:#8b8b8b;margin:0 0 8px}}</style></head><body><main><h1>{}</h1>
<p>{}</p>
<p>{}</p>
</main></body></html>"#,
        lang.unwrap_or_else(|| "en".into()),
        tr!("no_access"),
        tr!("no_access_rescan"),
        tr!("no_access_cookie"),
    )
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

fn cookie<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    req.headers()
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix(name)?.strip_prefix('='))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_and_cookies() {
        assert_eq!(query_param("t=abc", "t"), Some("abc"));
        assert_eq!(query_param("x=1&t=abc", "t"), Some("abc"));
        assert_eq!(query_param("tt=abc", "t"), None);
        let req = Request::builder()
            .header(header::COOKIE, "a=1; overlai_lan=tok")
            .header(header::COOKIE, "b=2")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(cookie(&req, "overlai_lan"), Some("tok"));
        assert_eq!(cookie(&req, "b"), Some("2"));
        assert_eq!(cookie(&req, "overlai"), None);
    }
}
