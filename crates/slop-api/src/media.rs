// SPDX-License-Identifier: AGPL-3.0-or-later
//! Store-page pictures and videos, from the maker's repository (decided 2026-10-19).
//!
//! Slop doesn't host them. A maker lists them in `.slop/media.json` in their repository:
//!
//! ```json
//! {
//!   "thumbnail": "thumbnail.png",
//!   "gallery": [
//!     { "image": "screenshots/title.png", "caption": "The title screen" },
//!     { "youtube": "https://youtu.be/…", "poster": "screenshots/trailer.png" }
//!   ]
//! }
//! ```
//!
//! Slop reads that file **at one commit** (the default branch's newest when the maker presses
//! "check", or a release's commit when one is fetched), downloads each picture once to check it
//! (format, size, dimensions, SHA-256), and keeps links to the files **at that commit**. Git
//! content at a commit can't change, so the pictures people see are the ones that were reviewed.
//!
//! Every new set of pictures waits for review, by a person or an AI reviewer (review.rs); the
//! store keeps showing the last approved set meanwhile. A set with the same pictures and
//! captions as the shown or waiting one isn't new, so nothing is reviewed twice.
//!
//! The maker puts the gallery in order on the website (`apps.media_order`); that isn't reviewed,
//! since it only moves pictures that were. New items go after the ones already placed.
//!
//! Files come from raw.githubusercontent.com, which isn't the GitHub API: only finding the
//! newest commit uses an API request. Players' browsers load the pictures from GitHub too.
//! Videos stay on YouTube; the page only loads YouTube's player when someone presses play.

use std::{collections::HashMap, io::Cursor};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post, put},
};
use image::{ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::{
    AppState, admin,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    updates::owned,
};

/// Where the list lives in the repository; picture paths are relative to its folder.
pub const MANIFEST_DIR: &str = ".slop";
pub const MANIFEST_PATH: &str = ".slop/media.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// The largest picture Slop accepts.
pub const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
/// Screenshots and videos in one app's gallery.
pub const MAX_GALLERY: usize = 20;
const MAX_CAPTION: usize = 200;
/// The store tile is shown at 460×215 (Steam's capsule shape); best at twice that.
const THUMB_MIN: (u32, u32) = (460, 215);
const SHOT_MIN: (u32, u32) = (320, 180);
const MAX_SIDE: u32 = 8192;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/media", get(overview))
        .route("/apps/{slug}/media/check", post(check))
        .route("/apps/{slug}/media/order", put(set_order))
}

// ---------------------------------------------------------------------------------------------
// What's stored

/// A picture Slop checked, linked at the commit it was read at.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Picture {
    /// Inside `.slop/`.
    pub path: String,
    pub url: String,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    /// png, jpeg, webp or gif.
    pub format: String,
    /// Perceptual hash (`dhash`, 16 hex digits): near-copies of a picture hash alike.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dhash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    Image {
        #[serde(flatten)]
        picture: Picture,
        caption: Option<String>,
    },
    Video {
        youtube_id: String,
        /// The maker's poster from the repository; without one, the page shows a plain card.
        poster: Option<Picture>,
        caption: Option<String>,
    },
}

impl Item {
    /// Identifies the item for the maker's order: `image:<path>` or `video:<youtube id>`.
    pub fn key(&self) -> String {
        match self {
            Item::Image { picture, .. } => format!("image:{}", picture.path),
            Item::Video { youtube_id, .. } => format!("video:{youtube_id}"),
        }
    }
}

/// Puts a gallery in the maker's order; items they haven't placed keep media.json's order, after.
pub fn arrange(items: &mut [Item], order: &[String]) {
    let rank = |item: &Item| {
        let key = item.key();
        order.iter().position(|k| *k == key).unwrap_or(usize::MAX)
    };
    // Stable: unplaced items keep their relative order.
    items.sort_by_key(rank);
}

/// A set of pictures, as makers and reviewers see it.
#[derive(Serialize, sqlx::FromRow)]
pub struct MediaSet {
    pub id: Uuid,
    pub commit_sha: String,
    #[sqlx(json(nullable))]
    pub thumbnail: Option<Picture>,
    #[sqlx(json)]
    pub gallery: Vec<Item>,
    pub status: String,
    pub status_reason: Option<String>,
    pub source: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub reviewed_at: Option<OffsetDateTime>,
    /// Other makers' apps whose icon looks like this thumbnail, for the reviewer.
    #[sqlx(json)]
    pub lookalikes: Vec<Lookalike>,
}

