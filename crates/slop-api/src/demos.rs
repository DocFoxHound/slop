// SPDX-License-Identifier: AGPL-3.0-or-later
//! Browser demos (plan phase 15): a newly released game's web build, playable in the browser for
//! its first days, then deleted. A promotion for new games, with hard limits so it can't grow
//! into a hosting bill.
//!
//! - **Off** unless `SLOP_PLAY_URL` names a separate origin (`play.…`) for them. Demos never run
//!   on Slop's own origin, so a game's code can't reach anyone's Slop session; there they're
//!   only for playing: no network beyond their own files (content security policy).
//! - **The maker opts in** with a web build (a .zip with `index.html`, e.g. Godot's HTML5
//!   export) attached to a release on the app's repository, during the first `days` after the
//!   app's release (14 by default). Slop downloads it once, scans it like any release file, and
//!   unpacks it. Up to 100 MB zipped and 250 MB unpacked.
//! - **Limits:** `max_live` demos at once (10), and `daily_bytes` served per day across all of
//!   them (20 GB); past that, players are told to install the game instead.
//! - **Ending:** after its days, when the maker ends it, or when the app leaves the store, the
//!   files are deleted (hourly, cleanup.rs).

use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path as UrlPath, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/apps/{slug}/demo", get(status).post(start).delete(end))
}

/// Served at the play origin only (`lib.rs` keeps everything else off it).
pub fn play_routes() -> Router<AppState> {
    Router::new()
        .route("/play/{slug}/", get(serve_index))
        .route("/play/{slug}/{*path}", get(serve))
}

