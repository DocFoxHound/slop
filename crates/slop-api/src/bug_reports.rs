// SPDX-License-Identifier: AGPL-3.0-or-later
//! Feedback for makers (plan phase 13): players' bug reports, and what changed with each release.
//!
//! - **Bug reports** go from a player who launched the app to its maker: what's wrong, the
//!   version and platform, and, if the player chooses, the last lines of the game's output from
//!   a crash (the Slop app offers them, with the home folder blanked out). They're private
//!   between the player, the maker and staff; logs are deleted after 90 days (retention.rs).
//!   Players with the same problem add "me too" to an open report instead of filing another.
//!   Makers mark reports fixed, won't fix or duplicate, and the player is told. Makers can turn
//!   reports off on the store page.
//! - **Issues stay the maker's:** Slop doesn't write to repositories. The website offers a
//!   prefilled GitHub issue, and a maker's AI agent can read reports and file them itself.
//! - **Feedback per release** (`GET /v1/apps/{slug}/release-feedback`): for each recent release,
//!   its % Not Slop from votes cast on it, the reviews written while it was current with their
//!   most common phrases (counted, no AI), and its bug reports.

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    notify::{self, Notice},
};

/// Open reports one player may have on one app.
const MAX_OPEN_PER_PLAYER: i64 = 10;
/// How many releases the feedback covers.
const FEEDBACK_RELEASES: i64 = 5;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/bug-reports", get(list).post(file))
        .route("/apps/{slug}/release-feedback", get(release_feedback))
        .route("/bug-reports/{id}", get(detail).patch(update))
        .route("/bug-reports/{id}/me-too", post(me_too).delete(not_me))
}

/// The app's ID, its maker, and whether it takes bug reports.
async fn app_of(state: &AppState, slug: &str) -> ApiResult<(Uuid, Uuid, bool, String)> {
    let row: Option<(Uuid, Uuid, bool, String)> =
        sqlx::query_as("select id, maker_id, bug_reports, title from apps where slug = $1")
            .bind(slug)
            .fetch_optional(&state.db)
            .await?;
    row.ok_or(ApiError::NotFound)
}

async fn has_played(state: &AppState, user: Uuid, app: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select exists (select 1 from user_app_usage where user_id = $1 and app_id = $2)",
    )
    .bind(user)
    .bind(app)
    .fetch_one(&state.db)
    .await
}

#[derive(Serialize, sqlx::FromRow)]
struct Report {
    id: Uuid,
    title: String,
    status: String,
    version: Option<String>,
    platform: Option<String>,
    me_too: i64,
    mine: bool,
    i_have_it_too: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    /// Only for the maker and the player who filed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    log: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    maker_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reporter: Option<String>,
}

#[derive(Serialize)]
struct ReportList {
    reports: Vec<Report>,
    /// The viewer makes the app: they see every report in full.
    is_maker: bool,
    /// The viewer can file one (they launched it, and the maker takes reports).
    can_report: bool,
    accepting: bool,
}

#[derive(Deserialize)]
struct ListQuery {
    /// open (default), fixed, wont_fix, duplicate or all.
    status: Option<String>,
}

/// SQL columns for a report as `viewer` ($2) may see it; `full` when they're the app's maker.
fn columns(full: bool) -> String {
    let private = |col: &str| {
        if full {
            format!("r.{col}")
        } else {
            format!("case when r.reporter_id = $2 then r.{col} end")
        }
    };
    format!(
        "r.id, r.title, r.status, r.version, r.platform,
         (select count(*) from bug_report_votes v where v.report_id = r.id) as me_too,
         r.reporter_id = $2 as mine,
         exists (select 1 from bug_report_votes v where v.report_id = r.id and v.user_id = $2) as i_have_it_too,
         r.created_at, {} as body, {} as log, {} as maker_note, {} as reporter",
        private("body"),
        private("log"),
        private("maker_note"),
        if full { "(select display_name from users u where u.id = r.reporter_id)".to_string() } else { "null::text".into() },
    )
}

