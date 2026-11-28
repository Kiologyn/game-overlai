use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

/// The built UI. Release builds embed it into the exe; debug builds read it from disk, so a
/// `npm run build` is picked up without recompiling the core.
#[derive(Embed)]
#[folder = "../web/dist/"]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if let Some(file) = (!path.is_empty()).then(|| Assets::get(path)).flatten() {
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable" // hashed file names
        } else {
            "no-cache"
        };
        return respond(file.data.into_owned(), file.metadata.mimetype(), cache);
    }
    // SPA routes (/, /panel, ...) get index.html; missing files with an extension are 404.
    let looks_like_file = path.rsplit('/').next().is_some_and(|last| last.contains('.'));
    match (looks_like_file, Assets::get("index.html")) {
        (false, Some(index)) => respond(index.data.into_owned(), "text/html; charset=utf-8", "no-cache"),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn respond(body: Vec<u8>, mime: &str, cache: &'static str) -> Response {
    let mut res = body.into_response();
    let headers = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime) {
        headers.insert(header::CONTENT_TYPE, v);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}
