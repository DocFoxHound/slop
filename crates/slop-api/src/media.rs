//! Store-page pictures and videos (decided 2026-10-19).
//!
//! A maker sets a **thumbnail** (the store tile) and a **gallery** of screenshots and YouTube
//! videos, shown as a carousel on the app's page. Like the rest of the store page, changes go
//! live straight away.
//!
//! Every picture is decoded and re-encoded here (SECURITY §8): only PNG, JPEG, WebP and GIF are
//! accepted, what's stored is a fresh PNG or JPEG without the original's metadata (camera,
//! location), and it's served with a sandboxing CSP and `nosniff`. Videos stay on YouTube; Slop
//! keeps a copy of the poster so the page loads nothing from YouTube until someone presses play.
//!
//! Pictures are served by an unguessable ID to anyone who has it (the desktop app's pages can't
//! send the session cookie), except for delisted apps. The ID is only ever handed to viewers who
//! may see the app.

use std::io::Cursor;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, put},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, ImageReader, imageops::FilterType};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    updates::owned,
};

/// The largest picture a maker can upload, before re-encoding.
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// Upload bodies: the picture as base64 (4/3 of its size) in a small JSON object.
pub const MAX_UPLOAD_BODY: usize = MAX_IMAGE_BYTES / 3 * 4 + 16 * 1024;
/// Screenshots and videos in one app's gallery.
pub const MAX_GALLERY: i64 = 12;
const MAX_CAPTION: usize = 200;

/// The store tile's size: Steam's capsule shape, at twice the size it's shown.
const THUMB: (u32, u32) = (920, 430);
/// Screenshots are scaled down to fit this.
const SHOT_MAX: (u32, u32) = (1920, 1080);
const SHOT_MIN: (u32, u32) = (320, 180);

/// Reading pictures and the maker's gallery changes that don't upload anything.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/media/{id}", get(serve))
        .route("/apps/{slug}/thumbnail", delete(remove_thumbnail))
        .route("/apps/{slug}/gallery/order", put(reorder))
        .route(
            "/apps/{slug}/gallery/{id}",
            delete(remove_item).patch(edit_item),
        )
}

/// Uploads, with a body limit big enough for a picture.
pub fn upload_routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/thumbnail", put(set_thumbnail))
        .route("/apps/{slug}/gallery", axum::routing::post(add_item))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY))
}

// ---------------------------------------------------------------------------------------------
// What the store page shows

/// One screenshot or video in an app's gallery.
#[derive(Serialize, sqlx::FromRow)]
pub struct GalleryItem {
    pub id: Uuid,
    /// image or video.
    pub kind: String,
    /// The picture, or the video's poster.
    pub url: String,
    pub youtube_id: Option<String>,
    pub caption: Option<String>,
    pub width: i32,
    pub height: i32,
}

/// An app's gallery, in order.
pub async fn gallery(state: &AppState, app_id: Uuid) -> sqlx::Result<Vec<GalleryItem>> {
    sqlx::query_as(
        "select id, role as kind, $2 || id::text as url, youtube_id, caption, width, height
         from app_media where app_id = $1 and role <> 'thumbnail' order by position, created_at",
    )
    .bind(app_id)
    .bind(state.config.media_base())
    .fetch_all(&state.db)
    .await
}

async fn serve(State(state): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Response> {
    let row: Option<(String, Vec<u8>)> = sqlx::query_as(
        "select m.content_type, m.bytes from app_media m join apps a on a.id = m.app_id
         where m.id = $1 and a.status <> 'delisted'",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (content_type, bytes) = row.ok_or(ApiError::NotFound)?;
    // A new picture always gets a new ID, so this one never changes. `private`: a delisted app's
    // pictures shouldn't live on in shared caches.
    let headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_str(&content_type).map_err(anyhow::Error::from)?,
        ),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, max-age=31536000, immutable"),
        ),
        (
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; sandbox"),
        ),
        (
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("inline"),
        ),
        // Pages on other origins (the desktop app's) may show it.
        (
            header::HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("cross-origin"),
        ),
    ];
    Ok((headers, bytes).into_response())
}

// ---------------------------------------------------------------------------------------------
// Pictures