/// Another maker's app with an icon like a new thumbnail.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Lookalike {
    pub slug: String,
    pub title: String,
    /// Differing bits out of 64: 0 is the same picture.
    pub distance: u32,
}

/// Hashes this close (out of 64 bits) count as looking alike.
const LOOKALIKE_DISTANCE: u32 = 10;

pub const SET_COLUMNS: &str = "id, commit_sha, thumbnail, gallery, status, status_reason, source, created_at, reviewed_at, lookalikes";

pub async fn load_set(db: &sqlx::PgPool, id: Uuid) -> sqlx::Result<MediaSet> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {SET_COLUMNS} from media_sets where id = $1"
    )))
    .bind(id)
    .fetch_one(db)
    .await
}

/// One screenshot or video on an app's page.
#[derive(Serialize)]
pub struct GalleryItem {
    pub id: String,
    /// image or video.
    pub kind: &'static str,
    /// The picture, or the video's poster (none: a plain card).
    pub url: Option<String>,
    pub youtube_id: Option<String>,
    pub caption: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// The approved gallery the store shows for an app.
pub async fn gallery(state: &AppState, app_id: Uuid) -> sqlx::Result<Vec<GalleryItem>> {
    let row: Option<(Uuid, sqlx::types::Json<Vec<Item>>, Vec<String>)> = sqlx::query_as(
        "select ms.id, ms.gallery, a.media_order from apps a join media_sets ms on ms.id = a.media_set_id
         where a.id = $1",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((set, mut items, order)) = row else {
        return Ok(vec![]);
    };
    arrange(&mut items.0, &order);
    Ok(items
        .0
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            let id = format!("{set}-{i}");
            match item {
                Item::Image { picture, caption } => GalleryItem {
                    id,
                    kind: "image",
                    url: Some(picture.url),
                    youtube_id: None,
                    caption,
                    width: Some(picture.width),
                    height: Some(picture.height),
                },
                Item::Video {
                    youtube_id,
                    poster,
                    caption,
                } => GalleryItem {
                    id,
                    kind: "video",
                    url: poster.as_ref().map(|p| p.url.clone()),
                    youtube_id: Some(youtube_id),
                    caption,
                    width: poster.as_ref().map(|p| p.width),
                    height: poster.as_ref().map(|p| p.height),
                },
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Reading .slop/media.json

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    thumbnail: Option<String>,
    #[serde(default)]
    gallery: Vec<ManifestItem>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestItem {
    image: Option<String>,
    youtube: Option<String>,
    poster: Option<String>,
    caption: Option<String>,
}

/// Why a repository's pictures couldn't be used, in words for the maker.
#[derive(Debug)]
pub struct Problem(pub String);

impl<E: std::fmt::Display> From<E> for Problem {
    fn from(err: E) -> Self {
        Self(err.to_string())
    }
}

macro_rules! problem {
    ($($arg:tt)*) => { return Err(Problem(format!($($arg)*))) };
}

/// A picture path inside `.slop/`: plain letters, digits, `.`, `_`, `-` and `/`, no `..`.
fn clean_path(path: &str) -> Result<String, Problem> {
    clean_file_path(
        path,
        &[".png", ".jpg", ".jpeg", ".webp", ".gif"],
        "a picture: use .png, .jpg, .webp or .gif",
    )
}

/// A path inside `.slop/` with one of `exts`; `what` finishes "isn't …" when it has another.
pub(crate) fn clean_file_path(path: &str, exts: &[&str], what: &str) -> Result<String, Problem> {
    let path = path.trim().trim_start_matches("./");
    let ok = !path.is_empty()
        && path.len() <= 200
        && !path.starts_with('/')
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..");
    if !ok {
        problem!(
            "\"{path}\" isn't a usable path: use a path inside .slop/ with letters, numbers, dots, dashes and slashes (e.g. \"screenshots/1.png\")"
        );
    }
    let lower = path.to_ascii_lowercase();
    if !exts.iter().any(|ext| lower.ends_with(ext)) {
        problem!("\"{path}\" isn't {what}");
    }
    Ok(path.to_string())
}

pub(crate) fn clean_caption(caption: Option<&str>) -> Result<Option<String>, Problem> {
    let Some(c) = caption.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    if c.chars().count() > MAX_CAPTION || c.chars().any(char::is_control) {
        problem!("captions are one line of up to {MAX_CAPTION} characters: \"{c}\"");
    }
    Ok(Some(c.to_string()))
}

/// The video ID in a YouTube link: watch?v=, youtu.be/, /embed/, /shorts/ and /live/ forms.
pub fn youtube_id(link: &str) -> Option<String> {
    let url: Url = link.trim().parse().ok()?;
    if !matches!(url.scheme(), "https" | "http") {
        return None;
    }
    let host = url
        .host_str()?
        .trim_start_matches("www.")
        .trim_start_matches("m.");
    let mut path = url.path_segments()?.filter(|s| !s.is_empty());
    let id = match host {
        "youtu.be" => path.next()?.to_string(),
        "youtube.com" | "youtube-nocookie.com" | "music.youtube.com" => match path.next()? {
            "watch" => url
                .query_pairs()
                .find(|(k, _)| k == "v")
                .map(|(_, v)| v.into_owned())?,
            "embed" | "shorts" | "live" | "v" => path.next()?.to_string(),
            _ => return None,
        },
        _ => return None,
    };
    let valid = id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    valid.then_some(id)
}

/// Downloads a file, refusing anything over `max` bytes. None when it isn't there.
pub(crate) async fn download(
    state: &AppState,
    url: Url,
    max: u64,
) -> anyhow::Result<Option<Vec<u8>>> {
    let mut res = state
        .fetch_http
        .get(url)
        .header("user-agent", "slop-api")
        .send()
        .await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let res_ok = res.status().is_success();
    anyhow::ensure!(res_ok, "the file host answered {}", res.status());
    if res.content_length().is_some_and(|n| n > max) {
        anyhow::bail!("too big");
    }
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        body.extend_from_slice(&chunk);
        anyhow::ensure!(body.len() as u64 <= max, "too big");
    }
    Ok(Some(body))
}

/// Checks a picture's format and size from its header. Pictures are never decoded in full or
/// re-served by Slop; browsers load them from the repository.
fn inspect(path: &str, bytes: &[u8], min: (u32, u32)) -> Result<(String, u32, u32), Problem> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| Problem(format!("{path} isn't a picture Slop can read")))?;
    let format = match reader.format() {
        Some(ImageFormat::Png) => "png",
        Some(ImageFormat::Jpeg) => "jpeg",
        Some(ImageFormat::WebP) => "webp",
        Some(ImageFormat::Gif) => "gif",
        _ => problem!("{path} isn't a PNG, JPEG, WebP or GIF picture"),
    };
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| Problem(format!("{path} is damaged or not really a {format}")))?;
    if w > MAX_SIDE || h > MAX_SIDE {
        problem!("{path} is {w}×{h}; pictures can be up to {MAX_SIDE} pixels on each side");
    }
    if w < min.0 || h < min.1 {
        problem!(
            "{path} is {w}×{h}; it needs to be at least {}×{}",
            min.0,
            min.1
        );
    }
    Ok((format.to_string(), w, h))
}