/// Whether a request is for the play origin.
pub fn is_play_host(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(play) = state.config.demos.play_url.as_ref() else {
        return false;
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let want = match play.port() {
        Some(p) => format!("{}:{p}", play.host_str().unwrap_or("")),
        None => play.host_str().unwrap_or("").to_string(),
    };
    host.eq_ignore_ascii_case(&want)
}

#[derive(Serialize, sqlx::FromRow)]
struct Demo {
    status: String,
    detail: Option<String>,
    scan_result: Option<String>,
    size_bytes: Option<i64>,
    plays: i32,
    #[serde(with = "time::serde::rfc3339")]
    ends_at: OffsetDateTime,
}

#[derive(Serialize)]
struct DemoStatus {
    /// Demos are hosted on this Slop.
    available: bool,
    /// The app can have one now: released within the last `days`.
    eligible: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    eligible_until: Option<OffsetDateTime>,
    demo: Option<Demo>,
    /// Where players play it, while it's live.
    play_url: Option<String>,
    days: i64,
}

/// The app's release date and whether it's public and listed.
async fn eligibility(state: &AppState, app_id: Uuid) -> ApiResult<Option<OffsetDateTime>> {
    let row: (String, String, String, OffsetDateTime) = sqlx::query_as(
        "select stage::text, status::text, visibility, stage_entered_at from apps where id = $1",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    let (stage, status, visibility, released_at) = row;
    let until = released_at + time::Duration::days(state.config.demos.days);
    Ok((stage == "released"
        && status == "listed"
        && visibility == "public"
        && until > OffsetDateTime::now_utc())
    .then_some(until))
}

pub(crate) fn play_link(state: &AppState, slug: &str) -> Option<String> {
    state
        .config
        .demos
        .play_url
        .as_ref()
        .map(|u| format!("{}/play/{slug}/", u.as_str().trim_end_matches('/')))
}

async fn status(
    State(state): State<AppState>,
    user: CurrentUser,
    UrlPath(slug): UrlPath<String>,
) -> ApiResult<Json<DemoStatus>> {
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let until = eligibility(&state, app_id).await?;
    let demo: Option<Demo> = sqlx::query_as(
        "select status, detail, scan_result, size_bytes, plays, ends_at from demos where app_id = $1",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let live = demo.as_ref().is_some_and(|d| d.status == "live");
    Ok(Json(DemoStatus {
        available: state.config.demos.play_url.is_some(),
        eligible: until.is_some(),
        eligible_until: until,
        play_url: live.then(|| play_link(&state, &slug)).flatten(),
        demo,
        days: state.config.demos.days,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    /// The web build: a .zip on a release of the app's repository.
    url: String,
}

async fn start(
    State(state): State<AppState>,
    user: CurrentUser,
    UrlPath(slug): UrlPath<String>,
    Json(b): Json<Start>,
) -> ApiResult<StatusCode> {
    if state.config.demos.play_url.is_none() {
        return Err(ApiError::Unavailable(
            "browser demos aren't offered on this Slop",
        ));
    }
    let (app_id, repo_url) = crate::updates::owned(&state, &slug, &user).await?;
    let Some(until) = eligibility(&state, app_id).await? else {
        return Err(ApiError::Conflict(format!(
            "demos are for the first {} days after a public app's release",
            state.config.demos.days
        )));
    };
    let (url, _) = crate::releases::validate_file_url(
        &repo_url,
        b.url.trim(),
        state.config.releases.allow_http_downloads,
    )?;
    let live: i64 = sqlx::query_scalar(
        "select count(*) from demos where status in ('fetching', 'live') and app_id <> $1",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    if live >= state.config.demos.max_live {
        return Err(ApiError::Conflict(
            "every demo slot is taken right now; try again in a few days".into(),
        ));
    }
    sqlx::query(
        "insert into demos (app_id, url, status, ends_at) values ($1, $2, 'fetching', $3)
         on conflict (app_id) do update set url = $2, status = 'fetching', detail = null, sha256 = null,
                size_bytes = null, scan_result = null, ends_at = $3",
    )
    .bind(app_id)
    .bind(&url)
    .bind(until)
    .execute(&state.db)
    .await?;
    let st = state.clone();
    tokio::spawn(async move {
        let result = fetch(&st, app_id, &slug, &url).await;
        let (status, detail) = match &result {
            Ok(()) => ("live", None),
            Err(e) => ("failed", Some(e.to_string())),
        };
        let updated = sqlx::query("update demos set status = $2, detail = coalesce($3, detail) where app_id = $1 and status = 'fetching'")
            .bind(app_id)
            .bind(status)
            .bind(detail)
            .execute(&st.db)
            .await;
        // Ended while it was being fetched: nothing will look at these files again, so they go
        // now.
        if result.is_ok() && updated.is_ok_and(|r| r.rows_affected() == 0) {
            let _ = std::fs::remove_dir_all(st.config.demos.dir.join(&slug));
        }
    });
    Ok(StatusCode::ACCEPTED)
}

/// Downloads, scans and unpacks a web build into `dir/<slug>`.
async fn fetch(state: &AppState, app_id: Uuid, slug: &str, url: &str) -> anyhow::Result<()> {
    let cfg = &state.config.demos;
    // The client for makers' addresses: public addresses only (lib.rs `PublicOnly`).
    let mut res = state.fetch_http.get(url).send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        bytes.extend_from_slice(&chunk);
        anyhow::ensure!(
            bytes.len() as u64 <= cfg.max_zip_bytes,
            "the web build is over 100 MB"
        );
    }
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let scratch = cfg.dir.join(format!(".fetch-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&scratch)?;
    let zip_path = scratch.join("web.zip");
    std::fs::write(&zip_path, &bytes)?;
    let verdict = state
        .config
        .releases
        .scanner
        .scan(&crate::scan::Target {
            path: &zip_path,
            file_name: "web.zip",
            sha256: &sha256,
        })
        .await;
    sqlx::query(
        "update demos set sha256 = $2, size_bytes = $3, scan_result = $4 where app_id = $1",
    )
    .bind(app_id)
    .bind(&sha256)
    .bind(bytes.len() as i64)
    .bind(verdict.result.as_str())
    .execute(&state.db)
    .await?;
    if !state.config.releases.scanner.approvable(verdict.result) {
        let _ = std::fs::remove_dir_all(&scratch);
        anyhow::bail!(
            "the scan didn't pass ({}): {}",
            verdict.result.as_str(),
            verdict.detail.unwrap_or_default()
        );
    }
    let unpacked = scratch.join("site");
    let result = unpack(&bytes, &unpacked, cfg.max_unpacked_bytes);
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(e);
    }
    let target = cfg.dir.join(slug);
    let old = cfg.dir.join(format!(".old-{}", Uuid::new_v4().simple()));
    if target.exists() {
        std::fs::rename(&target, &old)?;
    }
    std::fs::rename(&unpacked, &target)?;
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

/// Unpacks a web build: plain files inside it only, at most `limit` bytes, with an `index.html`
/// at the top (or inside one top folder, which is lifted out).
pub fn unpack(zip: &[u8], dest: &Path, limit: u64) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip))?;
    anyhow::ensure!(archive.len() <= 10_000, "the web build has too many files");
    let names: Vec<String> = archive
        .file_names()
        .map(|n| n.map(|c| c.into_owned()))
        .collect::<Result<_, _>>()?;
    // One top folder (an export inside "web/"): lift it out.
    let prefix = if names.iter().any(|n| n == "index.html") {
        String::new()
    } else {
        let tops: std::collections::BTreeSet<&str> =
            names.iter().filter_map(|n| n.split('/').next()).collect();
        match tops.iter().next() {
            Some(top)
                if tops.len() == 1 && names.iter().any(|n| *n == format!("{top}/index.html")) =>
            {
                format!("{top}/")
            }
            _ => anyhow::bail!("the web build needs an index.html at its top"),
        }
    };
    std::fs::create_dir_all(dest)?;
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        if !file.is_file() {
            continue;
        }
        let name = file.name()?.into_owned();
        let Some(rel) = name.strip_prefix(&prefix) else {
            continue;
        };
        let rel = PathBuf::from(rel);
        anyhow::ensure!(
            rel.components().all(|c| matches!(c, Component::Normal(_))),
            "a path outside the web build: {name}"
        );
        let mut data = Vec::new();
        file.by_ref().take(limit + 1).read_to_end(&mut data)?;
        total += data.len() as u64;
        anyhow::ensure!(total <= limit, "the web build is over 250 MB unpacked");
        let path = dest.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)?;
    }
    Ok(())
}

async fn end(
    State(state): State<AppState>,
    user: CurrentUser,
    UrlPath(slug): UrlPath<String>,
) -> ApiResult<StatusCode> {
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    sqlx::query(
        "update demos set status = 'ended', ends_at = least(ends_at, now()) where app_id = $1",
    )
    .bind(app_id)
    .execute(&state.db)
    .await?;
    let _ = std::fs::remove_dir_all(state.config.demos.dir.join(&slug));
    Ok(StatusCode::NO_CONTENT)
}

/// Ends demos past their days, or whose app left the store, and deletes their files.
pub async fn expire(state: &AppState) -> anyhow::Result<u64> {
    let ended: Vec<String> = sqlx::query_scalar(
        "update demos d set status = 'ended' from apps a
         where a.id = d.app_id and d.status in ('live', 'fetching', 'failed')
           and (d.ends_at < now() or a.status <> 'listed' or a.visibility <> 'public')
         returning a.slug",
    )
    .fetch_all(&state.db)
    .await?;
    for slug in &ended {
        let _ = std::fs::remove_dir_all(state.config.demos.dir.join(slug));
    }
    Ok(ended.len() as u64)
}

// ---------------------------------------------------------------------------------------------
// Serving, on the play origin

/// What a demo page may do: run its own scripts and WebAssembly, and reach nothing but its own
/// files.
const DEMO_CSP: &str = "default-src 'self' blob: data:; script-src 'self' 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' blob:; \
    style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; media-src 'self' blob: data:; \
    connect-src 'self' blob: data:; worker-src 'self' blob:; frame-ancestors 'none'; form-action 'none'; base-uri 'none'";

fn plain(status: StatusCode, text: &'static str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        text,
    )
        .into_response()
}

async fn serve_index(
    State(state): State<AppState>,
    UrlPath(slug): UrlPath<String>,
    req: Request,
) -> Response {
    serve_file(&state, &slug, "index.html", req).await
}

async fn serve(
    State(state): State<AppState>,
    UrlPath((slug, path)): UrlPath<(String, String)>,
    req: Request,
) -> Response {
    serve_file(&state, &slug, &path, req).await
}

async fn serve_file(state: &AppState, slug: &str, path: &str, req: Request) -> Response {
    if !is_play_host(state, req.headers()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let live: Option<Uuid> = sqlx::query_scalar(
        "select d.app_id from demos d join apps a on a.id = d.app_id
         where a.slug = $1 and d.status = 'live' and d.ends_at > now() and a.status = 'listed' and a.visibility = 'public'",
    )
    .bind(slug)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some(app_id) = live else {
        return plain(
            StatusCode::NOT_FOUND,
            "<!doctype html><title>Demo ended</title><p>This demo isn't available. Get the game on Slop.</p>",
        );
    };
    let rel = PathBuf::from(path);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let file = state.config.demos.dir.join(slug).join(&rel);
    let Ok(meta) = std::fs::metadata(&file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // The daily cap across all demos, counting what this request will really send: nothing
    // for HEAD, the asked-for slice for a range (security review, 2026-10-22: charging the
    // whole file for those let a few hundred tiny requests switch demos off for the day).
    let charged = if req.method() == axum::http::Method::HEAD {
        0
    } else {
        range_len(req.headers().get(axum::http::header::RANGE), meta.len()).unwrap_or(meta.len())
    };
    let today = OffsetDateTime::now_utc().date();
    let used: i64 = sqlx::query_scalar(
        "insert into demo_traffic (day, bytes) values ($1, $2)
         on conflict (day) do update set bytes = demo_traffic.bytes + $2 returning bytes",
    )
    .bind(today)
    .bind(charged as i64)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);
    if used as u64 > state.config.demos.daily_bytes {
        return plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "<!doctype html><title>Demos are busy</title><p>Demos have had a lot of players today. Install the game from Slop to play, or try again tomorrow.</p>",
        );
    }
    if path == "index.html" && req.method() == axum::http::Method::GET {
        let _ = sqlx::query("update demos set plays = plays + 1 where app_id = $1")
            .bind(app_id)
            .execute(&state.db)
            .await;
    }
    let res = tower_http::services::ServeFile::new(&file)
        .oneshot(req)
        .await
        .map(IntoResponse::into_response)
        .unwrap_or_else(|_| StatusCode::NOT_FOUND.into_response());
    let (mut parts, body) = res.into_parts();
    let h = &mut parts.headers;
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(DEMO_CSP),
    );
    h.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    // Godot's threads need SharedArrayBuffer, which needs these two.
    h.insert(
        "cross-origin-embedder-policy",
        HeaderValue::from_static("require-corp"),
    );
    h.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    Response::from_parts(parts, Body::new(body))
}

/// The bytes a single `Range: bytes=a-b` (or `a-`, `-n`) asks for, within a file of `len`.
fn range_len(header: Option<&axum::http::HeaderValue>, len: u64) -> Option<u64> {
    let spec = header?.to_str().ok()?.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => (len.saturating_sub(n.parse().ok()?), len.checked_sub(1)?),
        (a, "") => (a.parse().ok()?, len.checked_sub(1)?),
        (a, b) => (
            a.parse().ok()?,
            b.parse::<u64>().ok()?.min(len.checked_sub(1)?),
        ),
    };
    (start <= end).then(|| end - start + 1)
}