/// The maker sees every report; a player sees the open ones' titles (to say "me too") and
/// their own in full.
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<ReportList>> {
    let (app_id, maker, accepting, _) = app_of(&state, &slug).await?;
    let is_maker = maker == user.id;
    let played = has_played(&state, user.id, app_id).await?;
    if !is_maker && !played {
        return Err(ApiError::NotFound);
    }
    let status = q.status.as_deref().unwrap_or("open");
    if !["open", "fixed", "wont_fix", "duplicate", "all"].contains(&status) {
        return Err(ApiError::invalid(
            "status",
            "open, fixed, wont_fix, duplicate or all",
        ));
    }
    let reports = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {} from bug_reports r where r.app_id = $1 and ($3 = 'all' or r.status = $3)
           and ($4 or r.status = 'open' or r.reporter_id = $2)
         order by me_too desc, r.created_at desc limit 200",
        columns(is_maker)
    )))
    .bind(app_id)
    .bind(user.id)
    .bind(status)
    .bind(is_maker)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(ReportList {
        reports,
        is_maker,
        can_report: played && accepting,
        accepting,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewReport {
    title: String,
    body: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    platform: Option<String>,
    /// The game's last output lines, only when the player chose to send them.
    #[serde(default)]
    log: Option<String>,
}

async fn file(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<NewReport>,
) -> ApiResult<(StatusCode, Json<Report>)> {
    crate::limits::check(&state, user.id, crate::limits::Action::Review)?;
    let (app_id, maker, accepting, app_title) = app_of(&state, &slug).await?;
    if !has_played(&state, user.id, app_id).await? {
        return Err(ApiError::Conflict(
            "play it through Slop first, so the maker knows which build you ran".into(),
        ));
    }
    if !accepting {
        return Err(ApiError::Conflict(
            "this app's maker takes bug reports elsewhere".into(),
        ));
    }
    let title = b.title.trim();
    let body = b.body.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(ApiError::invalid("title", "1 to 120 characters"));
    }
    if body.is_empty() || body.chars().count() > 4000 {
        return Err(ApiError::invalid("body", "1 to 4,000 characters"));
    }
    let version = b
        .version
        .map(|v| v.trim().chars().take(40).collect::<String>())
        .filter(|v| !v.is_empty());
    if let Some(p) = &b.platform
        && !["windows", "linux", "mac", "android"].contains(&p.as_str())
    {
        return Err(ApiError::invalid(
            "platform",
            "windows, linux, mac or android",
        ));
    }
    let log = b.log.map(|l| {
        // Keep the end: that's where a crash is.
        let chars: Vec<char> = l.chars().collect();
        chars[chars.len().saturating_sub(16_000)..]
            .iter()
            .collect::<String>()
    });
    let open: i64 = sqlx::query_scalar(
        "select count(*) from bug_reports where app_id = $1 and reporter_id = $2 and status = 'open'",
    )
    .bind(app_id)
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    if open >= MAX_OPEN_PER_PLAYER {
        return Err(ApiError::Conflict(format!(
            "you have {MAX_OPEN_PER_PLAYER} open reports on this app already"
        )));
    }
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into bug_reports (id, app_id, reporter_id, version, platform, title, body, log)
         values ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(app_id)
    .bind(user.id)
    .bind(&version)
    .bind(&b.platform)
    .bind(title)
    .bind(body)
    .bind(log.filter(|l| !l.trim().is_empty()))
    .execute(&mut *tx)
    .await?;
    if maker != user.id {
        notify::send(
            &mut tx,
            Notice {
                user_id: maker,
                category: "apps",
                title: format!("A bug report on {app_title}"),
                body: title.to_string(),
                link: format!("/publish/{slug}/manage#bug-reports"),
                collapse_key: Some(format!("bugs:{app_id}")),
            },
        )
        .await?;
    }
    let report = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {} from bug_reports r where r.id = $1",
        columns(false)
    )))
    .bind(id)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(report)))
}

/// A report, for its maker or the player who filed it.
async fn load(
    state: &AppState,
    id: Uuid,
    user: &CurrentUser,
) -> ApiResult<(Report, Uuid, String, bool)> {
    let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "select a.maker_id, r.reporter_id, a.slug from bug_reports r join apps a on a.id = r.app_id where r.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (maker, reporter, slug) = row.ok_or(ApiError::NotFound)?;
    let is_maker = maker == user.id;
    if !is_maker && reporter != user.id {
        return Err(ApiError::NotFound);
    }
    let report = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {} from bug_reports r where r.id = $1",
        columns(is_maker)
    )))
    .bind(id)
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    Ok((report, reporter, slug, is_maker))
}

async fn detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Report>> {
    Ok(Json(load(&state, id, &user).await?.0))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    status: Option<String>,
    maker_note: Option<String>,
}