pub(crate) struct Reader<'a> {
    state: &'a AppState,
    /// `<raw base><owner>/<repo>/<commit>/.slop/`
    base: Url,
    seen: HashMap<String, Picture>,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(state: &'a AppState, base: Url) -> Self {
        Self {
            state,
            base,
            seen: HashMap::new(),
        }
    }

    pub(crate) async fn picture(
        &mut self,
        path: &str,
        min: (u32, u32),
    ) -> Result<Picture, Problem> {
        let path = clean_path(path)?;
        if let Some(p) = self.seen.get(&path) {
            inspect_dims(p, min)?;
            return Ok(p.clone());
        }
        let url = self.base.join(&path)?;
        let bytes = match download(self.state, url.clone(), MAX_IMAGE_BYTES).await {
            Ok(Some(b)) => b,
            Ok(None) => problem!("{MANIFEST_DIR}/{path} isn't in the repository at that commit"),
            Err(e) if e.to_string() == "too big" => {
                problem!("{path} is over 8 MB")
            }
            Err(e) => problem!("couldn't download {path}: {e:#}"),
        };
        let (format, width, height) = inspect(&path, &bytes, min)?;
        let picture = Picture {
            path: path.clone(),
            url: url.to_string(),
            sha256: hex::encode(Sha256::digest(&bytes)),
            width,
            height,
            format,
            dhash: dhash_off_thread(bytes.to_vec())
                .await
                .map(|h| format!("{h:016x}")),
        };
        self.seen.insert(path, picture.clone());
        Ok(picture)
    }
}

