// SPDX-License-Identifier: AGPL-3.0-or-later
//! SLAPorSLOP's scanning server (`slop-api scan-server`, deploy/README.md "Scanning server"): a
//! separate machine that scans what makers send, so nothing they sent is opened on the machine
//! with the website and the database (owner decision 2026-11-18). Production's API sends it each
//! file it would have scanned itself (`SLOP_SCANNER=remote`, scan.rs `Remote`), the fingerprints
//! to check against new signatures, and repository archives for the source check. It runs the
//! same `slop-scan` command a local scanner does: in a container with no network, a read-only
//! root and the file on stdin.
//!
//! It keeps nothing: a job's file is deleted when its scan ends, and the answer an hour later.
//! It has no database and no secrets but the token production sends.
//!
//! A job is sent with `POST /v1/jobs?kind=file&name=<file name>` (or `kind=hashes`, `kind=source`)
//! with the file as the body, and answers `{"id"}`. `GET /v1/jobs/<id>` says `queued`, `running`,
//! `done` (with the scanner's JSON as `output`) or `failed` (with an `error`). Scans run a few at a
//! time (`SLOP_SCAN_JOBS`); the rest wait their turn, up to `MAX_WAITING`, after which it answers
//! 503 and production tries again later.

use std::{
    collections::HashMap,
    ffi::OsStr,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{io::AsyncWriteExt, sync::Semaphore};
use uuid::Uuid;

/// Jobs waiting for their turn, at most; more are turned away (503) to be sent again later.
const MAX_WAITING: usize = 200;
/// How long a finished job's answer is kept for production to collect.
const KEEP_ANSWERS: Duration = Duration::from_secs(60 * 60);
/// The longest a file's scan may take (the container stops itself at 25 minutes).
const SCAN_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// The longest checking fingerprints may take.
const MATCH_TIMEOUT: Duration = Duration::from_secs(20 * 60);

pub struct Settings {
    /// Where it listens: this machine only (127.0.0.1:8095); Caddy lets only production in.
    pub listen: SocketAddr,
    /// The token production sends (SLOP_SCAN_TOKEN on both machines).
    pub token: String,
    /// The scanner command (deploy/scanner/slop-scan).
    pub command: String,
    /// Scans at once (each container may use 4 GB).
    pub jobs: usize,
    /// The largest file taken: the API's SLOP_MAX_FILE_BYTES.
    pub max_bytes: u64,
    /// Where files wait for their scan (deleted after it).
    pub dir: PathBuf,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        let var =
            |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
        let token = var("SLOP_SCAN_TOKEN", "").trim().to_string();
        anyhow::ensure!(
            token.len() >= 32,
            "SLOP_SCAN_TOKEN must be at least 32 characters (openssl rand -hex 32)"
        );
        let command = var("SLOP_SCAN_COMMAND", "slop-scan").trim().to_string();
        anyhow::ensure!(!command.is_empty(), "SLOP_SCAN_COMMAND is empty");
        let jobs: usize = var("SLOP_SCAN_JOBS", "3").parse()?;
        anyhow::ensure!(jobs >= 1, "SLOP_SCAN_JOBS must be at least 1");
        Ok(Self {
            listen: var("SLOP_SCAN_LISTEN", "127.0.0.1:8095").parse()?,
            token,
            command,
            jobs,
            max_bytes: var("SLOP_MAX_FILE_BYTES", "2147483648").parse()?,
            dir: std::env::temp_dir(),
        })
    }
}

#[derive(Clone)]
enum JobState {
    Queued,
    Running,
    Done(Value),
    Failed(String),
}

struct Job {
    state: JobState,
    finished: Option<Instant>,
}

struct Inner {
    settings: Settings,
    token_hash: [u8; 32],
    jobs: Mutex<HashMap<Uuid, Job>>,
    permits: Semaphore,
}

#[derive(Clone)]
struct Server(Arc<Inner>);

pub fn router(settings: Settings) -> Router {
    let token_hash = Sha256::digest(settings.token.as_bytes()).into();
    let permits = Semaphore::new(settings.jobs);
    Router::new()
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/v1/jobs", post(start))
        .route("/v1/jobs/{id}", get(progress))
        // Files are streamed to disk with their own limit (`max_bytes`).
        .layer(DefaultBodyLimit::disable())
        .with_state(Server(Arc::new(Inner {
            settings,
            token_hash,
            jobs: Mutex::new(HashMap::new()),
            permits,
        })))
}

