//! Slop's legal documents, their versions, and who accepted which (LEGAL §11, LEGAL-GUIDANCE §14
//! and §15).
//!
//! The documents live in `crates/slop-api/legal/*.md`, each with a version and a summary of what
//! changed. On start-up `sync` publishes any version the database doesn't have yet. A document's
//! first version takes effect at once; every later one takes effect **15 days** after it's
//! published, so users and business users get notice (P2B Regulation Art. 3(2)). A published
//! version is never changed: edit the file and give it a new version.
//!
//! - **Terms of Service:** accepted at sign-up.
//! - **Publisher Agreement:** accepted before a first app is created.
//! - **People Terms:** accepted before making a creative profile or hiring.
//! - **Privacy Policy** and **Content Policy:** read, not accepted; changes are announced.
//!
//! Once a new version is in effect, the actions behind a document wait until it's accepted
//! (`require`); everything else carries on, with a notice.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
};

/// How long before a changed document takes effect.
pub const NOTICE: Duration = Duration::days(15);

/// (kind, file, accepted rather than only read)
const DOCUMENTS: &[(&str, &str, bool)] = &[
    ("terms", include_str!("../legal/terms.md"), true),
    ("privacy", include_str!("../legal/privacy.md"), false),
    ("content", include_str!("../legal/content.md"), false),
    ("publisher", include_str!("../legal/publisher.md"), true),
    ("people", include_str!("../legal/people.md"), true),
];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/legal", get(index))
        .route("/legal/{kind}", get(document))
        .route("/me/legal", get(my_notices))
        .route("/me/legal/accept", post(accept))
}

#[derive(Debug, PartialEq)]
pub struct Parsed<'a> {
    pub title: &'a str,
    pub version: &'a str,
    pub summary: &'a str,
    pub body: &'a str,
}

/// Splits a document's header (`title:`, `version:`, `summary:` between `---` lines) from its body.
pub fn parse(file: &str) -> anyhow::Result<Parsed<'_>> {
    let rest = file
        .strip_prefix("---\n")
        .ok_or_else(|| anyhow::anyhow!("a legal document starts with a --- header"))?;
    let (header, body) = rest
        .split_once("\n---\n")
        .ok_or_else(|| anyhow::anyhow!("the header ends with ---"))?;
    let field = |name: &str| {
        header
            .lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("the header needs {name}"))
    };
    let version = field("version")?;
    anyhow::ensure!(
        version.len() >= 10
            && time::Date::parse(
                &version[..10],
                time::macros::format_description!("[year]-[month]-[day]")
            )
            .is_ok()
            && version[10..].chars().all(|c| c.is_ascii_lowercase())
            && version.len() <= 11,
        "a version is a date, optionally followed by one letter (2026-10-20b)"
    );
    Ok(Parsed {
        title: field("title")?,
        version,
        summary: field("summary")?,
        body: body.trim(),
    })
}

fn sha256(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// Publishes any document version the database doesn't have yet.
pub async fn sync(db: &PgPool) -> anyhow::Result<()> {
    for (kind, file, _) in DOCUMENTS {
        let doc = parse(file)?;
        let hash = sha256(doc.body);
        let existing: Option<String> = sqlx::query_scalar(
            "select sha256 from legal_documents where kind = $1 and version = $2",
        )
        .bind(kind)
        .bind(doc.version)
        .fetch_optional(db)
        .await?;
        match existing {
            Some(h) if h == hash => continue,
            Some(_) => {
                // Users accepted the words as published; they can't change under them.
                tracing::error!(
                    kind,
                    version = doc.version,
                    "a published legal document was edited without a new version; keeping the published text"
                );
                continue;
            }
            None => {}
        }
        let first: bool =
            sqlx::query_scalar("select not exists (select 1 from legal_documents where kind = $1)")
                .bind(kind)
                .fetch_one(db)
                .await?;
        let notice = if first { Duration::ZERO } else { NOTICE };
        sqlx::query(
            "insert into legal_documents (kind, version, title, summary, body_md, sha256, effective_at)
             values ($1, $2, $3, $4, $5, $6, now() + $7)",
        )
        .bind(kind)
        .bind(doc.version)
        .bind(doc.title)
        .bind(doc.summary)
        .bind(doc.body)
        .bind(&hash)
        .bind(notice)
        .execute(db)
        .await?;
        tracing::info!(
            kind,
            version = doc.version,
            first,
            "published a legal document"
        );
    }
    Ok(())
}

fn accepted_kind(kind: &str) -> bool {
    DOCUMENTS.iter().any(|(k, _, a)| *k == kind && *a)
}

pub fn title(kind: &str) -> &'static str {
    match kind {
        "terms" => "Terms of Service",
        "privacy" => "Privacy Policy",
        "content" => "Content Policy",
        "publisher" => "Publisher Agreement",
        "people" => "People Terms",
        _ => "document",
    }
}

