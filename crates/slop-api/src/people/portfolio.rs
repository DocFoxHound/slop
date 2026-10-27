// SPDX-License-Identifier: AGPL-3.0-or-later
//! Creatives' portfolios, from a public GitHub repository (DESIGN §15.1).
//!
//! Like store-page pictures (media.rs), SLAPorSLOP doesn't host them. The creative lists their work in
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
//! SLAPorSLOP reads it at one commit, checks every file once (pictures from their header, audio from
//! its signature, sizes), and links to the files at that commit. Every new set waits for review
//! (admin_web.rs, review.rs); a profile goes live with its first approved portfolio.
//!
//! The same commit's `.slop/page/` (the creative's own page design, design.rs) is read with it
//! and is part of the set: reviewed with the works, and shown only once approved.

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    SKILLS,
    design::{self, Design},
    my_creative,
};
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
    /// The creative's own page design, if the repository has one at that commit.
    #[sqlx(json(nullable))]
    pub design: Option<Design>,
    pub status: String,
    pub status_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub reviewed_at: Option<OffsetDateTime>,
}

pub const SET_COLUMNS: &str = "id, repo_url, commit_sha, avatar, banner, works, design, status, status_reason, created_at, reviewed_at";

pub async fn load_set(db: &sqlx::PgPool, id: Uuid) -> sqlx::Result<PortfolioSet> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {SET_COLUMNS} from portfolio_sets where id = $1"
    )))
    .bind(id)
    .fetch_one(db)
    .await
}

/// The creative's portfolio that's newer than the approved one: the one waiting for review, or
/// else the newest turned down since the approved one (none approved: the newest turned down).
/// Their own page previews it (mod.rs `page`); nobody else ever sees it before it's approved.
pub async fn newer_set(db: &sqlx::PgPool, creative_id: Uuid) -> sqlx::Result<Option<PortfolioSet>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {SET_COLUMNS} from portfolio_sets
         where creative_id = $1
           and (status = 'pending'
                or (status = 'rejected'
                    and created_at > coalesce((select l.created_at from creatives c
                                                 join portfolio_sets l on l.id = c.portfolio_set_id
                                               where c.user_id = $1), '-infinity')))
         order by status = 'pending' desc, created_at desc limit 1"
    )))
    .bind(creative_id)
    .fetch_optional(db)
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

/// One line of up to `max` characters, or nothing. `field` names it in the problem.
fn one_line(field: &str, value: Option<&str>, max: usize) -> Result<Option<String>, Problem> {
    let Some(t) = value.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    let n = t.chars().count();
    if n > max {
        problem!("\"{field}\" can be up to {max} characters; this one has {n}");
    }
    if t.chars().any(char::is_control) {
        problem!("\"{field}\" has to be one line, without line breaks or tabs");
    }
    Ok(Some(t.to_string()))
}

