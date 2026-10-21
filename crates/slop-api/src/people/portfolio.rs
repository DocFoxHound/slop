// SPDX-License-Identifier: AGPL-3.0-or-later
//! Creatives' portfolios, from a public GitHub repository (DESIGN §15.1).
//!
//! Like store-page pictures (media.rs), Slop doesn't host them. The creative lists their work in
//! `.slop/portfolio.json` in any public GitHub repository:
//!
//! ```json
//! {
//!   "avatar": "avatar.png",
//!   "banner": "banner.png",
//!   "works": [
//!     { "image": "works/knight.png", "title": "Knight sprite sheet", "skill": "pixel-art" },
//!     { "youtube": "https://youtu.be/…", "poster": "works/reel.png", "title": "Showreel" },
//!     { "audio": "tracks/theme.ogg", "poster": "covers/theme.png", "title": "Main theme" }
//!   ]
//! }
//! ```
//!
//! Slop reads it at one commit, checks every file once (pictures from their header, audio from
//! its signature, sizes), and links to the files at that commit. Every new set waits for review
//! (admin_web.rs, review.rs); a profile goes live with its first approved portfolio.

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{SKILLS, my_creative};
use crate::{
    AppState, admin,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    media::{self, Picture, Problem, Reader},
};

pub const MANIFEST_PATH: &str = ".slop/portfolio.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub const MAX_WORKS: usize = 30;
const MAX_AUDIO_BYTES: u64 = 15 * 1024 * 1024;
const MAX_TITLE: usize = 80;
/// Pixel art is often small; browsers scale it up.
const WORK_MIN: (u32, u32) = (32, 32);
const AVATAR_MIN: (u32, u32) = (128, 128);
const BANNER_MIN: (u32, u32) = (1200, 300);

macro_rules! problem {
    ($($arg:tt)*) => { return Err(Problem(format!($($arg)*))) };
}

/// A checked audio file, linked at the commit it was read at.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AudioFile {
    pub path: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    /// mp3, ogg, wav, flac or m4a.
    pub format: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Work {
    Image {
        #[serde(flatten)]
        picture: Picture,
        title: Option<String>,
        caption: Option<String>,
        skill: Option<String>,
    },
    Video {
        youtube_id: String,
        poster: Option<Picture>,
        title: Option<String>,
        caption: Option<String>,
        skill: Option<String>,
    },
    Audio {
        audio: AudioFile,
        poster: Option<Picture>,
        title: Option<String>,
        caption: Option<String>,
        skill: Option<String>,
    },
}

impl Work {
    /// `image:<path>`, `video:<youtube id>` or `audio:<path>`, for the creative's order.
    pub fn key(&self) -> String {
        match self {
            Work::Image { picture, .. } => format!("image:{}", picture.path),
            Work::Video { youtube_id, .. } => format!("video:{youtube_id}"),
            Work::Audio { audio, .. } => format!("audio:{}", audio.path),
        }
    }
}

pub fn arrange(works: &mut [Work], order: &[String]) {
    works.sort_by_key(|w| {
        let key = w.key();
        order.iter().position(|k| *k == key).unwrap_or(usize::MAX)
    });
}

#[derive(Serialize, sqlx::FromRow)]
pub struct PortfolioSet {
    pub id: Uuid,
    pub repo_url: String,
    pub commit_sha: String,
    #[sqlx(json(nullable))]
    pub avatar: Option<Picture>,
    #[sqlx(json(nullable))]
    pub banner: Option<Picture>,
    #[sqlx(json)]
    pub works: Vec<Work>,
    pub status: String,
    pub status_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub reviewed_at: Option<OffsetDateTime>,
}

pub const SET_COLUMNS: &str = "id, repo_url, commit_sha, avatar, banner, works, status, status_reason, created_at, reviewed_at";

pub async fn load_set(db: &sqlx::PgPool, id: Uuid) -> sqlx::Result<PortfolioSet> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {SET_COLUMNS} from portfolio_sets where id = $1"
    )))
    .bind(id)
    .fetch_one(db)
    .await
}

// ---------------------------------------------------------------------------------------------
// Reading .slop/portfolio.json

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    avatar: Option<String>,
    banner: Option<String>,
    #[serde(default)]
    works: Vec<ManifestWork>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestWork {
    image: Option<String>,
    youtube: Option<String>,
    audio: Option<String>,
    poster: Option<String>,
    title: Option<String>,
    caption: Option<String>,
    skill: Option<String>,
}