fn inspect_dims(p: &Picture, min: (u32, u32)) -> Result<(), Problem> {
    if p.width < min.0 || p.height < min.1 {
        problem!(
            "{} is {}×{}; it needs to be at least {}×{}",
            p.path,
            p.width,
            p.height,
            min.0,
            min.1
        );
    }
    Ok(())
}

/// What a check found.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// There's no `.slop/media.json` at that commit; the shown pictures stay as they are.
    NoManifest,
    /// Same pictures as the ones shown or already waiting.
    Unchanged,
    /// A new set is waiting for review.
    Pending(Uuid),
}

/// The newest commit on the repository's default branch.
pub(crate) async fn head_commit(state: &AppState, repo_path: &str) -> Result<String, Problem> {
    #[derive(Deserialize)]
    struct Commit {
        sha: String,
    }
    let commit: Option<Commit> =
        crate::updates::gh_json(state, &format!("repos/{repo_path}/commits/HEAD"))
            .await
            .map_err(|e| Problem(format!("couldn't ask GitHub for the newest commit: {e:#}")))?;
    match commit {
        Some(c) => Ok(c.sha.to_ascii_lowercase()),
        None => problem!(
            "GitHub can't find the repository; it has to be public for its pictures to show"
        ),
    }
}

/// Reads `.slop/media.json` at `commit` (default: the default branch's newest), and if it
/// lists something new, stores it as a set waiting for review. Records the result on the app.
pub async fn refresh(
    state: &AppState,
    app_id: Uuid,
    repo_url: &str,
    commit: Option<&str>,
    source: &str,
) -> Result<Outcome, Problem> {
    let result = read_and_store(state, app_id, repo_url, commit, source).await;
    let problem = result.as_ref().err().map(|p| p.0.clone());
    sqlx::query("update apps set media_checked_at = now(), media_problem = $2 where id = $1")
        .bind(app_id)
        .bind(problem)
        .execute(&state.db)
        .await
        .map_err(|e| Problem(e.to_string()))?;
    result
}