fn clean_skill(skill: Option<&str>) -> Result<Option<String>, Problem> {
    match skill.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) if SKILLS.iter().any(|(slug, _)| *slug == s) => Ok(Some(s.to_string())),
        Some(s) => problem!(
            "\"{s}\" isn't one of the skills; use one of {}",
            SKILLS
                .iter()
                .map(|(slug, _)| *slug)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Puts what a problem is about in front of it: `work 3 "image": …`.
fn about<T>(what: &str, checked: Result<T, Problem>) -> Result<T, Problem> {
    checked.map_err(|Problem(p)| Problem(format!("{what}: {p}")))
}

/// Shown with "isn't a YouTube video link", so the fix is plain.
const YOUTUBE_FORMS: &str = "use a video's own link, like https://youtu.be/VIDEO_ID or https://www.youtube.com/watch?v=VIDEO_ID (Shorts, /embed/ and /live/ links work too)";

/// At most this many problems are listed at once; the rest are counted.
const MAX_PROBLEMS: usize = 12;

pub(super) async fn read_audio(
    state: &AppState,
    base: &url::Url,
    path: &str,
) -> Result<AudioFile, Problem> {
    let path = media::clean_file_path(
        path,
        &[".mp3", ".ogg", ".oga", ".opus", ".wav", ".flac", ".m4a"],
        "an audio file: use .mp3, .ogg, .wav, .flac or .m4a",
    )?;
    let url = base.join(&path)?;
    let bytes = match media::download(state, url.clone(), MAX_AUDIO_BYTES).await {
        Ok(Some(b)) => b,
        Ok(None) => problem!(".slop/{path} isn't in the repository at that commit"),
        Err(e) if e.to_string() == "too big" => {
            problem!(
                "{path} is over {} MB, the most for a track",
                MAX_AUDIO_BYTES / (1024 * 1024)
            )
        }
        Err(e) => problem!("couldn't download {path}: {e:#}"),
    };
    let Some(format) = audio_format(&bytes) else {
        problem!(
            "{path} isn't an MP3, Ogg, WAV, FLAC or M4A file (SLAPorSLOP reads its first bytes, not its name)"
        );
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

/// A portfolio at one commit, every file read and checked.
pub(crate) struct Checked {
    pub avatar: Option<Picture>,
    pub banner: Option<Picture>,
    pub works: Vec<Work>,
    /// The page design and its fingerprint, if the repository has one.
    pub design: Option<(Design, String)>,
}

/// Reads `.slop/portfolio.json` in `repo_path` (`owner/name`) at `commit`, with the page design,
/// and checks every file, writing nothing: storing a new set is `read_and_store`'s, and the dry
/// run (checks.rs) only reports, without the look-alike hashes only a stored set needs. None
/// when there's no portfolio.json. Otherwise every problem, each naming where it is; the design
/// is read once the works are fine.
pub(crate) async fn read_and_check(
    state: &AppState,
    repo_path: &str,
    commit: &str,
    dry_run: bool,
) -> Result<Option<Checked>, Vec<String>> {
    let one = |p: Problem| vec![p.0];
    let base = state
        .config
        .media
        .github_raw_base
        .join(&format!("{repo_path}/{commit}/.slop/"))
        .map_err(|e| one(e.into()))?;
    let manifest_url = base.join("portfolio.json").map_err(|e| one(e.into()))?;
    let manifest = match media::download(state, manifest_url, MAX_MANIFEST_BYTES).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(None),
        Err(e) => return Err(vec![format!("couldn't download {MANIFEST_PATH}: {e:#}")]),
    };
    let manifest: Manifest = serde_json::from_slice(&manifest).map_err(|e| {
        vec![format!(
            "{MANIFEST_PATH} isn't valid: {e}. It's one JSON object with \"avatar\", \"banner\" and \"works\" (a list); each work has one of \"image\", \"youtube\" or \"audio\", and optionally \"poster\", \"title\", \"caption\" and \"skill\""
        )]
    })?;
    if manifest.works.is_empty() {
        return Err(vec![format!(
            "{MANIFEST_PATH} needs at least one work: add a \"works\" list, e.g. \"works\": [{{ \"image\": \"works/first.png\" }}]"
        )]);
    }
    if manifest.works.len() > MAX_WORKS {
        return Err(vec![format!(
            "{MANIFEST_PATH} lists {} works; the most is {MAX_WORKS}",
            manifest.works.len()
        )]);
    }

    // Every problem is listed, each naming where it is, so one check finds them all.
    let mut problems: Vec<String> = Vec::new();
    let mut note = |r: Result<(), Problem>| {
        if let Err(Problem(p)) = r {
            problems.push(p);
        }
    };
    let mut reader = Reader::new(state, base.clone());
    if dry_run {
        reader = reader.without_hashes();
    }
    let mut avatar = None;
    if let Some(p) = &manifest.avatar {
        note(about("\"avatar\"", reader.picture(p, AVATAR_MIN).await).map(|p| avatar = Some(p)));
    }
    let mut banner = None;
    if let Some(p) = &manifest.banner {
        note(about("\"banner\"", reader.picture(p, BANNER_MIN).await).map(|p| banner = Some(p)));
    }
    let mut works = Vec::with_capacity(manifest.works.len());
    // Each work's key and number, to say which one repeats which.
    let mut first_listed: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (i, w) in manifest.works.iter().enumerate() {
        let n = i + 1;
        let at = |field: &str| format!("work {n} \"{field}\"");
        let checked = async {
            let title = about(
                &format!("work {n}"),
                one_line("title", w.title.as_deref(), MAX_TITLE),
            )?;
            let caption = about(
                &format!("work {n}"),
                one_line("caption", w.caption.as_deref(), media::MAX_CAPTION),
            )?;
            let skill = about(&at("skill"), clean_skill(w.skill.as_deref()))?;
            let given: Vec<&str> = [
                ("image", w.image.is_some()),
                ("youtube", w.youtube.is_some()),
                ("audio", w.audio.is_some()),
            ]
            .into_iter()
            .filter_map(|(k, set)| set.then_some(k))
            .collect();
            if given.len() != 1 {
                problem!(
                    "work {n} needs exactly one of \"image\", \"youtube\" or \"audio\"; it has {}",
                    if given.is_empty() {
                        "none of them".to_string()
                    } else {
                        given
                            .iter()
                            .map(|k| format!("\"{k}\""))
                            .collect::<Vec<_>>()
                            .join(" and ")
                    }
                );
            }
            if w.image.is_some() && w.poster.is_some() {
                problem!(
                    "work {n}: \"poster\" is only for \"youtube\" and \"audio\" works; a picture is its own preview, so remove it"
                );
            }
            let poster = match &w.poster {
                Some(p) => Some(about(&at("poster"), reader.picture(p, WORK_MIN).await)?),
                None => None,
            };
            Ok::<_, Problem>(match (&w.image, &w.youtube, &w.audio) {
                (Some(path), _, _) => Work::Image {
                    picture: about(&at("image"), reader.picture(path, WORK_MIN).await)?,
                    title,
                    caption,
                    skill,
                },
                (_, Some(link), _) => {
                    let Some(youtube_id) = media::youtube_id(link) else {
                        problem!(
                            "{}: \"{link}\" isn't a YouTube video link; {YOUTUBE_FORMS}",
                            at("youtube")
                        );
                    };
                    Work::Video {
                        youtube_id,
                        poster,
                        title,
                        caption,
                        skill,
                    }
                }
                (_, _, Some(path)) => Work::Audio {
                    audio: about(&at("audio"), read_audio(state, &base, path).await)?,
                    poster,
                    title,
                    caption,
                    skill,
                },
                (None, None, None) => unreachable!("exactly one was checked above"),
            })
        };
        match checked.await {
            Ok(work) => {
                let key = work.key();
                if let Some(first) = first_listed.get(&key) {
                    problems.push(format!(
                        "work {n} is the same as work {first} ({key}); list each work once"
                    ));
                } else {
                    first_listed.insert(key, n);
                }
                works.push(work);
            }
            Err(Problem(p)) => problems.push(p),
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let design = design::read(state, &base, &mut reader, &works)
        .await
        .map_err(one)?;
    Ok(Some(Checked {
        avatar,
        banner,
        works,
        design,
    }))
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
    let Checked {
        avatar,
        banner,
        works,
        design,
    } = match read_and_check(state, &repo_path, &commit, false).await {
        Ok(Some(checked)) => checked,
        Ok(None) => return Ok(Outcome::NoManifest),
        Err(mut problems) => {
            let more = problems.len().saturating_sub(MAX_PROBLEMS);
            problems.truncate(MAX_PROBLEMS);
            if more > 0 {
                problems.push(format!("…and {more} more"));
            }
            return Err(Problem(problems.join("\n")));
        }
    };

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
        // Sets from before page designs keep their fingerprint.
        let value = match &design {
            None => json!([pic(&avatar), pic(&banner), sorted]),
            Some((_, design)) => json!([pic(&avatar), pic(&banner), sorted, design]),
        };
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
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, avatar, banner, works, design, fingerprint, source)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'creative')",
    )
    .bind(id)
    .bind(creative_id)
    .bind(repo_url)
    .bind(&commit)
    .bind(avatar.map(sqlx::types::Json))
    .bind(banner.map(sqlx::types::Json))
    .bind(sqlx::types::Json(&works))
    .bind(design.map(|(d, _)| sqlx::types::Json(d)))
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
    /// (creative, status, handle, first approved portfolio, page hidden, can be paid)
    type Row = (Uuid, String, String, bool, bool, bool);
    let row: Option<Row> = sqlx::query_as(
        "select ps.creative_id, ps.status, c.handle, c.portfolio_set_id is null,
                c.hidden_reason is not null, c.stripe_status = 'active'
         from portfolio_sets ps
         join creatives c on c.user_id = ps.creative_id where ps.id = $1 for update of ps",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (creative_id, status, handle, first, hidden, paid) =
        row.ok_or_else(|| anyhow::anyhow!("no such portfolio"))?;
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
    let (title, body) = if first && !hidden {
        (
            "Your People page is live".to_string(),
            format!(
                "SLAPorSLOP approved your portfolio: makers can find your page on People now.{}",
                if paid {
                    ""
                } else {
                    " To be hired, set up payments under Get paid on your page's editor."
                }
            ),
        )
    } else {
        (
            "Your new portfolio was approved".to_string(),
            if hidden {
                "Your People page is still hidden: see the decision under Account → Moderation."
                    .to_string()
            } else {
                "It's on your People page now.".to_string()
            },
        )
    };
    crate::notify::send(
        &mut tx,
        crate::notify::Notice {
            user_id: creative_id,
            category: "moderation",
            title,
            body,
            link: format!("/people/{handle}"),
            collapse_key: None,
        },
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
    /// (creative, status, handle, an approved portfolio shows)
    type Row = (Uuid, String, String, bool);
    let row: Option<Row> = sqlx::query_as(
        "select ps.creative_id, ps.status, c.handle, c.portfolio_set_id is not null
         from portfolio_sets ps
         join creatives c on c.user_id = ps.creative_id where ps.id = $1 for update of ps",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (creative_id, status, handle, live) =
        row.ok_or_else(|| anyhow::anyhow!("no such portfolio"))?;
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
    crate::notify::send(
        &mut tx,
        crate::notify::Notice {
            user_id: creative_id,
            category: "moderation",
            title: "Your portfolio wasn't approved".into(),
            body: format!(
                "{reason}\n\nFix it in your repository, then use \"Check my portfolio now\" on your People page's editor: the new one is reviewed again. {}",
                if live {
                    "Your page keeps showing the portfolio approved before."
                } else {
                    "Only you can see your page until a portfolio is approved."
                }
            ),
            link: "/people/me#portfolio".into(),
            collapse_key: None,
        },
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
    /// The creative's services, which their page shows below a design.
    pub services: Vec<super::ServiceView>,
}

pub async fn queued(state: &AppState) -> ApiResult<Vec<QueuedPortfolio>> {
    /// (set, handle, name, AI use, live set, order, services)
    type Row = (
        Uuid,
        String,
        String,
        String,
        Option<Uuid>,
        Vec<String>,
        sqlx::types::Json<Vec<super::Service>>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "select ps.id, c.handle, c.name, c.ai_use, c.portfolio_set_id, c.portfolio_order, c.services
         from portfolio_sets ps join creatives c on c.user_id = ps.creative_id
         where ps.status = 'pending' order by ps.created_at",
    )
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, handle, name, ai_use, live, order, services) in rows {
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
            services: services
                .0
                .into_iter()
                .map(|s| super::ServiceView {
                    skill_name: super::skill(&s.skill).name,
                    service: s,
                })
                .collect(),
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
        // Their own work: the repository must belong to their linked GitHub account (or an
        // organization they own), like an app's (security review, 2026-10-22).
        let owned = crate::repo_owner::assess(&state, user.id, repo).await?;
        if !owned.ok {
            // "github_account" when linking a GitHub account fixes it: the editor offers that
            // button right there.
            let field = if owned.link == Some("github") {
                "github_account"
            } else {
                "repo_url"
            };
            return Err(ApiError::invalid(field, owned.message));
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
            "add the address of the public GitHub repository your portfolio is in, e.g. https://github.com/you/portfolio",
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

    /// web/public/portfolio.md (the guide for people and their AI assistants) states the limits
    /// this file enforces; it has to change when they do.
    #[test]
    fn the_portfolio_guide_states_every_limit() {
        let guide = include_str!("../../../../web/public/portfolio.md");
        let mb = |b: u64| format!("{} MB", b / (1024 * 1024));
        let size = |(w, h): (u32, u32)| format!("{w}×{h}");
        let mut want = vec![
            MANIFEST_PATH.to_string(),
            format!("{} KB", MAX_MANIFEST_BYTES / 1024),
            format!("1 to {MAX_WORKS}"),
            mb(MAX_AUDIO_BYTES),
            mb(media::MAX_IMAGE_BYTES),
            format!("**{MAX_TITLE}**"),
            format!("**{}**", media::MAX_CAPTION),
            size(WORK_MIN),
            size(AVATAR_MIN),
            size(BANNER_MIN),
            "8192".to_string(),
            "## Your page's design".to_string(),
            "Check my portfolio now".to_string(),
        ];
        want.extend(SKILLS.iter().map(|(slug, _)| format!("`{slug}`")));
        for w in want {
            assert!(guide.contains(&w), "portfolio.md doesn't say {w}");
        }
    }

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