pub async fn run(settings: Settings) -> anyhow::Result<()> {
    let listen = settings.listen;
    // Files left by a server that stopped in the middle of a scan.
    if let Ok(entries) = std::fs::read_dir(&settings.dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("slop-job-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, "scanning server listening");
    axum::serve(listener, router(settings)).await?;
    Ok(())
}

impl Server {
    /// The token compared by its hash, so how long the comparison takes says nothing about it.
    fn allowed(&self, headers: &HeaderMap) -> bool {
        let given = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        let hash: [u8; 32] = Sha256::digest(given.as_bytes()).into();
        hash.iter()
            .zip(self.0.token_hash.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }

    fn jobs(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Job>> {
        self.0.jobs.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, id: Uuid, state: JobState) {
        let finished = matches!(state, JobState::Done(_) | JobState::Failed(_)).then(Instant::now);
        self.jobs().insert(id, Job { state, finished });
    }
}

fn refuse(code: StatusCode, why: &str) -> Response {
    (code, Json(json!({ "error": why }))).into_response()
}

#[derive(Deserialize)]
struct Start {
    kind: String,
    name: Option<String>,
}

/// Only the extension matters to the scanner; plain characters, not too long.
fn plain_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .rev()
        .take(100)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.starts_with('.') {
        out.insert_str(0, "file");
    }
    out
}

async fn start(
    State(server): State<Server>,
    Query(q): Query<Start>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if !server.allowed(&headers) {
        return refuse(StatusCode::UNAUTHORIZED, "wrong token");
    }
    let name = match (q.kind.as_str(), q.name.as_deref()) {
        ("file", Some(name)) => Some(plain_name(name)),
        ("file", None) => return refuse(StatusCode::BAD_REQUEST, "a file needs its name"),
        ("hashes" | "source", _) => None,
        _ => return refuse(StatusCode::BAD_REQUEST, "kind is file, hashes or source"),
    };
    {
        let mut jobs = server.jobs();
        jobs.retain(|_, j| j.finished.is_none_or(|t| t.elapsed() < KEEP_ANSWERS));
        let waiting = jobs.values().filter(|j| j.finished.is_none()).count();
        if waiting >= MAX_WAITING {
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "busy; send it again later");
        }
    }
    let id = Uuid::now_v7();
    let path = server.0.settings.dir.join(format!("slop-job-{id}"));
    let received = async {
        let mut file = tokio::fs::File::create(&path).await?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            size += chunk.len() as u64;
            if size > server.0.settings.max_bytes {
                return Ok::<_, anyhow::Error>(Err(StatusCode::PAYLOAD_TOO_LARGE));
            }
            hash.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(Ok((hex::encode(hash.finalize()), size)))
    }
    .await;
    let (sha256, size) = match received {
        Ok(Ok(got)) => got,
        Ok(Err(code)) => {
            let _ = tokio::fs::remove_file(&path).await;
            return refuse(code, "the file is over the size limit");
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            // Cut off part-way: production sends it again.
            return refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                &format!("receiving it: {e}"),
            );
        }
    };
    if let Some(expected) = headers.get("x-slop-sha256").and_then(|v| v.to_str().ok())
        && expected != sha256
    {
        let _ = tokio::fs::remove_file(&path).await;
        // Something changed it on the way: it's sent again.
        return refuse(
            StatusCode::CONFLICT,
            "the file arrived different from what was sent",
        );
    }
    server.set(id, JobState::Queued);
    let worker = server.clone();
    let kind = q.kind.clone();
    tokio::spawn(async move {
        let Ok(_permit) = worker.0.permits.acquire().await else {
            return;
        };
        worker.set(id, JobState::Running);
        let began = Instant::now();
        let command = worker.0.settings.command.as_str();
        let path_arg = path.as_os_str();
        let out = match kind.as_str() {
            "file" => {
                let name = name.unwrap_or_default();
                crate::scan::run_command(
                    command,
                    &[path_arg, OsStr::new(&name)],
                    None,
                    SCAN_TIMEOUT,
                )
                .await
            }
            "hashes" => {
                crate::scan::run_command(
                    command,
                    &[OsStr::new("--match-hashes")],
                    Some(&path),
                    MATCH_TIMEOUT,
                )
                .await
            }
            _ => {
                crate::scan::run_command(
                    command,
                    &[OsStr::new("--source"), path_arg],
                    None,
                    SCAN_TIMEOUT,
                )
                .await
            }
        };
        let _ = tokio::fs::remove_file(&path).await;
        match out {
            Ok(output) => {
                tracing::info!(%id, kind, size, secs = began.elapsed().as_secs(), result = output["result"].as_str().unwrap_or(""), "scanned");
                worker.set(id, JobState::Done(output));
            }
            Err(e) => {
                tracing::error!(%id, kind, size, error = %e, "scan failed");
                worker.set(id, JobState::Failed(format!("{e:#}")));
            }
        }
    });
    (StatusCode::ACCEPTED, Json(json!({ "id": id }))).into_response()
}

async fn progress(
    State(server): State<Server>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    if !server.allowed(&headers) {
        return refuse(StatusCode::UNAUTHORIZED, "wrong token");
    }
    let state = server.jobs().get(&id).map(|j| j.state.clone());
    match state {
        None => refuse(StatusCode::NOT_FOUND, "no such job"),
        Some(JobState::Queued) => Json(json!({ "state": "queued" })).into_response(),
        Some(JobState::Running) => Json(json!({ "state": "running" })).into_response(),
        Some(JobState::Done(output)) => {
            Json(json!({ "state": "done", "output": output })).into_response()
        }
        Some(JobState::Failed(error)) => {
            Json(json!({ "state": "failed", "error": error })).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_kept_plain() {
        assert_eq!(plain_name("My Game (x64).exe"), "My_Game__x64_.exe");
        assert_eq!(plain_name(".hidden"), "file.hidden");
        assert_eq!(plain_name(""), "file");
        assert_eq!(plain_name(&format!("{}.zip", "a".repeat(300))).len(), 100);
        assert!(plain_name(&format!("{}.zip", "a".repeat(300))).ends_with(".zip"));
    }
}