struct Picture {
    content_type: &'static str,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
enum Fit {
    /// Cropped to fill exactly this size: the store tile.
    Fill(u32, u32),
    /// Scaled down to fit inside the first size; refused below the second.
    Within((u32, u32), (u32, u32)),
}

fn decode_base64(field: &'static str, data: &str) -> ApiResult<Vec<u8>> {
    let data = data.trim();
    // Accept a data: URL as well as bare base64.
    let data = data
        .split_once(";base64,")
        .map_or(data, |(_, rest)| rest)
        .trim();
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| ApiError::invalid(field, "the picture didn't arrive intact; try again"))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(ApiError::invalid(field, "pictures can be up to 8 MB"));
    }
    Ok(bytes)
}

/// Decodes a picture in one of the accepted formats and re-encodes it to `fit`. Runs on a
/// blocking thread: decoding a large picture takes a while.
async fn process(field: &'static str, bytes: Vec<u8>, fit: Fit) -> ApiResult<Picture> {
    tokio::task::spawn_blocking(move || process_now(field, &bytes, fit))
        .await
        .map_err(anyhow::Error::from)?
}

fn process_now(field: &'static str, bytes: &[u8], fit: Fit) -> ApiResult<Picture> {
    let unreadable = || ApiError::invalid(field, "that isn't a PNG, JPEG, WebP or GIF picture");
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| unreadable())?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Gif)
    ) {
        return Err(unreadable());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| unreadable())?;
    let (w, h) = (img.width(), img.height());

    let img = match fit {
        Fit::Fill(tw, th) => {
            if w < tw / 2 || h < th / 2 {
                return Err(ApiError::invalid(
                    field,
                    format!(
                        "the thumbnail needs to be at least {}×{} (best: {tw}×{th})",
                        tw / 2,
                        th / 2
                    ),
                ));
            }
            img.resize_to_fill(tw, th, FilterType::Lanczos3)
        }
        Fit::Within((mw, mh), (minw, minh)) => {
            if w < minw || h < minh {
                return Err(ApiError::invalid(
                    field,
                    format!("pictures need to be at least {minw}×{minh}"),
                ));
            }
            if w > mw || h > mh {
                img.resize(mw, mh, FilterType::Lanczos3)
            } else {
                img
            }
        }
    };
    encode(&img).map_err(|e| ApiError::Internal(e.into()))
}

/// PNG when the picture has transparency (pixel art, logos), JPEG otherwise.
fn encode(img: &DynamicImage) -> image::ImageResult<Picture> {
    let mut out = Vec::new();
    let transparent = img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p.0[3] < 255);
    let content_type = if transparent {
        img.to_rgba8()
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)?;
        "image/png"
    } else {
        let rgb = img.to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88).encode_image(&rgb)?;
        "image/jpeg"
    };
    Ok(Picture {
        content_type,
        bytes: out,
        width: img.width(),
        height: img.height(),
    })
}

// ---------------------------------------------------------------------------------------------
// YouTube

/// The video ID in a YouTube link: watch?v=, youtu.be/, /embed/, /shorts/ and /live/ forms.
fn youtube_id(link: &str) -> Option<String> {
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

/// Fetches the video's poster, which also shows that the video exists.
async fn youtube_poster(state: &AppState, id: &str) -> ApiResult<Picture> {
    let missing = || ApiError::invalid("url", "couldn't find that YouTube video; is it public?");
    let url = state
        .config
        .media
        .youtube_image_base
        .join(&format!("vi/{id}/hqdefault.jpg"))
        .map_err(anyhow::Error::from)?;
    let res = state.http.get(url).send().await.map_err(|err| {
        tracing::warn!(error = %err, "YouTube poster");
        ApiError::Unavailable("couldn't reach YouTube; try again in a minute")
    })?;
    if !res.status().is_success() {
        return Err(missing());
    }
    let bytes = res.bytes().await.map_err(|_| missing())?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(missing());
    }
    process("url", bytes.to_vec(), Fit::Within(SHOT_MAX, (1, 1)))
        .await
        .map_err(|_| missing())
}

// ---------------------------------------------------------------------------------------------
// Changes

#[derive(Deserialize)]
struct ThumbnailUpload {
    /// The picture, base64 (or a data: URL).
    image: String,
}