async fn read_and_store(
    state: &AppState,
    app_id: Uuid,
    repo_url: &str,
    commit: Option<&str>,
    source: &str,
) -> Result<Outcome, Problem> {
    let Some(repo_path) = slop_provenance::github_repo_path(repo_url) else {
        problem!("pictures from the repository only work for GitHub repositories so far");
    };
    let commit = match commit {
        Some(c) => c.to_ascii_lowercase(),
        None => head_commit(state, &repo_path).await?,
    };
    let base = state
        .config
        .media
        .github_raw_base
        .join(&format!("{repo_path}/{commit}/{MANIFEST_DIR}/"))?;
    let manifest = match download(state, base.join("media.json")?, MAX_MANIFEST_BYTES).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(Outcome::NoManifest),
        Err(e) => problem!("couldn't download {MANIFEST_PATH}: {e:#}"),
    };
    let manifest: Manifest = serde_json::from_slice(&manifest)
        .map_err(|e| Problem(format!("{MANIFEST_PATH} isn't valid: {e}")))?;
    if manifest.gallery.len() > MAX_GALLERY {
        problem!(
            "{MANIFEST_PATH} lists {} gallery items; the most is {MAX_GALLERY}",
            manifest.gallery.len()
        );
    }

    let mut reader = Reader {
        state,
        base,
        seen: HashMap::new(),
    };
    let thumbnail = match &manifest.thumbnail {
        Some(path) => Some(reader.picture(path, THUMB_MIN).await?),
        None => None,
    };
    let mut gallery = Vec::with_capacity(manifest.gallery.len());
    for (i, item) in manifest.gallery.iter().enumerate() {
        let n = i + 1;
        let caption = clean_caption(item.caption.as_deref())?;
        gallery.push(match (&item.image, &item.youtube) {
            (Some(path), None) => {
                if item.poster.is_some() {
                    problem!("gallery item {n}: \"poster\" is only for videos");
                }
                Item::Image {
                    picture: reader.picture(path, SHOT_MIN).await?,
                    caption,
                }
            }
            (None, Some(link)) => {
                let Some(youtube_id) = youtube_id(link) else {
                    problem!("gallery item {n}: \"{link}\" isn't a YouTube video link");
                };
                let poster = match &item.poster {
                    Some(path) => Some(reader.picture(path, SHOT_MIN).await?),
                    None => None,
                };
                Item::Video {
                    youtube_id,
                    poster,
                    caption,
                }
            }
            _ => problem!("gallery item {n} needs either \"image\" or \"youtube\""),
        });
    }

    let mut keys: Vec<String> = gallery.iter().map(Item::key).collect();
    keys.sort();
    if let Some(dup) = keys.windows(2).find(|w| w[0] == w[1]) {
        problem!(
            "{MANIFEST_PATH} lists {} twice",
            dup[0].split_once(':').map_or("", |(_, v)| v)
        );
    }

    let fingerprint = fingerprint(thumbnail.as_ref(), &gallery);
    let mut tx = state.db.begin().await?;
    sqlx::query("select 1 from apps where id = $1 for update")
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    let live: Option<String> = sqlx::query_scalar(
        "select ms.fingerprint from apps a join media_sets ms on ms.id = a.media_set_id where a.id = $1",
    )
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;
    let pending: Option<(Uuid, String)> = sqlx::query_as(
        "select id, fingerprint from media_sets where app_id = $1 and status = 'pending'",
    )
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;
    if pending.as_ref().is_some_and(|(_, f)| *f == fingerprint) {
        return Ok(Outcome::Unchanged);
    }
    // Anything waiting is out of date now.
    sqlx::query(
        "update media_sets set status = 'superseded', status_reason = 'replaced by newer pictures'
         where app_id = $1 and status = 'pending'",
    )
    .bind(app_id)
    .execute(&mut *tx)
    .await?;
    if live.as_deref() == Some(fingerprint.as_str()) {
        tx.commit().await?;
        return Ok(Outcome::Unchanged);
    }
    // The same pictures were turned down (after the shown ones): don't queue them again.
    let rejected: Option<String> = sqlx::query_scalar(
        "select ms.status_reason from media_sets ms join apps a on a.id = ms.app_id
         where ms.app_id = $1 and ms.status = 'rejected' and ms.fingerprint = $2
           and (a.media_set_id is null
                or ms.created_at > (select created_at from media_sets where id = a.media_set_id))
         order by ms.created_at desc limit 1",
    )
    .bind(app_id)
    .bind(&fingerprint)
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    if let Some(reason) = rejected {
        tx.commit().await?;
        problem!("these are the pictures that were turned down: {reason}");
    }
    let id = Uuid::now_v7();
    let like = lookalikes(&mut tx, app_id, thumbnail.as_ref()).await?;
    sqlx::query(
        "insert into media_sets (id, app_id, commit_sha, thumbnail, gallery, fingerprint, source, lookalikes)
         values ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(app_id)
    .bind(&commit)
    .bind(thumbnail.map(sqlx::types::Json))
    .bind(sqlx::types::Json(&gallery))
    .bind(&fingerprint)
    .bind(source)
    .bind(sqlx::types::Json(&like))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Outcome::Pending(id))
}

/// A difference hash: the picture shrunk to 9×8 in grey, one bit per pair of neighbours (is the
/// left one brighter?). Resizing, recompressing or a small colour change barely moves it.
pub fn dhash(bytes: &[u8]) -> Option<u64> {
    // Decoding is bounded: a picture that would need more memory than this isn't hashed (it
    // still goes to review) rather than letting one upload take the server's memory.
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(160 * 1024 * 1024);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.limits(limits);
    let img = reader.decode().ok()?.into_luma8();
    let small = image::imageops::resize(&img, 9, 8, image::imageops::FilterType::Triangle);
    let mut hash = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            hash <<= 1;
            if small.get_pixel(x, y).0[0] > small.get_pixel(x + 1, y).0[0] {
                hash |= 1;
            }
        }
    }
    Some(hash)
}

/// `dhash` off the async workers (decoding a big picture takes a while).
pub async fn dhash_off_thread(bytes: Vec<u8>) -> Option<u64> {
    tokio::task::spawn_blocking(move || dhash(&bytes))
        .await
        .ok()
        .flatten()
}

