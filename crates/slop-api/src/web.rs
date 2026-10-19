//! Serving the built website (`web/dist`) from the API, when `SLOP_WEB_DIR` is set.
//!
//! The site and the API then share an origin, which the session cookie and the CSRF checks
//! need. Pages get a Content-Security-Policy that allows only Slop's own scripts and styles
//! (SECURITY §8.1); API responses keep the stricter one set in `router`.

use std::path::Path;

use axum::{
    extract::Request,
    http::{HeaderValue, header},
    middleware::{self, Next},
    response::Response,
    routing::Router,
};
use tower_http::services::{ServeDir, ServeFile};

/// Pages may load only Slop's own scripts, styles, images and API. Inline styles are allowed
/// because the UI sets per-app colours with `style`; inline scripts are not. The one frame is
/// YouTube's privacy-enhanced player, loaded when someone plays a store-page video.
const PAGE_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' data:; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; \
    frame-src https://www.youtube-nocookie.com; form-action 'self'; frame-ancestors 'none'";

async fn page_headers(req: Request, next: Next) -> Response {
    // Vite fingerprints everything under /assets/, so those never change.
    let immutable = req.uri().path().starts_with("/assets/");
    // Plain-text documents for tools and AI agents (publishing.md, llms.txt) are UTF-8.
    let path = req.uri().path();
    let text_type = if path.ends_with(".md") {
        Some("text/markdown; charset=utf-8")
    } else if path.ends_with(".txt") {
        Some("text/plain; charset=utf-8")
    } else {
        None
    };
    let mut res = next.run(req).await;
    let ok = res.status().is_success();
    let headers = res.headers_mut();
    if let (Some(kind), true) = (text_type, ok) {
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    }
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(PAGE_CSP),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if immutable {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        }),
    );
    res
}

/// Files from `dir`, with `index.html` for any other path (the UI routes in the browser).
/// `/publish/guide` gets `guide.html`, the same page with the guide already in it, so tools
/// that don't run JavaScript can read it.
pub fn service<S: Clone + Send + Sync + 'static>(dir: &Path) -> Router<S> {
    let index = dir.join("index.html");
    let files = ServeDir::new(dir).fallback(ServeFile::new(&index));
    let guide = dir.join("guide.html");
    let guide = if guide.exists() { guide } else { index };
    Router::new()
        .route_service("/publish/guide", ServeFile::new(guide))
        .fallback_service(files)
        .layer(middleware::from_fn(page_headers))
}