#[derive(Serialize)]
struct ThumbnailSet {
    thumbnail_url: String,
}

async fn touch(conn: &mut PgConnection, app_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("update apps set updated_at = now() where id = $1")
        .bind(app_id)
        .execute(conn)
        .await?;
    Ok(())
}

async fn set_thumbnail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(body): Json<ThumbnailUpload>,
) -> ApiResult<Json<ThumbnailSet>> {
    let (app_id, _) = owned(&state, &slug, &user).await?;
    let bytes = decode_base64("image", &body.image)?;
    let pic = process("image", bytes, Fit::Fill(THUMB.0, THUMB.1)).await?;
    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    sqlx::query("delete from app_media where app_id = $1 and role = 'thumbnail'")
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    insert(&mut tx, id, app_id, "thumbnail", 0, None, None, &pic).await?;
    touch(&mut tx, app_id).await?;
    tx.commit().await?;
    Ok(Json(ThumbnailSet {
        thumbnail_url: format!("{}{id}", state.config.media_base()),
    }))
}

async fn remove_thumbnail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let (app_id, _) = owned(&state, &slug, &user).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("delete from app_media where app_id = $1 and role = 'thumbnail'")
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    touch(&mut tx, app_id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[allow(clippy::too_many_arguments)]
async fn insert(
    conn: &mut PgConnection,
    id: Uuid,
    app_id: Uuid,
    role: &str,
    position: i16,
    youtube_id: Option<&str>,
    caption: Option<&str>,
    pic: &Picture,
) -> sqlx::Result<()> {
    sqlx::query(
        "insert into app_media (id, app_id, role, position, youtube_id, caption, content_type, bytes, width, height)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(id)
    .bind(app_id)
    .bind(role)
    .bind(position)
    .bind(youtube_id)
    .bind(caption)
    .bind(pic.content_type)
    .bind(&pic.bytes)
    .bind(pic.width as i32)
    .bind(pic.height as i32)
    .execute(conn)
    .await?;
    Ok(())
}

fn validate_caption(caption: Option<&str>) -> ApiResult<Option<String>> {
    let Some(caption) = caption.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    if caption.chars().count() > MAX_CAPTION {
        return Err(ApiError::invalid(
            "caption",
            format!("captions can be up to {MAX_CAPTION} characters"),
        ));
    }
    Ok(Some(caption.to_string()))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum NewItem {
    Image {
        image: String,
        caption: Option<String>,
    },
    Video {
        /// A YouTube link.
        url: String,
        caption: Option<String>,
    },
}

async fn add_item(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(item): Json<NewItem>,
) -> ApiResult<(StatusCode, Json<Vec<GalleryItem>>)> {
    let (app_id, _) = owned(&state, &slug, &user).await?;
    let count: i64 = sqlx::query_scalar(
        "select count(*) from app_media where app_id = $1 and role <> 'thumbnail'",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    if count >= MAX_GALLERY {
        return Err(ApiError::Conflict(format!(
            "the gallery holds up to {MAX_GALLERY} screenshots and videos; remove one first"
        )));
    }

    let (role, youtube, caption, pic) = match item {
        NewItem::Image { image, caption } => {
            let caption = validate_caption(caption.as_deref())?;
            let bytes = decode_base64("image", &image)?;
            let pic = process("image", bytes, Fit::Within(SHOT_MAX, SHOT_MIN)).await?;
            ("image", None, caption, pic)
        }
        NewItem::Video { url, caption } => {
            let caption = validate_caption(caption.as_deref())?;
            let id = youtube_id(&url).ok_or_else(|| {
                ApiError::invalid(
                    "url",
                    "paste a YouTube link, like https://www.youtube.com/watch?v=…",
                )
            })?;
            let pic = youtube_poster(&state, &id).await?;
            ("video", Some(id), caption, pic)
        }
    };

    let mut tx = state.db.begin().await?;
    // Lock the app so two uploads at once can't both take the last place.
    sqlx::query("select 1 from apps where id = $1 for update")
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    let (count, next): (i64, i16) = sqlx::query_as(
        "select count(*), coalesce(max(position) + 1, 0)::smallint from app_media
         where app_id = $1 and role <> 'thumbnail'",
    )
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;
    if count >= MAX_GALLERY {
        return Err(ApiError::Conflict(format!(
            "the gallery holds up to {MAX_GALLERY} screenshots and videos; remove one first"
        )));
    }
    insert(
        &mut tx,
        Uuid::new_v4(),
        app_id,
        role,
        next,
        youtube.as_deref(),
        caption.as_deref(),
        &pic,
    )
    .await?;
    touch(&mut tx, app_id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(gallery(&state, app_id).await?)))
}

#[derive(Deserialize)]
struct ItemEdit {
    caption: Option<String>,
}

async fn edit_item(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(edit): Json<ItemEdit>,
) -> ApiResult<Json<Vec<GalleryItem>>> {
    let (app_id, _) = owned(&state, &slug, &user).await?;
    let caption = validate_caption(edit.caption.as_deref())?;
    let done = sqlx::query(
        "update app_media set caption = $3 where id = $1 and app_id = $2 and role <> 'thumbnail'",
    )
    .bind(id)
    .bind(app_id)
    .bind(caption)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(Json(gallery(&state, app_id).await?))
}

async fn remove_item(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<Vec<GalleryItem>>> {
    let (app_id, _) = owned(&state, &slug, &user).await?;
    let mut tx = state.db.begin().await?;
    let done =
        sqlx::query("delete from app_media where id = $1 and app_id = $2 and role <> 'thumbnail'")
            .bind(id)
            .bind(app_id)
            .execute(&mut *tx)
            .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    touch(&mut tx, app_id).await?;
    tx.commit().await?;
    Ok(Json(gallery(&state, app_id).await?))
}

#[derive(Deserialize)]
struct Order {
    /// Every gallery item's ID, in the new order.
    ids: Vec<Uuid>,
}

async fn reorder(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(order): Json<Order>,
) -> ApiResult<Json<Vec<GalleryItem>>> {
    let (app_id, _) = owned(&state, &slug, &user).await?;
    let mut tx = state.db.begin().await?;
    let mut current: Vec<Uuid> = sqlx::query_scalar(
        "select id from app_media where app_id = $1 and role <> 'thumbnail' for update",
    )
    .bind(app_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut wanted = order.ids.clone();
    current.sort();
    wanted.sort();
    if current != wanted {
        return Err(ApiError::invalid(
            "ids",
            "the gallery changed; reload and try again",
        ));
    }
    for (position, id) in order.ids.iter().enumerate() {
        sqlx::query("update app_media set position = $2 where id = $1")
            .bind(id)
            .bind(position as i16)
            .execute(&mut *tx)
            .await?;
    }
    touch(&mut tx, app_id).await?;
    tx.commit().await?;
    Ok(Json(gallery(&state, app_id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_links() {
        let id = |s: &str| youtube_id(s);
        for link in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=10",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
        ] {
            assert_eq!(id(link).as_deref(), Some("dQw4w9WgXcQ"), "{link}");
        }
        for link in [
            "https://vimeo.com/123",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/channel/UCabcdefghijk",
            "https://evil.example/watch?v=dQw4w9WgXcQ",
            "javascript:alert(1)",
        ] {
            assert_eq!(id(link), None, "{link}");
        }
    }

    fn png(w: u32, h: u32, alpha: u8) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 40, 90, alpha]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn pictures_are_reencoded_to_fit() {
        let thumb = process_now("image", &png(1000, 1000, 255), Fit::Fill(920, 430)).unwrap();
        assert_eq!((thumb.width, thumb.height), (920, 430));
        assert_eq!(thumb.content_type, "image/jpeg");

        let shot = process_now(
            "image",
            &png(3840, 1080, 128),
            Fit::Within(SHOT_MAX, SHOT_MIN),
        )
        .unwrap();
        assert_eq!((shot.width, shot.height), (1920, 540));
        assert_eq!(shot.content_type, "image/png", "transparency is kept");

        assert!(
            process_now(
                "image",
                &png(100, 100, 255),
                Fit::Within(SHOT_MAX, SHOT_MIN)
            )
            .is_err()
        );
        assert!(process_now("image", b"<svg onload=alert(1)>", Fit::Fill(920, 430)).is_err());
    }
}
