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
/// because the UI sets per-app colours with `style`; inline scripts are not.
const PAGE_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' data:; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; \
    form-action 'self'; frame-ancestors 'none'";

async fn page_headers(req: Request, next: Next) -> Response {
    // Vite fingerprints everything under /assets/, so those never change.
    let immutable = req.uri().path().starts_with("/assets/");
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
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
pub fn service<S: Clone + Send + Sync + 'static>(dir: &Path) -> Router<S> {
    let files = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
    Router::new()
        .fallback_service(files)
        .layer(middleware::from_fn(page_headers))
}