/// The version in effect now.
pub async fn current_version(
    conn: &mut sqlx::PgConnection,
    kind: &str,
) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar(
        "select version from legal_documents where kind = $1 and effective_at <= now()
         order by effective_at desc, version desc limit 1",
    )
    .bind(kind)
    .fetch_optional(conn)
    .await
}

/// Refuses an action until the user has accepted the version of `kind` in effect now.
pub async fn require(db: &PgPool, user_id: Uuid, kind: &'static str) -> ApiResult<()> {
    let ok: bool = sqlx::query_scalar(
        "select coalesce((
             select exists (select 1 from legal_acceptances a
                            where a.user_id = $1 and a.kind = d.kind and a.version = d.version)
             from legal_documents d where d.kind = $2 and d.effective_at <= now()
             order by d.effective_at desc, d.version desc limit 1), true)",
    )
    .bind(user_id)
    .bind(kind)
    .fetch_one(db)
    .await?;
    if ok {
        Ok(())
    } else {
        Err(ApiError::TermsRequired(kind))
    }
}

/// Records that the user accepted a version (the current one, or one announced for later).
pub async fn record_acceptance(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    kind: &str,
    version: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "insert into legal_acceptances (user_id, kind, version) values ($1, $2, $3)
         on conflict do nothing",
    )
    .bind(user_id)
    .bind(kind)
    .bind(version)
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(Serialize, sqlx::FromRow)]
struct VersionInfo {
    version: String,
    summary: String,
    #[serde(with = "time::serde::rfc3339")]
    published_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    effective_at: OffsetDateTime,
}

#[derive(Serialize)]
struct IndexEntry {
    kind: &'static str,
    title: &'static str,
    accepted: bool,
    current: Option<VersionInfo>,
    /// A change that's been announced and takes effect later.
    upcoming: Option<VersionInfo>,
}

async fn versions(db: &PgPool, kind: &str) -> sqlx::Result<Vec<VersionInfo>> {
    sqlx::query_as(
        "select version, summary, published_at, effective_at from legal_documents
         where kind = $1 order by effective_at desc, version desc",
    )
    .bind(kind)
    .fetch_all(db)
    .await
}

fn split(mut all: Vec<VersionInfo>) -> (Option<VersionInfo>, Option<VersionInfo>) {
    let now = OffsetDateTime::now_utc();
    let upcoming = match all.first() {
        Some(v) if v.effective_at > now => Some(all.remove(0)),
        _ => None,
    };
    let current = all.into_iter().find(|v| v.effective_at <= now);
    (current, upcoming)
}