/// What an audio file is, from its first bytes. Nothing else of it is read.
pub fn audio_format(bytes: &[u8]) -> Option<&'static str> {
    let starts = |sig: &[u8]| bytes.starts_with(sig);
    if starts(b"ID3") || (bytes.len() > 1 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0) {
        Some("mp3")
    } else if starts(b"OggS") {
        Some("ogg")
    } else if starts(b"fLaC") {
        Some("flac")
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        Some("wav")
    } else if bytes.get(4..8) == Some(b"ftyp") {
        Some("m4a")
    } else {
        None
    }
}

fn clean_title(title: Option<&str>) -> Result<Option<String>, Problem> {
    let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if t.chars().count() > MAX_TITLE || t.chars().any(char::is_control) {
        problem!("titles are one line of up to {MAX_TITLE} characters: \"{t}\"");
    }
    Ok(Some(t.to_string()))
}

fn clean_skill(skill: Option<&str>) -> Result<Option<String>, Problem> {
    match skill.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) if SKILLS.iter().any(|(slug, _)| *slug == s) => Ok(Some(s.to_string())),
        Some(s) => problem!(
            "\"{s}\" isn't one of the skills: {}",
            SKILLS
                .iter()
                .map(|(slug, _)| *slug)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

async fn read_audio(state: &AppState, base: &url::Url, path: &str) -> Result<AudioFile, Problem> {
    let path = media::clean_file_path(
        path,
        &[".mp3", ".ogg", ".oga", ".opus", ".wav", ".flac", ".m4a"],
        "an audio file: use .mp3, .ogg, .wav, .flac or .m4a",
    )?;
    let url = base.join(&path)?;
    let bytes = match media::download(state, url.clone(), MAX_AUDIO_BYTES).await {
        Ok(Some(b)) => b,
        Ok(None) => problem!(".slop/{path} isn't in the repository at that commit"),
        Err(e) if e.to_string() == "too big" => problem!("{path} is over 15 MB"),
        Err(e) => problem!("couldn't download {path}: {e:#}"),
    };
    let Some(format) = audio_format(&bytes) else {
        problem!("{path} isn't an MP3, Ogg, WAV, FLAC or M4A file");
    };
    Ok(AudioFile {
        path,
        url: url.to_string(),
        sha256: hex::encode(Sha256::digest(&bytes)),
        bytes: bytes.len() as u64,
        format: format.to_string(),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    NoManifest,
    Unchanged,
    Pending(Uuid),
}

/// Reads the portfolio at the repository's newest commit and queues it if it's new.
pub async fn refresh(
    state: &AppState,
    creative_id: Uuid,
    repo_url: &str,
) -> Result<Outcome, Problem> {
    let result = read_and_store(state, creative_id, repo_url).await;
    let problem = result.as_ref().err().map(|p| p.0.clone());
    sqlx::query(
        "update creatives set portfolio_checked_at = now(), portfolio_problem = $2 where user_id = $1",
    )
    .bind(creative_id)
    .bind(problem)
    .execute(&state.db)
    .await
    .map_err(|e| Problem(e.to_string()))?;
    result
}

async fn read_and_store(
    state: &AppState,
    creative_id: Uuid,
    repo_url: &str,
) -> Result<Outcome, Problem> {
    let Some(repo_path) = slop_provenance::github_repo_path(repo_url) else {
        problem!(
            "portfolios come from public GitHub repositories, e.g. https://github.com/you/portfolio"
        );
    };
    let commit = media::head_commit(state, &repo_path).await?;
    let base = state
        .config
        .media
        .github_raw_base
        .join(&format!("{repo_path}/{commit}/.slop/"))?;
    let manifest =
        match media::download(state, base.join("portfolio.json")?, MAX_MANIFEST_BYTES).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(Outcome::NoManifest),
            Err(e) => problem!("couldn't download {MANIFEST_PATH}: {e:#}"),
        };
    let manifest: Manifest = serde_json::from_slice(&manifest)
        .map_err(|e| Problem(format!("{MANIFEST_PATH} isn't valid: {e}")))?;
    if manifest.works.is_empty() {
        problem!("{MANIFEST_PATH} needs at least one work");
    }
    if manifest.works.len() > MAX_WORKS {
        problem!(
            "{MANIFEST_PATH} lists {} works; the most is {MAX_WORKS}",
            manifest.works.len()
        );
    }

    let mut reader = Reader::new(state, base.clone());
    let avatar = match &manifest.avatar {
        Some(p) => Some(reader.picture(p, AVATAR_MIN).await?),
        None => None,
    };
    let banner = match &manifest.banner {
        Some(p) => Some(reader.picture(p, BANNER_MIN).await?),
        None => None,
    };
    let mut works = Vec::with_capacity(manifest.works.len());
    for (i, w) in manifest.works.iter().enumerate() {
        let n = i + 1;
        let title = clean_title(w.title.as_deref())?;
        let caption = media::clean_caption(w.caption.as_deref())?;
        let skill = clean_skill(w.skill.as_deref())?;
        let poster = match &w.poster {
            Some(p) => Some(reader.picture(p, WORK_MIN).await?),
            None => None,
        };
        works.push(match (&w.image, &w.youtube, &w.audio) {
            (Some(path), None, None) => {
                if poster.is_some() {
                    problem!("work {n}: \"poster\" is only for videos and audio");
                }
                Work::Image {
                    picture: reader.picture(path, WORK_MIN).await?,
                    title,
                    caption,
                    skill,
                }
            }
            (None, Some(link), None) => {
                let Some(youtube_id) = media::youtube_id(link) else {
                    problem!("work {n}: \"{link}\" isn't a YouTube video link");
                };
                Work::Video {
                    youtube_id,
                    poster,
                    title,
                    caption,
                    skill,
                }
            }
            (None, None, Some(path)) => Work::Audio {
                audio: read_audio(state, &base, path).await?,
                poster,
                title,
                caption,
                skill,
            },
            _ => problem!("work {n} needs exactly one of \"image\", \"youtube\" or \"audio\""),
        });
    }
    let mut keys: Vec<String> = works.iter().map(Work::key).collect();
    keys.sort();
    if let Some(dup) = keys.windows(2).find(|w| w[0] == w[1]) {
        problem!("{MANIFEST_PATH} lists {} twice", dup[0]);
    }

    let fingerprint = {
        let mut sorted: Vec<serde_json::Value> = works
            .iter()
            .map(|w| {
                let mut v = serde_json::to_value(w).unwrap_or_default();
                // Where it was read from doesn't change what it is.
                strip_urls(&mut v);
                v
            })
            .collect();
        sorted.sort_by_key(|v| v.to_string());
        let pic = |p: &Option<Picture>| p.as_ref().map(|p| json!([p.path, p.sha256]));
        let value = json!([pic(&avatar), pic(&banner), sorted]);
        hex::encode(Sha256::digest(value.to_string().as_bytes()))
    };

    let mut tx = state.db.begin().await?;
    sqlx::query("select 1 from creatives where user_id = $1 for update")
        .bind(creative_id)
        .execute(&mut *tx)
        .await?;
    let live: Option<String> = sqlx::query_scalar(
        "select ps.fingerprint from creatives c join portfolio_sets ps on ps.id = c.portfolio_set_id
         where c.user_id = $1",
    )
    .bind(creative_id)
    .fetch_optional(&mut *tx)
    .await?;
    let pending: Option<String> = sqlx::query_scalar(
        "select fingerprint from portfolio_sets where creative_id = $1 and status = 'pending'",
    )
    .bind(creative_id)
    .fetch_optional(&mut *tx)
    .await?;
    if pending.as_deref() == Some(fingerprint.as_str()) {
        return Ok(Outcome::Unchanged);
    }
    sqlx::query(
        "update portfolio_sets set status = 'superseded', status_reason = 'replaced by a newer portfolio'
         where creative_id = $1 and status = 'pending'",
    )
    .bind(creative_id)
    .execute(&mut *tx)
    .await?;
    if live.as_deref() == Some(fingerprint.as_str()) {
        tx.commit().await?;
        return Ok(Outcome::Unchanged);
    }
    let rejected: Option<String> = sqlx::query_scalar(
        "select status_reason from portfolio_sets
         where creative_id = $1 and status = 'rejected' and fingerprint = $2
         order by created_at desc limit 1",
    )
    .bind(creative_id)
    .bind(&fingerprint)
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    if let Some(reason) = rejected {
        tx.commit().await?;
        problem!("this is the portfolio that was turned down: {reason}");
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, avatar, banner, works, fingerprint, source)
         values ($1, $2, $3, $4, $5, $6, $7, $8, 'creative')",
    )
    .bind(id)
    .bind(creative_id)
    .bind(repo_url)
    .bind(&commit)
    .bind(avatar.map(sqlx::types::Json))
    .bind(banner.map(sqlx::types::Json))
    .bind(sqlx::types::Json(&works))
    .bind(&fingerprint)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Outcome::Pending(id))
}