#[cfg(test)]
mod tests {
    use super::{range_len, unpack};
    use std::io::Write;

    #[test]
    fn ranges_count_what_they_send() {
        let h = |v: &str| axum::http::HeaderValue::from_str(v).unwrap();
        assert_eq!(range_len(Some(&h("bytes=0-0")), 1000), Some(1));
        assert_eq!(range_len(Some(&h("bytes=990-")), 1000), Some(10));
        assert_eq!(range_len(Some(&h("bytes=-100")), 1000), Some(100));
        assert_eq!(range_len(Some(&h("bytes=0-99999")), 1000), Some(1000));
        assert_eq!(range_len(None, 1000), None);
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn web_builds_unpack_safely() {
        let dir = std::env::temp_dir().join(format!("slop-demo-{}", rand::random::<u64>()));
        unpack(
            &zip_of(&[("index.html", b"<p>hi"), ("game.wasm", b"\0asm")]),
            &dir.join("a"),
            1000,
        )
        .unwrap();
        assert!(dir.join("a/game.wasm").exists());
        // One top folder is lifted out.
        unpack(
            &zip_of(&[("web/index.html", b"x"), ("web/a.js", b"y")]),
            &dir.join("b"),
            1000,
        )
        .unwrap();
        assert!(dir.join("b/index.html").exists());
        assert!(
            unpack(&zip_of(&[("game.js", b"x")]), &dir.join("c"), 1000).is_err(),
            "no index.html"
        );
        assert!(
            unpack(
                &zip_of(&[("index.html", b"x"), ("../evil.js", b"x")]),
                &dir.join("d"),
                1000
            )
            .is_err()
        );
        assert!(
            unpack(
                &zip_of(&[("index.html", &[0u8; 600]), ("b.bin", &[0u8; 600])]),
                &dir.join("e"),
                1000
            )
            .is_err(),
            "too big"
        );
    }
}
