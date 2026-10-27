// SPDX-License-Identifier: AGPL-3.0-or-later
//! Serving the built website (`web/dist`) from the API, when `SLOP_WEB_DIR` is set.
//!
//! The site and the API then share an origin, which the session cookie and the CSRF checks
//! need. Pages get a Content-Security-Policy that allows only SLAPorSLOP's own scripts and styles
//! (SECURITY §8.1); API responses keep the stricter one set in `router`.

use std::path::Path;

use axum::{
    extract::Request,
    http::{HeaderValue, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::Router,
};
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

/// Pages may load only SLAPorSLOP's own scripts, styles, images and API. Inline styles are allowed
/// because the UI sets per-app colours with `style`; inline scripts are not. Store-page pictures
/// come from makers' GitHub repositories (media.rs). The one frame is
/// YouTube's privacy-enhanced player, loaded when someone plays a store-page video. Composers'
/// portfolio tracks also play from GitHub (people/portfolio.rs).
const PAGE_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' data: https://raw.githubusercontent.com; media-src https://raw.githubusercontent.com; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; \
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

/// Files a page route never ends in: a missing one is a plain 404, not the website, so a tool
/// (or an AI agent) reading `/portfolio.md` from a server without it learns that it isn't there,
/// instead of getting an empty page that only fills in with JavaScript.
const DOCUMENTS: &[&str] = &["md", "txt", "json", "xml", "csv", "yaml", "yml", "pdf"];

fn is_document(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or("");
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && DOCUMENTS.contains(&ext.to_ascii_lowercase().as_str())
    })
}

/// Files from `dir`, with `index.html` for any other path (the UI routes in the browser), except
/// documents (`DOCUMENTS`), which are a 404 when missing. `/publish/guide` gets `guide.html`, the
/// same page with the guide already in it, so tools that don't run JavaScript can read it.
pub fn service<S: Clone + Send + Sync + 'static>(dir: &Path) -> Router<S> {
    let index = dir.join("index.html");
    let pages = ServeDir::new(dir).fallback(ServeFile::new(&index));
    let documents = ServeDir::new(dir);
    let guide = dir.join("guide.html");
    let guide = if guide.exists() { guide } else { index };
    Router::new()
        .route_service("/publish/guide", ServeFile::new(guide))
        .fallback(move |req: Request| async move {
            if is_document(req.uri().path()) {
                documents.oneshot(req).await.into_response()
            } else {
                pages.oneshot(req).await.into_response()
            }
        })
        .layer(middleware::from_fn(page_headers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_are_told_apart_from_page_routes() {
        for path in ["/portfolio.md", "/llms.txt", "/x/data.JSON", "/sitemap.xml"] {
            assert!(is_document(path), "{path}");
        }
        for path in [
            "/",
            "/people/me",
            "/app/castle-rock",
            "/library",
            "/u/0193",
            "/.md",
            "/publish/guide",
        ] {
            assert!(!is_document(path), "{path}");
        }
    }

    #[tokio::test]
    async fn a_missing_document_is_a_404_and_a_page_route_is_the_website() {
        let dir = std::env::temp_dir().join(format!("slop-web-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("index.html"),
            "<!doctype html><title>SLAPorSLOP</title>",
        )
        .unwrap();
        std::fs::write(dir.join("publishing.md"), "# How to publish").unwrap();
        let app: Router = service(&dir);
        let get = |path: &str| {
            let app = app.clone();
            let req = Request::builder()
                .uri(path)
                .body(axum::body::Body::empty())
                .unwrap();
            async move { app.oneshot(req).await.unwrap() }
        };
        assert_eq!(get("/publishing.md").await.status(), 200);
        assert_eq!(get("/portfolio.md").await.status(), 404);
        let page = get("/people/me").await;
        assert_eq!(page.status(), 200);
        let body = axum::body::to_bytes(page.into_body(), 1 << 16)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("<title>SLAPorSLOP</title>"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