/// Other makers' apps whose live icon looks like `thumbnail`, closest first.
async fn lookalikes(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
    thumbnail: Option<&Picture>,
) -> sqlx::Result<Vec<Lookalike>> {
    let Some(mine) = thumbnail
        .and_then(|t| t.dhash.as_deref())
        .and_then(|h| u64::from_str_radix(h, 16).ok())
    else {
        return Ok(vec![]);
    };
    let others: Vec<(String, String, String)> = sqlx::query_as(
        "select a.slug, a.title, ms.thumbnail->>'dhash'
         from apps a join media_sets ms on ms.id = a.media_set_id
         where a.maker_id <> (select maker_id from apps where id = $1)
           and ms.thumbnail ? 'dhash'",
    )
    .bind(app_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut found: Vec<Lookalike> = others
        .into_iter()
        .filter_map(|(slug, title, hash)| {
            let distance = (u64::from_str_radix(&hash, 16).ok()? ^ mine).count_ones();
            (distance <= LOOKALIKE_DISTANCE).then_some(Lookalike {
                slug,
                title,
                distance,
            })
        })
        .collect();
    found.sort_by_key(|l| l.distance);
    found.truncate(5);
    Ok(found)
}

/// Live thumbnails from before hashing: hash a few at a time (cleanup.rs), so look-alikes are
/// found among them too.
pub async fn hash_old_thumbnails(state: &AppState, limit: i64) -> anyhow::Result<usize> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "select ms.id, ms.thumbnail->>'url' from apps a join media_sets ms on ms.id = a.media_set_id
         where ms.thumbnail is not null and not ms.thumbnail ? 'dhash'
           and ms.thumbnail->>'url' like 'http%'
         limit $1",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    let mut done = 0;
    for (id, url) in rows {
        // A picture that can't be read gets an empty hash, so it isn't tried again.
        let hash = match download(state, url.parse()?, MAX_IMAGE_BYTES).await {
            Ok(Some(bytes)) => dhash_off_thread(bytes.to_vec())
                .await
                .map(|h| format!("{h:016x}")),
            _ => None,
        };
        sqlx::query("update media_sets set thumbnail = thumbnail || jsonb_build_object('dhash', $2::text) where id = $1")
            .bind(id)
            .bind(hash.unwrap_or_default())
            .execute(&state.db)
            .await?;
        done += 1;
    }
    Ok(done)
}

/// What the pictures are, regardless of which commit they were read at or their order.
fn fingerprint(thumbnail: Option<&Picture>, gallery: &[Item]) -> String {
    let pic = |p: &Picture| json!([p.path, p.sha256]);
    let mut sorted: Vec<&Item> = gallery.iter().collect();
    sorted.sort_by_key(|i| i.key());
    let items: Vec<_> = sorted
        .into_iter()
        .map(|item| match item {
            Item::Image { picture, caption } => json!(["image", pic(picture), caption]),
            Item::Video {
                youtube_id,
                poster,
                caption,
            } => json!(["video", youtube_id, poster.as_ref().map(pic), caption]),
        })
        .collect();
    let value = json!([thumbnail.map(pic), items]);
    hex::encode(Sha256::digest(value.to_string().as_bytes()))
}

/// After a release is fetched: reads the pictures at its commit. Never fails the release.
pub async fn refresh_for_release(state: &AppState, app_id: Uuid, repo_url: &str, commit: &str) {
    if slop_provenance::github_repo_path(repo_url).is_none() {
        return;
    }
    match refresh(state, app_id, repo_url, Some(commit), "release").await {
        Ok(Outcome::Pending(set)) => {
            tracing::info!(%app_id, %set, "new pictures waiting for review")
        }
        Ok(_) => {}
        Err(Problem(problem)) => tracing::info!(%app_id, %problem, "pictures not read"),
    }
}

// ---------------------------------------------------------------------------------------------
// Review (admin_web.rs and review.rs call these)