async fn index(State(state): State<AppState>) -> ApiResult<Json<Vec<IndexEntry>>> {
    let mut out = Vec::new();
    for (kind, _, accepted) in DOCUMENTS {
        let (current, upcoming) = split(versions(&state.db, kind).await?);
        out.push(IndexEntry {
            kind,
            title: title(kind),
            accepted: *accepted,
            current,
            upcoming,
        });
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
struct DocQuery {
    version: Option<String>,
}

#[derive(Serialize)]
struct Document {
    kind: String,
    title: String,
    version: String,
    summary: String,
    body_md: String,
    sha256: String,
    #[serde(with = "time::serde::rfc3339")]
    published_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    effective_at: OffsetDateTime,
    in_effect: bool,
    /// Every version, newest first.
    versions: Vec<VersionInfo>,
}

/// One document: the version in effect, or a named one (past or announced).
async fn document(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    Query(q): Query<DocQuery>,
) -> ApiResult<Json<Document>> {
    type Row = (
        String,
        String,
        String,
        String,
        String,
        OffsetDateTime,
        OffsetDateTime,
    );
    let row: Option<Row> = sqlx::query_as(
        "select version, title, summary, body_md, sha256, published_at, effective_at
         from legal_documents where kind = $1 and ($2::text is null or version = $2)
           and ($2::text is not null or effective_at <= now())
         order by effective_at desc, version desc limit 1",
    )
    .bind(&kind)
    .bind(&q.version)
    .fetch_optional(&state.db)
    .await?;
    let (version, title, summary, body_md, sha256, published_at, effective_at) =
        row.ok_or(ApiError::NotFound)?;
    Ok(Json(Document {
        in_effect: effective_at <= OffsetDateTime::now_utc(),
        versions: versions(&state.db, &kind).await?,
        kind,
        title,
        version,
        summary,
        body_md,
        sha256,
        published_at,
        effective_at,
    }))
}

#[derive(Serialize)]
struct Notice {
    kind: &'static str,
    title: &'static str,
    version: String,
    summary: String,
    #[serde(with = "time::serde::rfc3339")]
    effective_at: OffsetDateTime,
    in_effect: bool,
    /// "accept" for agreements the user is party to, "read" for policies.
    action: &'static str,
}

/// What the signed-in user should be told about: agreements they're party to with a version
/// they haven't accepted (in effect or announced), and policy changes since they joined.
async fn my_notices(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<Notice>>> {
    let joined: OffsetDateTime = sqlx::query_scalar("select created_at from users where id = $1")
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;
    let now = OffsetDateTime::now_utc();
    let mut out = Vec::new();
    for (kind, _, accepted) in DOCUMENTS {
        let party: bool = !accepted
            || *kind == "terms"
            || sqlx::query_scalar(
                "select exists (select 1 from legal_acceptances where user_id = $1 and kind = $2)",
            )
            .bind(user.id)
            .bind(kind)
            .fetch_one(&state.db)
            .await?;
        if !party {
            continue;
        }
        let (current, upcoming) = split(versions(&state.db, kind).await?);
        for v in [current, upcoming].into_iter().flatten() {
            let in_effect = v.effective_at <= now;
            if *accepted {
                let done: bool = sqlx::query_scalar(
                    "select exists (select 1 from legal_acceptances
                                    where user_id = $1 and kind = $2 and version = $3)",
                )
                .bind(user.id)
                .bind(kind)
                .bind(&v.version)
                .fetch_one(&state.db)
                .await?;
                if done {
                    continue;
                }
            } else if v.published_at <= joined
                || (in_effect && v.effective_at < now - Duration::days(30))
            {
                // Policies the user saw when they joined, or changes long since announced.
                continue;
            }
            out.push(Notice {
                kind,
                title: title(kind),
                version: v.version,
                summary: v.summary,
                effective_at: v.effective_at,
                in_effect,
                action: if *accepted { "accept" } else { "read" },
            });
        }
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptBody {
    kind: String,
    version: String,
}

async fn accept(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<AcceptBody>,
) -> ApiResult<StatusCode> {
    if !accepted_kind(&body.kind) {
        return Err(ApiError::invalid(
            "kind",
            "that document isn't one you accept",
        ));
    }
    // The version in effect now, or one announced for later. Never an old one.
    let mut conn = state.db.acquire().await?;
    let current = current_version(&mut conn, &body.kind).await?;
    let acceptable: bool = sqlx::query_scalar(
        "select exists (select 1 from legal_documents where kind = $1 and version = $2
                        and (effective_at > now() or version = $3))",
    )
    .bind(&body.kind)
    .bind(&body.version)
    .bind(current)
    .fetch_one(&mut *conn)
    .await?;
    if !acceptable {
        return Err(ApiError::invalid(
            "version",
            "that isn't the current version; reload and read it again",
        ));
    }
    record_acceptance(&mut conn, user.id, &body.kind, &body.version).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_document_parses_and_says_it_is_a_draft_until_the_lawyer_has_read_it() {
        for (kind, file, _) in DOCUMENTS {
            let doc = parse(file).unwrap_or_else(|e| panic!("{kind}: {e}"));
            assert!(doc.body.len() > 500, "{kind} has a body");
            assert!(!doc.body.contains("---\n"), "{kind}: one header only");
        }
    }

    #[test]
    fn headers_are_checked() {
        let ok = "---\ntitle: T\nversion: 2026-10-20b\nsummary: S\n---\nBody";
        assert_eq!(
            parse(ok).unwrap(),
            Parsed {
                title: "T",
                version: "2026-10-20b",
                summary: "S",
                body: "Body"
            }
        );
        for bad in [
            "title: T\n---\nBody",
            "---\ntitle: T\nversion: soon\nsummary: S\n---\nBody",
            "---\ntitle: T\nversion: 2026-10-20\n---\nBody",
            "---\ntitle: T\nversion: 2026-10-20bb\nsummary: S\n---\nBody",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