fn strip_urls(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            map.remove("url");
            map.values_mut().for_each(strip_urls);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_urls),
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------
// Review

pub async fn approve_set(db: &sqlx::PgPool, id: Uuid, reviewer: &str) -> anyhow::Result<()> {
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "select ps.creative_id, ps.status, c.handle from portfolio_sets ps
         join creatives c on c.user_id = ps.creative_id where ps.id = $1 for update of ps",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (creative_id, status, handle) = row.ok_or_else(|| anyhow::anyhow!("no such portfolio"))?;
    anyhow::ensure!(
        status == "pending",
        "this portfolio isn't waiting for review ({status})"
    );
    sqlx::query(
        "update portfolio_sets set status = 'approved', reviewed_by = $2, reviewed_at = now() where id = $1",
    )
    .bind(id)
    .bind(reviewer)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update creatives set portfolio_set_id = $2, updated_at = now() where user_id = $1",
    )
    .bind(creative_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    admin::audit(
        &mut tx,
        reviewer,
        "portfolio.approve",
        &handle,
        json!({ "set": id }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn reject_set(
    db: &sqlx::PgPool,
    id: Uuid,
    reviewer: &str,
    reason: &str,
) -> anyhow::Result<()> {
    let reason = reason.trim();
    anyhow::ensure!(!reason.is_empty(), "say why, for the creative");
    let mut tx = db.begin().await?;
    let row: Option<(String, String)> = sqlx::query_as(
        "select ps.status, c.handle from portfolio_sets ps
         join creatives c on c.user_id = ps.creative_id where ps.id = $1 for update of ps",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, handle) = row.ok_or_else(|| anyhow::anyhow!("no such portfolio"))?;
    anyhow::ensure!(
        status == "pending",
        "this portfolio isn't waiting for review ({status})"
    );
    sqlx::query(
        "update portfolio_sets set status = 'rejected', status_reason = $3, reviewed_by = $2, reviewed_at = now()
         where id = $1",
    )
    .bind(id)
    .bind(reviewer)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    admin::audit(
        &mut tx,
        reviewer,
        "portfolio.reject",
        &handle,
        json!({ "set": id, "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// A portfolio waiting for review, with the one shown now (admin_web.rs, review.rs).
#[derive(Serialize)]
pub struct QueuedPortfolio {
    pub handle: String,
    pub name: String,
    pub ai_use: String,
    pub set: PortfolioSet,
    pub live: Option<PortfolioSet>,
    pub notes: Vec<crate::review::Note>,
}

pub async fn queued(state: &AppState) -> ApiResult<Vec<QueuedPortfolio>> {
    /// (set, handle, name, AI use, live set, order)
    type Row = (Uuid, String, String, String, Option<Uuid>, Vec<String>);
    let rows: Vec<Row> = sqlx::query_as(
        "select ps.id, c.handle, c.name, c.ai_use, c.portfolio_set_id, c.portfolio_order
         from portfolio_sets ps join creatives c on c.user_id = ps.creative_id
         where ps.status = 'pending' order by ps.created_at",
    )
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, handle, name, ai_use, live, order) in rows {
        let mut set = load_set(&state.db, id).await?;
        arrange(&mut set.works, &order);
        let live = match live {
            Some(l) => Some(load_set(&state.db, l).await?),
            None => None,
        };
        out.push(QueuedPortfolio {
            handle,
            name,
            ai_use,
            set,
            live,
            notes: crate::review::notes(&state.db, "portfolio", id).await?,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// The creative's view

#[derive(Serialize)]
pub struct Overview {
    repo_url: Option<String>,
    live: Option<PortfolioSet>,
    pending: Option<PortfolioSet>,
    rejected: Option<PortfolioSet>,
    #[serde(with = "time::serde::rfc3339::option")]
    checked_at: Option<OffsetDateTime>,
    problem: Option<String>,
    order: Vec<String>,
    manifest_path: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<&'static str>,
}

async fn load_overview(state: &AppState, creative_id: Uuid) -> ApiResult<Overview> {
    type Row = (
        Option<String>,
        Option<Uuid>,
        Option<OffsetDateTime>,
        Option<String>,
        Vec<String>,
    );
    let (repo_url, live, checked_at, problem, order): Row = sqlx::query_as(
        "select portfolio_repo, portfolio_set_id, portfolio_checked_at, portfolio_problem, portfolio_order
         from creatives where user_id = $1",
    )
    .bind(creative_id)
    .fetch_one(&state.db)
    .await?;
    let sorted = |set: Option<PortfolioSet>| {
        set.map(|mut s| {
            arrange(&mut s.works, &order);
            s
        })
    };
    let newest = |status: &'static str| {
        sqlx::query_as::<_, PortfolioSet>(sqlx::AssertSqlSafe(format!(
            "select {SET_COLUMNS} from portfolio_sets where creative_id = $1 and status = $2
             order by created_at desc limit 1"
        )))
        .bind(creative_id)
        .bind(status)
        .fetch_optional(&state.db)
    };
    let live = sorted(match live {
        Some(id) => Some(load_set(&state.db, id).await?),
        None => None,
    });
    let pending = sorted(newest("pending").await?);
    let rejected = newest("rejected")
        .await?
        .filter(|r| pending.is_none() && live.as_ref().is_none_or(|l| l.created_at < r.created_at));
    Ok(Overview {
        repo_url,
        live,
        pending,
        rejected,
        checked_at,
        problem,
        order,
        manifest_path: MANIFEST_PATH,
        result: None,
    })
}

pub async fn overview(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Overview>> {
    let me = my_creative(&state, &user).await?;
    Ok(Json(load_overview(&state, me).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckBody {
    /// Sets the repository first, when given.
    repo_url: Option<String>,
}

/// "Check my portfolio now".
pub async fn check(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<CheckBody>,
) -> ApiResult<Json<Overview>> {
    let me = my_creative(&state, &user).await?;
    if let Some(repo) = body.repo_url.as_deref().map(str::trim) {
        if slop_provenance::github_repo_path(repo).is_none() {
            return Err(ApiError::invalid(
                "repo_url",
                "use a public GitHub repository link, e.g. https://github.com/you/portfolio",
            ));
        }
        sqlx::query(
            "update creatives set portfolio_repo = $2, updated_at = now() where user_id = $1",
        )
        .bind(me)
        .bind(repo)
        .execute(&state.db)
        .await?;
    }
    let repo: Option<String> =
        sqlx::query_scalar("select portfolio_repo from creatives where user_id = $1")
            .bind(me)
            .fetch_one(&state.db)
            .await?;
    let Some(repo) = repo else {
        return Err(ApiError::invalid(
            "repo_url",
            "add the repository your portfolio is in",
        ));
    };
    let result = match refresh(&state, me, &repo).await {
        Ok(Outcome::Pending(_)) => "pending",
        Ok(Outcome::Unchanged) => "unchanged",
        Ok(Outcome::NoManifest) => "none",
        Err(_) => "problem",
    };
    let mut overview = load_overview(&state, me).await?;
    overview.result = Some(result);
    Ok(Json(overview))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Order {
    keys: Vec<String>,
}

/// The creative puts their works in order. Not reviewed: it only moves reviewed works.
pub async fn set_order(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(order): Json<Order>,
) -> ApiResult<Json<Overview>> {
    let me = my_creative(&state, &user).await?;
    if order.keys.len() > MAX_WORKS * 2
        || order.keys.iter().any(|k| {
            k.len() > 220
                || !(k.starts_with("image:") || k.starts_with("video:") || k.starts_with("audio:"))
        })
    {
        return Err(ApiError::invalid("keys", "that isn't a portfolio order"));
    }
    sqlx::query("update creatives set portfolio_order = $2, updated_at = now() where user_id = $1")
        .bind(me)
        .bind(&order.keys)
        .execute(&state.db)
        .await?;
    Ok(Json(load_overview(&state, me).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_is_recognised_from_its_signature() {
        assert_eq!(audio_format(b"ID3\x04\0\0"), Some("mp3"));
        assert_eq!(audio_format(&[0xFF, 0xFB, 0x90, 0x00]), Some("mp3"));
        assert_eq!(audio_format(b"OggS\0\x02"), Some("ogg"));
        assert_eq!(audio_format(b"fLaC\0\0"), Some("flac"));
        assert_eq!(audio_format(b"RIFF\x24\0\0\0WAVEfmt "), Some("wav"));
        assert_eq!(audio_format(b"\0\0\0\x20ftypM4A "), Some("m4a"));
        assert_eq!(audio_format(b"<html>"), None);
        assert_eq!(audio_format(b"RIFF\x24\0\0\0AVI "), None);
    }
}