pub async fn approve_set(db: &sqlx::PgPool, id: Uuid, reviewer: &str) -> anyhow::Result<()> {
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "select ms.app_id, ms.status, a.slug from media_sets ms join apps a on a.id = ms.app_id
         where ms.id = $1 for update of ms",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (app_id, status, slug) = row.context("no such set of pictures")?;
    anyhow::ensure!(
        status == "pending",
        "these pictures aren't waiting for review ({status})"
    );
    sqlx::query(
        "update media_sets set status = 'approved', reviewed_by = $2, reviewed_at = now() where id = $1",
    )
    .bind(id)
    .bind(reviewer)
    .execute(&mut *tx)
    .await?;
    sqlx::query("update apps set media_set_id = $2, updated_at = now() where id = $1")
        .bind(app_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    admin::audit(
        &mut tx,
        reviewer,
        "media.approve",
        &slug,
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
    anyhow::ensure!(!reason.is_empty(), "say why, for the maker");
    let mut tx = db.begin().await?;
    let row: Option<(String, String)> = sqlx::query_as(
        "select ms.status, a.slug from media_sets ms join apps a on a.id = ms.app_id
         where ms.id = $1 for update of ms",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, slug) = row.context("no such set of pictures")?;
    anyhow::ensure!(
        status == "pending",
        "these pictures aren't waiting for review ({status})"
    );
    sqlx::query(
        "update media_sets set status = 'rejected', status_reason = $3, reviewed_by = $2, reviewed_at = now()
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
        "media.reject",
        &slug,
        json!({ "set": id, "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Whether an app has pictures waiting for review (it can't be listed until they're reviewed).
pub async fn has_pending(conn: &mut sqlx::PgConnection, app_id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select exists (select 1 from media_sets where app_id = $1 and status = 'pending')",
    )
    .bind(app_id)
    .fetch_one(conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// The maker's view

#[derive(Serialize)]
struct Overview {
    /// The pictures the store shows.
    live: Option<MediaSet>,
    /// Newer pictures waiting for review.
    pending: Option<MediaSet>,
    /// The newest set that was turned down, if nothing newer was found since.
    rejected: Option<MediaSet>,
    #[serde(with = "time::serde::rfc3339::option")]
    checked_at: Option<OffsetDateTime>,
    /// What was wrong the last time Slop read the repository.
    problem: Option<String>,
    /// Only GitHub repositories so far.
    supported: bool,
    manifest_path: &'static str,
    /// The maker's gallery order (item keys); `live` and `pending` are already in it.
    order: Vec<String>,
    /// The result of a check, when this answers one: pending, unchanged or none.
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<&'static str>,
}

async fn load_overview(state: &AppState, app_id: Uuid, repo_url: &str) -> ApiResult<Overview> {
    let (live, checked_at, problem, order): (
        Option<Uuid>,
        Option<OffsetDateTime>,
        Option<String>,
        Vec<String>,
    ) = sqlx::query_as(
        "select media_set_id, media_checked_at, media_problem, media_order from apps where id = $1",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    let live = match live {
        Some(id) => Some(load_set(&state.db, id).await?),
        None => None,
    };
    let in_order = |set: Option<MediaSet>| {
        set.map(|mut s| {
            arrange(&mut s.gallery, &order);
            s
        })
    };
    let newest = |status: &'static str| {
        sqlx::query_as::<_, MediaSet>(sqlx::AssertSqlSafe(format!(
            "select {SET_COLUMNS} from media_sets where app_id = $1 and status = $2
             order by created_at desc limit 1"
        )))
        .bind(app_id)
        .bind(status)
        .fetch_optional(&state.db)
    };
    let live = in_order(live);
    let pending = in_order(newest("pending").await?);
    let rejected = newest("rejected")
        .await?
        .filter(|r| pending.is_none() && live.as_ref().is_none_or(|l| l.created_at < r.created_at));
    Ok(Overview {
        live,
        pending,
        rejected,
        checked_at,
        problem,
        supported: slop_provenance::github_repo_path(repo_url).is_some(),
        manifest_path: MANIFEST_PATH,
        order,
        result: None,
    })
}

async fn overview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Overview>> {
    let (app_id, repo_url) = owned(&state, &slug, &user).await?;
    Ok(Json(load_overview(&state, app_id, &repo_url).await?))
}

/// "Check my repository now": reads the default branch's newest commit.
async fn check(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Overview>> {
    let (app_id, repo_url) = owned(&state, &slug, &user).await?;
    // Each check downloads and decodes up to 21 pictures.
    crate::limits::check(&state, user.id, crate::limits::Action::Lookup)?;
    let result = match refresh(&state, app_id, &repo_url, None, "maker").await {
        Ok(Outcome::Pending(_)) => "pending",
        Ok(Outcome::Unchanged) => "unchanged",
        Ok(Outcome::NoManifest) => "none",
        Err(_) => "problem",
    };
    let mut overview = load_overview(&state, app_id, &repo_url).await?;
    overview.result = Some(result);
    Ok(Json(overview))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Order {
    /// Item keys, first to last.
    keys: Vec<String>,
}

/// The maker puts the gallery in order. Not reviewed: it only moves reviewed pictures.
async fn set_order(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(order): Json<Order>,
) -> ApiResult<Json<Overview>> {
    let (app_id, repo_url) = owned(&state, &slug, &user).await?;
    if order.keys.len() > MAX_GALLERY * 2
        || order
            .keys
            .iter()
            .any(|k| k.len() > 220 || !(k.starts_with("image:") || k.starts_with("video:")))
    {
        return Err(ApiError::invalid("keys", "that isn't a gallery order"));
    }
    sqlx::query("update apps set media_order = $2, updated_at = now() where id = $1")
        .bind(app_id)
        .bind(&order.keys)
        .execute(&state.db)
        .await?;
    Ok(Json(load_overview(&state, app_id, &repo_url).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An icon: a coloured disc on a gradient, as PNG.
    fn icon(size: u32, shift: u8, disc_at: f32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(size, size, |x, y| {
            let (fx, fy) = (x as f32 / size as f32, y as f32 / size as f32);
            let inside = (fx - disc_at).powi(2) + (fy - 0.5).powi(2) < 0.08;
            if inside {
                image::Rgb([220u8.saturating_add(shift), 40, 40])
            } else {
                let g = (fx * 200.0) as u8;
                image::Rgb([g.saturating_add(shift), g, 255 - g])
            }
        });
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn near_copies_of_an_icon_hash_alike() {
        let original = dhash(&icon(256, 0, 0.35)).unwrap();
        let resized_and_tinted = dhash(&icon(96, 12, 0.35)).unwrap();
        // Another game's icon: stripes, darkening downwards.
        let stripes = image::RgbImage::from_fn(256, 256, |x, y| {
            let v = if (x / 32 + y / 48).is_multiple_of(2) {
                230
            } else {
                30
            };
            image::Rgb([v, (255 - y) as u8, v / 2])
        });
        let mut png = Cursor::new(Vec::new());
        stripes.write_to(&mut png, ImageFormat::Png).unwrap();
        let different = dhash(&png.into_inner()).unwrap();
        assert!((original ^ resized_and_tinted).count_ones() <= LOOKALIKE_DISTANCE);
        assert!((original ^ different).count_ones() > LOOKALIKE_DISTANCE);
        assert_eq!(dhash(b"not a picture"), None);
    }

    #[test]
    fn youtube_links() {
        for link in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=10",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
        ] {
            assert_eq!(youtube_id(link).as_deref(), Some("dQw4w9WgXcQ"), "{link}");
        }
        for link in [
            "https://vimeo.com/123",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/channel/UCabcdefghijk",
            "https://evil.example/watch?v=dQw4w9WgXcQ",
            "javascript:alert(1)",
        ] {
            assert_eq!(youtube_id(link), None, "{link}");
        }
    }

    #[test]
    fn the_makers_order_comes_first() {
        let video = |id: &str| Item::Video {
            youtube_id: id.into(),
            poster: None,
            caption: None,
        };
        let mut items = vec![
            video("aaaaaaaaaaa"),
            video("bbbbbbbbbbb"),
            video("ccccccccccc"),
        ];
        arrange(
            &mut items,
            &[
                "video:ccccccccccc".into(),
                "video:gone0000000".into(),
                "video:aaaaaaaaaaa".into(),
            ],
        );
        let keys: Vec<_> = items.iter().map(Item::key).collect();
        assert_eq!(
            keys,
            [
                "video:ccccccccccc",
                "video:aaaaaaaaaaa",
                "video:bbbbbbbbbbb"
            ]
        );
        // The order doesn't change what's reviewed.
        let reversed: Vec<_> = items.iter().rev().cloned().collect();
        assert_eq!(fingerprint(None, &items), fingerprint(None, &reversed));
    }

    #[test]
    fn paths_stay_inside_the_folder() {
        assert_eq!(clean_path("./shots/1.png").unwrap(), "shots/1.png");
        for bad in [
            "../secret.png",
            "/etc/x.png",
            "shots//1.png",
            "a b.png",
            "shots/../../x.png",
            "notes.txt",
            "x.svg",
            "%2e%2e/x.png",
        ] {
            assert!(clean_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pictures_are_checked_from_their_header() {
        let png = |w, h| {
            let mut out = Vec::new();
            image::RgbImage::new(w, h)
                .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
                .unwrap();
            out
        };
        assert_eq!(
            inspect("t.png", &png(920, 430), THUMB_MIN).unwrap(),
            ("png".to_string(), 920, 430)
        );
        assert!(inspect("t.png", &png(400, 200), THUMB_MIN).is_err());
        assert!(inspect("x.png", b"<svg onload=alert(1)>", SHOT_MIN).is_err());
    }
}