/// The maker marks a report fixed, won't fix or duplicate (or open again), with a note the
/// player sees. The player, and when it's fixed everyone who said "me too", are told.
async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(b): Json<Change>,
) -> ApiResult<Json<Report>> {
    let (_, reporter, slug, is_maker) = load(&state, id, &user).await?;
    if !is_maker {
        return Err(ApiError::NotFound);
    }
    if let Some(s) = &b.status
        && !["open", "fixed", "wont_fix", "duplicate"].contains(&s.as_str())
    {
        return Err(ApiError::invalid(
            "status",
            "open, fixed, wont_fix or duplicate",
        ));
    }
    let note = b.maker_note.as_deref().map(str::trim);
    if note.is_some_and(|n| n.chars().count() > 2000) {
        return Err(ApiError::invalid("maker_note", "up to 2,000 characters"));
    }
    let mut tx = state.db.begin().await?;
    let (title, before): (String, String) =
        sqlx::query_as("select title, status from bug_reports where id = $1 for update")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(
        "update bug_reports set status = coalesce($2, status),
                maker_note = case when $3::text is null then maker_note else nullif($3, '') end,
                updated_at = now()
         where id = $1",
    )
    .bind(id)
    .bind(&b.status)
    .bind(note)
    .execute(&mut *tx)
    .await?;
    if let Some(status) = b.status.as_deref().filter(|s| *s != before) {
        let mut tell = vec![reporter];
        if status == "fixed" {
            tell.extend(
                sqlx::query_scalar::<_, Uuid>(
                    "select user_id from bug_report_votes where report_id = $1",
                )
                .bind(id)
                .fetch_all(&mut *tx)
                .await?,
            );
        }
        let what = match status {
            "fixed" => "was fixed",
            "wont_fix" => "won't be fixed",
            "duplicate" => "is a duplicate of another report",
            _ => "is open again",
        };
        for who in tell.into_iter().filter(|w| *w != user.id) {
            notify::send(
                &mut tx,
                Notice {
                    user_id: who,
                    category: "apps",
                    title: format!("Your bug report {what}"),
                    body: format!(
                        "\"{title}\"{}",
                        note.map(|n| format!(": {n}")).unwrap_or_default()
                    ),
                    link: format!("/app/{slug}#bug-reports"),
                    collapse_key: Some(format!("bug:{id}")),
                },
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(Json(load(&state, id, &user).await?.0))
}

async fn me_too(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let row: Option<(Uuid, Uuid)> = sqlx::query_as(
        "select app_id, reporter_id from bug_reports where id = $1 and status = 'open'",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (app_id, reporter) = row.ok_or(ApiError::NotFound)?;
    if !has_played(&state, user.id, app_id).await? {
        return Err(ApiError::NotFound);
    }
    if reporter == user.id {
        return Err(ApiError::Conflict("it's your own report".into()));
    }
    sqlx::query(
        "insert into bug_report_votes (report_id, user_id) values ($1, $2) on conflict do nothing",
    )
    .bind(id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn not_me(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    sqlx::query("delete from bug_report_votes where report_id = $1 and user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Feedback per release

#[derive(Serialize)]
struct ReleaseFeedback {
    version: String,
    #[serde(with = "time::serde::rfc3339")]
    approved_at: OffsetDateTime,
    /// Votes cast while players had this version: how many, and % Not Slop (unweighted).
    votes: i64,
    not_slop_percent: Option<i64>,
    /// Reviews written or changed while it was the newest release.
    reviews_up: i64,
    reviews_down: i64,
    /// The phrases those reviews use most, with how many reviews use each.
    praised: Vec<(String, i64)>,
    criticised: Vec<(String, i64)>,
    bug_reports: i64,
}

async fn release_feedback(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<ReleaseFeedback>>> {
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let releases: Vec<(String, OffsetDateTime)> = sqlx::query_as(
        "select version, reviewed_at from releases
         where app_id = $1 and status in ('approved', 'revoked') and reviewed_at is not null
         order by reviewed_at desc limit $2",
    )
    .bind(app_id)
    .bind(FEEDBACK_RELEASES)
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::new();
    for (i, (version, approved_at)) in releases.iter().enumerate() {
        // Until the next (newer) release, or now.
        let until = if i == 0 {
            None
        } else {
            Some(releases[i - 1].1)
        };
        let (votes, up): (i64, i64) = sqlx::query_as(
            "select count(*), count(*) filter (where vote = 1) from (
                 select distinct on (user_id) vote from vote_events
                 where app_id = $1 and release_version = $2 order by user_id, created_at desc) v
             where vote <> 0",
        )
        .bind(app_id)
        .bind(version)
        .fetch_one(&state.db)
        .await?;
        let reviews: Vec<(String, i16)> = sqlx::query_as(
            "select r.body, v.vote from app_reviews r join votes v on v.user_id = r.user_id and v.app_id = r.app_id
             where r.app_id = $1 and r.hidden_at is null and r.updated_at >= $2 and ($3::timestamptz is null or r.updated_at < $3)",
        )
        .bind(app_id)
        .bind(approved_at)
        .bind(until)
        .fetch_all(&state.db)
        .await?;
        let bug_reports: i64 = sqlx::query_scalar(
            "select count(*) from bug_reports where app_id = $1 and version = $2",
        )
        .bind(app_id)
        .bind(version)
        .fetch_one(&state.db)
        .await?;
        let texts = |vote: i16| {
            reviews
                .iter()
                .filter(move |(_, v)| *v == vote)
                .map(|(b, _)| b.as_str())
        };
        out.push(ReleaseFeedback {
            version: version.clone(),
            approved_at: *approved_at,
            votes,
            not_slop_percent: (votes > 0).then(|| (100 * up + votes / 2) / votes),
            reviews_up: texts(1).count() as i64,
            reviews_down: texts(-1).count() as i64,
            praised: common_phrases(texts(1), 6),
            criticised: common_phrases(texts(-1), 6),
            bug_reports,
        });
    }
    Ok(Json(out))
}

/// Words that say little on their own.
const STOP: &[&str] = &[
    "a", "about", "after", "again", "all", "also", "am", "an", "and", "any", "are", "as", "at",
    "be", "because", "been", "but", "by", "can", "could", "did", "do", "does", "don't", "even",
    "for", "from", "game", "get", "got", "had", "has", "have", "he", "her", "his", "how", "i",
    "i'm", "if", "in", "into", "is", "it", "it's", "its", "just", "like", "me", "more", "my", "no",
    "not", "now", "of", "on", "one", "only", "or", "other", "our", "out", "really", "so", "some",
    "still", "than", "that", "the", "their", "them", "then", "there", "they", "this", "to", "too",
    "up", "very", "was", "we", "were", "what", "when", "which", "while", "who", "will", "with",
    "would", "you", "your", "app", "play", "played", "playing",
];

/// The phrases (single words and pairs of words) used in the most texts, at least twice each;
/// a pair beats its own words when it's as common.
pub fn common_phrases<'a>(texts: impl Iterator<Item = &'a str>, top: usize) -> Vec<(String, i64)> {
    let mut counts: HashMap<String, i64> = HashMap::new();
    for text in texts {
        let lower = text.to_lowercase();
        let words: Vec<&str> = lower
            .split(|c: char| !(c.is_alphanumeric() || c == '\''))
            .map(|w| w.trim_matches('\''))
            .filter(|w| w.chars().count() > 1)
            .collect();
        let mut seen: Vec<String> = Vec::new();
        for (i, w) in words.iter().enumerate() {
            if STOP.contains(w) {
                continue;
            }
            seen.push((*w).to_string());
            if let Some(next) = words.get(i + 1).filter(|n| !STOP.contains(n)) {
                seen.push(format!("{w} {next}"));
            }
        }
        seen.sort();
        seen.dedup();
        for phrase in seen {
            *counts.entry(phrase).or_default() += 1;
        }
    }
    let pairs: Vec<(String, i64)> = counts
        .iter()
        .filter(|(p, _)| p.contains(' '))
        .map(|(p, n)| (p.clone(), *n))
        .collect();
    let mut list: Vec<(String, i64)> = counts
        .into_iter()
        .filter(|(p, n)| {
            *n >= 2
                && (p.contains(' ')
                    || !pairs
                        .iter()
                        .any(|(pair, m)| m >= n && pair.split(' ').any(|w| w == p)))
        })
        .collect();
    list.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(b.0.contains(' ').cmp(&a.0.contains(' ')))
            .then(a.0.cmp(&b.0))
    });
    list.truncate(top);
    list
}

#[cfg(test)]
mod tests {
    use super::common_phrases;

    #[test]
    fn common_phrases_count_reviews_not_repeats() {
        let reviews = [
            "The new jump feels floaty. Floaty floaty floaty.",
            "Love the music, but the jump feels floaty now.",
            "Great music and the levels are fun.",
        ];
        let top = common_phrases(reviews.iter().copied(), 4);
        assert_eq!(top[0], ("feels floaty".to_string(), 2));
        assert!(top.contains(&("music".to_string(), 2)));
        // "jump feels" is as common as "jump" and "feels": the pairs stand in for the words.
        assert!(!top.iter().any(|(p, _)| p == "feels"));
        assert!(common_phrases(["only once"].iter().copied(), 5).is_empty());
    }
}
