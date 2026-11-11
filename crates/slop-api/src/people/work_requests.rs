// SPDX-License-Identifier: AGPL-3.0-or-later
//! Open requests on People (DESIGN §15.9): a board where a maker says what one of their store
//! projects needs (one skill, a title, a description, an optional date; never a budget), and
//! creatives who can be hired put their name forward.
//!
//! - Posting: signed in, 18 or older, People Terms accepted, nothing owed on finished work
//!   (`jobs::require_nothing_owed`); at most 3 open at once (`MAX_OPEN`, held exactly by an
//!   advisory lock per account) and a per-account daily limit (limits.rs `WorkRequest`). A
//!   request runs out after 30 days (`expire`), or sooner when its maker closes it.
//! - Reading: everyone the directory is open to, signed out too, newest first and nothing else,
//!   optionally one skill. A request shows only while it is open, its maker's account is open and
//!   its project is in the store for the viewer (`push_visible`). Two people with a block between
//!   them are not shown each other's requests (`push_not_blocked`); hiring doesn't read blocks.
//! - Putting a name forward is a bare pointer to the creative's People page, optionally naming one
//!   of their packages for the skill: no text. Only the maker sees who did, in the order received
//!   (at most 100 at a time), and is told in the jobs category: one notice per request while it's
//!   unread. A name taken back keeps its row, so putting it forward again tells nobody.
//! - SLAPorSLOP never matches, ranks, suggests or tells creatives about a request, and sets no
//!   time to answer. The board creates no job: hiring is the hire request (jobs.rs).

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Postgres, QueryBuilder};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::{Card, CardRow, OPEN_SQL, SKILLS, Service, Skill, skill};
use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog,
    error::{ApiError, ApiResult},
    limits::{self, Action},
    notify,
    policy::ViewerPolicy,
};

/// Open requests one account may have at once.
const MAX_OPEN: i64 = 3;
/// Days an ended request (closed or run out) is kept for its maker before it is deleted.
const KEEP_ENDED_DAYS: i32 = 90;
/// Days a hidden request is kept: the appeal window (moderation::APPEAL_WINDOW, 183 days) and a
/// month more.
const KEEP_HIDDEN_DAYS: i32 = 183 + 30;
const PAGE: i64 = 50;
/// Names put forward that come back per request at a time, in the order received.
const NAMES: i64 = 100;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/work-requests", get(board).post(create))
        .route("/work-requests/{id}", get(one))
        .route("/work-requests/{id}/close", post(close))
        .route(
            "/work-requests/{id}/response",
            put(put_forward).delete(take_back),
        )
        .route("/me/work-requests", get(mine))
}

/// A request on the board: open, not run out, not hidden, its maker's account open, and not for
/// a dedicated server (aliases `w`, `a`, `u`). Always followed by the store rule for `a`
/// (`push_visible`). abuse.rs's link scan repeats these conditions.
const ON_BOARD: &str = "w.status = 'open' and w.expires_at > now() and w.hidden_reason is null
     and u.suspended_at is null and u.closed_at is null and a.branch_kind is distinct from 'server'";

/// A signed-in person, or an AI agent's token with People access. Anyone else is treated as a
/// signed-out visitor: no `mine`, no `put_forward`, no `owner`, and no blocks.
fn personal(user: &CurrentUser) -> bool {
    user.agent.as_ref().is_none_or(|a| a.people)
}

/// The caller's id when `personal`.
fn viewer(user: Option<&CurrentUser>) -> Option<Uuid> {
    user.filter(|u| personal(u)).map(|u| u.id)
}

/// ` and not exists (a block, either way, between the viewer and {other})`. Pushes nothing
/// when there is no viewer. `other` is a column of the statement, never input.
fn push_not_blocked(qb: &mut QueryBuilder<Postgres>, viewer: Option<Uuid>, other: &'static str) {
    let Some(v) = viewer else { return };
    qb.push(" and not exists (select 1 from user_blocks b where (b.user_id = ")
        .push_bind(v)
        .push(format!(
            " and b.blocked_id = {other}) or (b.user_id = {other} and b.blocked_id = "
        ))
        .push_bind(v)
        .push("))");
}

/// The one rule for "this request is on the board for this viewer": `ON_BOARD`, its project in
/// the store for the viewer's age view, and no block between the viewer and its maker. Used by
/// the board, by one request for anyone but its maker, by putting a name forward and by reporting
/// (`on_board_for`), and by the requests a creative put their name forward for (`mine`).
fn push_visible(
    qb: &mut QueryBuilder<Postgres>,
    state: &AppState,
    policy: ViewerPolicy,
    viewer: Option<Uuid>,
) {
    qb.push("(").push(ON_BOARD).push(" and ");
    catalog::push_store_visible(qb, state.config.catalog, policy);
    push_not_blocked(qb, viewer, "w.user_id");
    qb.push(")");
}

/// `select <RequestRow columns> from work_requests w join apps a … join users u … left join` the
/// viewer's own response, if any.
fn push_select(
    qb: &mut QueryBuilder<Postgres>,
    state: &AppState,
    policy: ViewerPolicy,
    viewer: Option<Uuid>,
) {
    qb.push(
        "select w.id, w.title, w.description_md, w.skill, w.wanted_by, w.status, w.created_at,
                w.expires_at, w.ended_at, w.hidden_reason, w.user_id, w.expires_at <= now() as past,
                a.slug as app_slug, a.title as app_title, u.display_name as maker,
                r.service_id as my_service_id, r.created_at as my_response_at, ",
    );
    // On the board in general (what its maker is told), and for this viewer.
    qb.push("(").push(ON_BOARD).push(" and ");
    catalog::push_store_visible(qb, state.config.catalog, ViewerPolicy::FULL_VIEW);
    qb.push(") as on_board, ");
    push_visible(qb, state, policy, viewer);
    qb.push(
        " as visible
         from work_requests w
         join apps a on a.id = w.app_id
         join users u on u.id = w.user_id
         left join work_request_responses r
           on r.request_id = w.id and r.withdrawn_at is null and r.creative_id = ",
    )
    .push_bind(viewer);
}

/// One request as a statement's row.
#[derive(sqlx::FromRow)]
pub(crate) struct RequestRow {
    pub(crate) id: Uuid,
    pub(crate) title: String,
    pub(crate) description_md: String,
    skill: String,
    wanted_by: Option<Date>,
    status: String,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    ended_at: Option<OffsetDateTime>,
    hidden_reason: Option<String>,
    /// The maker.
    pub(crate) user_id: Uuid,
    /// Its 30 days are over (the hourly `expire` may not have run yet).
    past: bool,
    app_slug: String,
    app_title: String,
    maker: String,
    my_service_id: Option<String>,
    my_response_at: Option<OffsetDateTime>,
    on_board: bool,
    visible: bool,
}

/// What the maker is told about their request's state: hidden first, then run out (by the
/// clock too), else what the row says.
fn owner_status(status: &str, hidden: bool, past: bool) -> &'static str {
    if hidden {
        "hidden"
    } else if status == "expired" || (status == "open" && past) {
        "expired"
    } else if status == "closed" {
        "closed"
    } else {
        "open"
    }
}

#[derive(Serialize)]
struct WorkRequest {
    id: Uuid,
    title: String,
    description_md: String,
    skill: Skill,
    /// YYYY-MM-DD.
    wanted_by: Option<String>,
    project: Project,
    /// The maker's display name.
    maker: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
    /// The viewer posted it.
    mine: bool,
    /// The viewer put their name forward.
    put_forward: Option<PutForward>,
    /// Only for its maker, and only where a route says so.
    owner: Option<Owner>,
}

#[derive(Serialize, sqlx::FromRow)]
struct Project {
    slug: String,
    title: String,
}

#[derive(Serialize)]
struct PutForward {
    service_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
}

#[derive(Serialize)]
struct Owner {
    /// open, closed, expired or hidden.
    status: &'static str,
    /// Showing on the board (its project in the store); an open request may not be.
    on_board: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    ended_at: Option<OffsetDateTime>,
    hidden_reason: Option<String>,
    /// Who put their name forward, in the order received: at most `NAMES` from the offset asked.
    responses: Vec<Named>,
    responses_total: i64,
}

#[derive(Serialize)]
struct Named {
    person: Card,
    /// The package they named, while it is still one of theirs.
    service: Option<NamedService>,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
}

#[derive(Serialize)]
struct NamedService {
    id: String,
    name: String,
}

impl RequestRow {
    fn into_json(self, viewer: Option<Uuid>, names: Option<(Vec<Named>, i64)>) -> WorkRequest {
        let owner = names.map(|(responses, responses_total)| Owner {
            status: owner_status(&self.status, self.hidden_reason.is_some(), self.past),
            on_board: self.on_board,
            ended_at: self.ended_at,
            hidden_reason: self.hidden_reason.clone(),
            responses,
            responses_total,
        });
        WorkRequest {
            id: self.id,
            title: self.title,
            description_md: self.description_md,
            skill: skill(&self.skill),
            wanted_by: self.wanted_by.map(|d| d.to_string()),
            project: Project {
                slug: self.app_slug,
                title: self.app_title,
            },
            maker: self.maker,
            created_at: self.created_at,
            expires_at: self.expires_at,
            mine: viewer == Some(self.user_id),
            put_forward: self.my_response_at.map(|at| PutForward {
                service_id: self.my_service_id,
                at,
            }),
            owner,
        }
    }
}

/// One request in any state, whoever may see it (the caller decides with `visible`).
async fn load(
    state: &AppState,
    id: Uuid,
    viewer: Option<Uuid>,
    policy: ViewerPolicy,
) -> ApiResult<Option<RequestRow>> {
    let mut qb = QueryBuilder::new("");
    push_select(&mut qb, state, policy, viewer);
    qb.push(" where w.id = ").push_bind(id);
    Ok(qb.build_query_as().fetch_optional(&state.db).await?)
}

/// The request as this viewer may see it on the board (`push_visible`), or None. The one check
/// behind one request for anyone but its maker, putting a name forward, and reporting
/// (moderation.rs).
pub(crate) async fn on_board_for(
    state: &AppState,
    id: Uuid,
    user: Option<&CurrentUser>,
    policy: ViewerPolicy,
) -> ApiResult<Option<RequestRow>> {
    Ok(load(state, id, viewer(user), policy)
        .await?
        .filter(|r| r.visible))
}

#[derive(sqlx::FromRow)]
struct NameRow {
    #[sqlx(flatten)]
    card: CardRow,
    request_id: Uuid,
    put_service_id: Option<String>,
    put_at: OffsetDateTime,
}

/// ` from work_request_responses r join creatives c … where` the names the maker sees: not taken
/// back, the page open to hiring (`OPEN_SQL`), and no block between them. Shared by the list and
/// its total so the two can't drift.
fn push_names_from(qb: &mut QueryBuilder<Postgres>, ids: &[Uuid], maker: Uuid) {
    qb.push(
        " from work_request_responses r join creatives c on c.user_id = r.creative_id
         where r.request_id = any(",
    )
    .push_bind(ids.to_vec())
    .push(") and r.withdrawn_at is null and ")
    .push(OPEN_SQL);
    push_not_blocked(qb, Some(maker), "c.user_id");
}

/// Who put their name forward for each of the maker's requests `ids`: `NAMES` per request from
/// `offset` on, in the order received, and how many there are. Never ordered by anything else.
async fn names(
    state: &AppState,
    ids: &[Uuid],
    maker: Uuid,
    offset: i64,
) -> ApiResult<HashMap<Uuid, (Vec<Named>, i64)>> {
    let mut out: HashMap<Uuid, (Vec<Named>, i64)> =
        ids.iter().map(|id| (*id, (Vec::new(), 0))).collect();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut qb = QueryBuilder::new(
        "with ranked as (
             select r.request_id, r.creative_id, r.service_id, r.created_at,
                    row_number() over (partition by r.request_id
                                       order by r.created_at, r.creative_id) as place",
    );
    push_names_from(&mut qb, ids, maker);
    qb.push(") ");
    super::push_card_select(
        &mut qb,
        ", r.request_id, r.service_id as put_service_id, r.created_at as put_at",
    );
    qb.push(" join ranked r on r.creative_id = c.user_id where r.place > ")
        .push_bind(offset)
        .push(" and r.place <= ")
        .push_bind(offset + NAMES)
        .push(" order by r.request_id, r.place");
    let rows: Vec<NameRow> = qb.build_query_as().fetch_all(&state.db).await?;
    for row in rows {
        // The package's name from the page as it is now, before the row becomes a card.
        let service = row.put_service_id.as_deref().and_then(|id| {
            row.card
                .services
                .iter()
                .find(|s| s.id == id)
                .map(|s| NamedService {
                    id: s.id.clone(),
                    name: s.name.clone(),
                })
        });
        if let Some((list, _)) = out.get_mut(&row.request_id) {
            list.push(Named {
                person: row.card.into_card(&state.config),
                service,
                at: row.put_at,
            });
        }
    }
    let mut qb = QueryBuilder::new("select r.request_id, count(*)");
    push_names_from(&mut qb, ids, maker);
    qb.push(" group by r.request_id");
    let totals: Vec<(Uuid, i64)> = qb.build_query_as().fetch_all(&state.db).await?;
    for (id, n) in totals {
        if let Some((_, total)) = out.get_mut(&id) {
            *total = n;
        }
    }
    Ok(out)
}

/// The maker's requests as JSON with `owner`.
async fn with_owner(
    state: &AppState,
    rows: Vec<RequestRow>,
    maker: Uuid,
    offset: i64,
) -> ApiResult<Vec<WorkRequest>> {
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let mut names = names(state, &ids, maker, offset).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let n = names.remove(&r.id).unwrap_or_default();
            r.into_json(Some(maker), Some(n))
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// The board, one request, and the caller's own

#[derive(Deserialize)]
struct BoardQuery {
    /// One skill's slug; an unknown one matches nothing.
    skill: Option<String>,
    offset: Option<i64>,
}

/// Open requests on the board for this viewer, newest first. Nothing else orders them.
async fn board(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Query(q): Query<BoardQuery>,
) -> ApiResult<Json<Vec<WorkRequest>>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let viewer = viewer(user.as_ref());
    let mut qb = QueryBuilder::new("");
    push_select(&mut qb, &state, policy, viewer);
    qb.push(" where ");
    push_visible(&mut qb, &state, policy, viewer);
    if let Some(s) = q.skill.as_deref().filter(|s| !s.is_empty()) {
        qb.push(" and w.skill = ").push_bind(s.to_string());
    }
    qb.push(" order by w.created_at desc, w.id desc limit ")
        .push_bind(PAGE)
        .push(" offset ")
        .push_bind(q.offset.unwrap_or(0).clamp(0, 10_000));
    let rows: Vec<RequestRow> = qb.build_query_as().fetch_all(&state.db).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| r.into_json(viewer, None))
            .collect(),
    ))
}

#[derive(Deserialize)]
struct OneQuery {
    /// For the maker: the names from this place on (100 at a time).
    responses_offset: Option<i64>,
}

/// One request: for its maker in any state until it is deleted, with who put their name forward;
/// for anyone else only while it is on the board for them.
async fn one(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<OneQuery>,
) -> ApiResult<Json<WorkRequest>> {
    let id: Uuid = id.parse().map_err(|_| ApiError::NotFound)?;
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let viewer = viewer(user.as_ref());
    let row = load(&state, id, viewer, policy)
        .await?
        .ok_or(ApiError::NotFound)?;
    if viewer == Some(row.user_id) {
        let (maker, offset) = (
            row.user_id,
            q.responses_offset.unwrap_or(0).clamp(0, 10_000),
        );
        let mut out = with_owner(&state, vec![row], maker, offset).await?;
        return out.pop().map(Json).ok_or(ApiError::NotFound);
    }
    if !row.visible {
        return Err(ApiError::NotFound);
    }
    Ok(Json(row.into_json(viewer, None)))
}

#[derive(Serialize)]
struct Mine {
    /// The caller's requests in every state: the open ones first, then the rest, each newest
    /// first (at most 50).
    posted: Vec<WorkRequest>,
    /// Requests the caller put their name forward for that are still on the board for them.
    put_forward: Vec<WorkRequest>,
    /// The apps a new request can be for.
    projects: Vec<Project>,
    /// The caller has an app at all (in the store or not).
    has_apps: bool,
    max_open: i64,
}

/// The caller's own requests, the ones they put their name forward for, and their projects. An
/// AI agent's token needs People access (maker_agents.rs).
async fn mine(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Mine>> {
    // A token for some of the maker's apps sees only those (as catalog.rs `my_apps`).
    let only = user
        .agent
        .as_ref()
        .filter(|a| !a.app_ids.is_empty())
        .map(|a| a.app_ids.clone());
    let me = Some(user.id);

    // Their own: the policy doesn't matter, `visible` isn't used.
    let mut qb = QueryBuilder::new("");
    push_select(&mut qb, &state, ViewerPolicy::FULL_VIEW, me);
    qb.push(" where w.user_id = ").push_bind(user.id);
    if let Some(ids) = &only {
        qb.push(" and a.id = any(").push_bind(ids.clone()).push(")");
    }
    // The ones the cap counts first (`create`), so they are always among the 50: the page counts
    // them to offer "Post a request", and each can be closed from there.
    qb.push(
        " order by (w.status = 'open' and w.hidden_reason is null and w.expires_at > now()) desc,
                   w.created_at desc, w.id desc limit ",
    )
    .push_bind(PAGE);
    let rows: Vec<RequestRow> = qb.build_query_as().fetch_all(&state.db).await?;
    let posted = with_owner(&state, rows, user.id, 0).await?;

    let policy =
        ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &HeaderMap::new()).await?;
    let mut qb = QueryBuilder::new("");
    push_select(&mut qb, &state, policy, me);
    qb.push(" where r.creative_id is not null and ");
    push_visible(&mut qb, &state, policy, me);
    qb.push(" order by r.created_at desc limit ")
        .push_bind(PAGE);
    let rows: Vec<RequestRow> = qb.build_query_as().fetch_all(&state.db).await?;
    let put_forward = rows.into_iter().map(|r| r.into_json(me, None)).collect();

    let mut qb = QueryBuilder::new("select a.slug, a.title from apps a where a.maker_id = ");
    qb.push_bind(user.id)
        .push(" and a.branch_kind is distinct from 'server' and ");
    catalog::push_store_visible(&mut qb, state.config.catalog, ViewerPolicy::FULL_VIEW);
    if let Some(ids) = &only {
        qb.push(" and a.id = any(").push_bind(ids.clone()).push(")");
    }
    qb.push(" order by a.title");
    let projects: Vec<Project> = qb.build_query_as().fetch_all(&state.db).await?;

    let has_apps: bool = sqlx::query_scalar(
        "select exists (select 1 from apps where maker_id = $1
                        and ($2::uuid[] is null or id = any($2)))",
    )
    .bind(user.id)
    .bind(only)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(Mine {
        posted,
        put_forward,
        projects,
        has_apps,
        max_open: MAX_OPEN,
    }))
}

// ---------------------------------------------------------------------------------------------
// Posting and closing

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRequest {
    /// One of the caller's own apps whose page is in the store.
    app_slug: String,
    skill: String,
    title: String,
    description_md: String,
    /// YYYY-MM-DD, from today to two years ahead.
    wanted_by: Option<String>,
    /// The "I'm 18 or older" tick.
    #[serde(default)]
    confirm_adult: bool,
}

/// The maker posts a request. It is public at once.
async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<NewRequest>,
) -> ApiResult<Json<WorkRequest>> {
    limits::check(&state, user.id, Action::WorkRequest)?;
    super::require_adult(&state, &user, body.confirm_adult).await?;
    crate::legal::require(&state.db, user.id, "people").await?;
    super::jobs::require_nothing_owed(&state.db, user.id, "posting a request").await?;
    let mut qb = QueryBuilder::new("select a.id from apps a where a.slug = ");
    qb.push_bind(body.app_slug.clone())
        .push(" and a.maker_id = ")
        .push_bind(user.id)
        .push(" and a.branch_kind is distinct from 'server' and ");
    // The full view: a request may be for an 18+ project (under-18 viewers just don't see it).
    catalog::push_store_visible(&mut qb, state.config.catalog, ViewerPolicy::FULL_VIEW);
    let app_id: Uuid = qb
        .build_query_scalar()
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| {
            ApiError::invalid("app_slug", "pick one of your projects that's in the store")
        })?;
    if !SKILLS.iter().any(|(s, _)| *s == body.skill) {
        return Err(ApiError::invalid("skill", "pick a skill from the list"));
    }
    let title = super::jobs::note("title", &body.title, 100, true)?;
    if title.chars().any(char::is_control) {
        return Err(ApiError::invalid("title", "one line"));
    }
    let description = super::jobs::note("description_md", &body.description_md, 2000, true)?;
    // The hire form's own rule, so "Ask … for a quote" never leaves the text out for it.
    if description
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(ApiError::invalid(
            "description_md",
            "has characters that aren't allowed",
        ));
    }
    let wanted_by = match body.wanted_by.as_deref().map(str::trim) {
        Some(d) if !d.is_empty() => Some(super::jobs::parse_due(
            "wanted_by",
            d,
            OffsetDateTime::now_utc().date(),
        )?),
        _ => None,
    };
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    // Two posts at once by one account wait here, so the cap holds exactly.
    sqlx::query("select pg_advisory_xact_lock(hashtextextended('work_requests:' || $1::text, 0))")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let open: i64 = sqlx::query_scalar(
        "select count(*) from work_requests
         where user_id = $1 and status = 'open' and hidden_reason is null and expires_at > now()",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if open >= MAX_OPEN {
        return Err(ApiError::Conflict(format!(
            "you already have {MAX_OPEN} open requests: close one first"
        )));
    }
    sqlx::query(
        "insert into work_requests (id, user_id, app_id, skill, title, description_md, wanted_by)
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(user.id)
    .bind(app_id)
    .bind(&body.skill)
    .bind(&title)
    .bind(&description)
    .bind(wanted_by)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    own(&state, id, &user).await
}

/// The maker's own request with `owner`, or 404.
async fn own(state: &AppState, id: Uuid, user: &CurrentUser) -> ApiResult<Json<WorkRequest>> {
    // Their own view: the policy doesn't matter, `visible` isn't used.
    let row = load(state, id, Some(user.id), ViewerPolicy::FULL_VIEW)
        .await?
        .filter(|r| r.user_id == user.id)
        .ok_or(ApiError::NotFound)?;
    let mut out = with_owner(state, vec![row], user.id, 0).await?;
    out.pop().map(Json).ok_or(ApiError::NotFound)
}

/// The maker closes their request (also while it is hidden). Closing an ended one changes
/// nothing.
async fn close(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkRequest>> {
    let id: Uuid = id.parse().map_err(|_| ApiError::NotFound)?;
    sqlx::query(
        "update work_requests set status = 'closed', ended_at = now()
         where id = $1 and user_id = $2 and status = 'open'",
    )
    .bind(id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    own(&state, id, &user).await
}

// ---------------------------------------------------------------------------------------------
// Putting a name forward

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseBody {
    /// One of the creative's own packages for the request's skill.
    #[serde(default)]
    service_id: Option<String>,
}

/// Records the name, or changes the package named, in one statement that checks the request is
/// still open (it may have been closed, hidden or run out since it was read). None: it isn't.
/// `bool`: the name is new, so the maker is told. A name taken back and put forward again keeps
/// its first place and time.
///
/// Names on one request take turns (`for no key update`, held until commit): the maker's notice
/// is one unread row per request, so a second name must see the first one's notice to add to it.
/// It doesn't wait on the names' own foreign key (`for key share`).
async fn store_response(
    conn: &mut PgConnection,
    request: Uuid,
    creative: Uuid,
    service: Option<&str>,
) -> sqlx::Result<Option<(OffsetDateTime, bool)>> {
    sqlx::query_as(
        "with w as (
             select id from work_requests
             where id = $1 and status = 'open' and hidden_reason is null and expires_at > now()
             for no key update
         )
         insert into work_request_responses (request_id, creative_id, service_id)
         select w.id, $2, $3 from w
         on conflict (request_id, creative_id) do update
             set service_id = excluded.service_id, withdrawn_at = null
         returning created_at, (xmax = 0) as inserted",
    )
    .bind(request)
    .bind(creative)
    .bind(service)
    .fetch_optional(conn)
    .await
}

/// A creative who can be hired and lists the request's skill puts their name forward. Nothing
/// but a pointer to their page, and the package they name, reaches the maker.
async fn put_forward(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ResponseBody>,
) -> ApiResult<Json<PutForward>> {
    limits::check(&state, user.id, Action::WorkResponse)?;
    crate::legal::require(&state.db, user.id, "people").await?;
    let id: Uuid = id.parse().map_err(|_| ApiError::NotFound)?;
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let request = on_board_for(&state, id, Some(&user), policy)
        .await?
        .ok_or(ApiError::NotFound)?;
    if request.user_id == user.id {
        return Err(ApiError::Conflict("this is your own request".into()));
    }
    let page: Option<(Vec<String>, sqlx::types::Json<Vec<Service>>, bool)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select c.skills, c.services, (c.taking_work and c.stripe_status = 'active')
             from creatives c where c.user_id = $1 and {OPEN_SQL}"
        )))
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?;
    let Some((skills, services, true)) = page else {
        return Err(ApiError::Conflict(
            "your People page has to be live and taking paid work before you can put your name forward"
                .into(),
        ));
    };
    if !skills.contains(&request.skill) {
        return Err(ApiError::Conflict(
            "your People page doesn't list this skill".into(),
        ));
    }
    let service = body.service_id.as_deref().filter(|s| !s.is_empty());
    if let Some(sid) = service
        && !services
            .0
            .iter()
            .any(|s| s.id == sid && s.skill == request.skill)
    {
        return Err(ApiError::invalid(
            "service_id",
            "pick one of your packages for this skill",
        ));
    }
    let mut tx = state.db.begin().await?;
    let at = record_name(
        &mut tx,
        id,
        request.user_id,
        &request.title,
        user.id,
        service,
    )
    .await?
    .ok_or(ApiError::NotFound)?;
    tx.commit().await?;
    Ok(Json(PutForward {
        service_id: service.map(str::to_string),
        at,
    }))
}

/// `store_response`, and the maker's notice when the name is new: what one transaction of
/// `put_forward` does. None: the request isn't open any more.
async fn record_name(
    conn: &mut PgConnection,
    request: Uuid,
    maker: Uuid,
    title: &str,
    creative: Uuid,
    service: Option<&str>,
) -> sqlx::Result<Option<OffsetDateTime>> {
    let Some((at, inserted)) = store_response(conn, request, creative, service).await? else {
        return Ok(None);
    };
    if inserted {
        // The maker only, merged per request while unread. Names nobody, so it stays true at any
        // count.
        notify::send(
            conn,
            notify::Notice {
                user_id: maker,
                category: "jobs",
                title: format!("New names put forward for “{title}”"),
                body: String::new(),
                link: format!("/people?view=requests&request={request}"),
                collapse_key: Some(format!("work_request:{request}")),
            },
        )
        .await?;
    }
    Ok(Some(at))
}

/// The creative takes their name back: it leaves the maker's list at once. The row stays, so
/// putting it forward again tells nobody. Always 204: nothing is learned from it.
async fn take_back(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let Ok(id) = id.parse::<Uuid>() else {
        return Ok(StatusCode::NO_CONTENT);
    };
    sqlx::query(
        "update work_request_responses set withdrawn_at = now()
         where request_id = $1 and creative_id = $2 and withdrawn_at is null",
    )
    .bind(id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Background

/// Ends open requests whose 30 days are over, and deletes requests kept long enough (a hidden
/// one is kept while an appeal about it waits). Returns how many ran out. Safe to call at any
/// time and from several copies at once.
pub async fn expire(db: &sqlx::PgPool, now: OffsetDateTime) -> anyhow::Result<u64> {
    // A hidden request is left alone, so an appeal can still put it back.
    let ran_out = sqlx::query(
        "update work_requests set status = 'expired', ended_at = $1
         where status = 'open' and hidden_reason is null and expires_at <= $1",
    )
    .bind(now)
    .execute(db)
    .await?
    .rows_affected();
    // Names put forward go with their request (on delete cascade).
    sqlx::query(
        "delete from work_requests
         where hidden_reason is null and ended_at < $1 - make_interval(days => $2)",
    )
    .bind(now)
    .bind(KEEP_ENDED_DAYS)
    .execute(db)
    .await?;
    // Hidden: once the appeal window is over, unless an appeal about it still waits (an undo
    // must find it).
    sqlx::query(
        "delete from work_requests w
         where w.hidden_reason is not null and w.hidden_at < $1 - make_interval(days => $2)
           and not exists (select 1 from moderation_decisions d
                           where d.subject_kind = 'work_request' and d.subject_id = w.id
                             and d.appeal_status = 'pending')",
    )
    .bind(now)
    .bind(KEEP_HIDDEN_DAYS)
    .execute(db)
    .await?;
    Ok(ran_out)
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;

    /// The request may end between the handler's check and the write: the write checks again.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_name_cannot_land_on_a_request_that_just_ended(db: PgPool) {
        let (maker, creative, app) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        for (id, name) in [(maker, "Maker"), (creative, "Creative")] {
            sqlx::query(
                "insert into users (id, display_name, country_code, age_16_confirmed_at)
                 values ($1, $2, 'US', now())",
            )
            .bind(id)
            .bind(name)
            .execute(&db)
            .await
            .unwrap();
        }
        sqlx::query(
            "insert into apps (id, slug, title, summary, kind, stage, status, maker_id, repo_url, license_spdx)
             values ($1, 'tavern', 'Tavern', 'An app', 'game', 'released', 'listed', $2,
                     'https://github.com/x/y', 'MIT')",
        )
        .bind(app)
        .bind(maker)
        .execute(&db)
        .await
        .unwrap();
        let new_request = || async {
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into work_requests (id, user_id, app_id, skill, title, description_md)
                 values ($1, $2, $3, 'music', 'Theme', 'A loop.')",
            )
            .bind(id)
            .bind(maker)
            .bind(app)
            .execute(&db)
            .await
            .unwrap();
            id
        };
        let mut conn = db.acquire().await.unwrap();
        for (what, set) in [
            ("closed", "status = 'closed', ended_at = now()"),
            ("hidden", "hidden_reason = 'Spam', hidden_at = now()"),
            ("run out", "expires_at = now() - interval '1 minute'"),
        ] {
            let id = new_request().await;
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "update work_requests set {set} where id = $1"
            )))
            .bind(id)
            .execute(&db)
            .await
            .unwrap();
            assert_eq!(
                store_response(&mut conn, id, creative, None).await.unwrap(),
                None,
                "{what}"
            );
        }
        let names: i64 = sqlx::query_scalar("select count(*) from work_request_responses")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(names, 0);
        // Open: new, then the same name again (not new, same time).
        let id = new_request().await;
        let (at, new) = store_response(&mut conn, id, creative, Some("a1b2c3d4"))
            .await
            .unwrap()
            .unwrap();
        assert!(new);
        assert_eq!(
            store_response(&mut conn, id, creative, None).await.unwrap(),
            Some((at, false))
        );
    }

    /// Two creatives at the same moment take turns on the request: the second waits until the
    /// first is stored, then adds to the maker's unread notice (one unread row per key) instead
    /// of failing on the first one's.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn two_names_at_once_add_up_in_one_notice(db: PgPool) {
        let (maker, a, b, app, id) = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        for (user, name) in [(maker, "Maker"), (a, "Ada"), (b, "Bo")] {
            sqlx::query(
                "insert into users (id, display_name, country_code, age_16_confirmed_at)
                 values ($1, $2, 'US', now())",
            )
            .bind(user)
            .bind(name)
            .execute(&db)
            .await
            .unwrap();
        }
        sqlx::query(
            "insert into apps (id, slug, title, summary, kind, stage, status, maker_id, repo_url, license_spdx)
             values ($1, 'tavern', 'Tavern', 'An app', 'game', 'released', 'listed', $2,
                     'https://github.com/x/y', 'MIT')",
        )
        .bind(app)
        .bind(maker)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "insert into work_requests (id, user_id, app_id, skill, title, description_md)
             values ($1, $2, $3, 'music', 'Theme', 'A loop.')",
        )
        .bind(id)
        .bind(maker)
        .bind(app)
        .execute(&db)
        .await
        .unwrap();

        // The first name is stored and its notice written, not yet committed.
        let mut first = db.begin().await.unwrap();
        assert!(
            record_name(&mut first, id, maker, "Theme", a, None)
                .await
                .unwrap()
                .is_some()
        );
        // The second, on its own connection meanwhile.
        let second = tokio::spawn({
            let db = db.clone();
            async move {
                let mut tx = db.begin().await?;
                let at = record_name(&mut tx, id, maker, "Theme", b, None).await?;
                tx.commit().await?;
                Ok::<_, sqlx::Error>(at)
            }
        });
        // Once it waits for the first, the first commits.
        loop {
            let waiting: bool = sqlx::query_scalar(
                "select exists (select 1 from pg_stat_activity
                                where datname = current_database() and wait_event_type = 'Lock')",
            )
            .fetch_one(&db)
            .await
            .unwrap();
            if waiting || second.is_finished() {
                break;
            }
            tokio::task::yield_now().await;
        }
        first.commit().await.unwrap();
        assert!(second.await.unwrap().unwrap().is_some());
        let told: Vec<(i32, Option<String>)> = sqlx::query_as(
            "select count, collapse_key from notifications where user_id = $1 and read_at is null",
        )
        .bind(maker)
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(told, [(2, Some(format!("work_request:{id}")))]);
    }

    #[test]
    fn the_maker_is_told_the_state_that_counts() {
        // Hidden wins over everything.
        assert_eq!(owner_status("open", true, false), "hidden");
        assert_eq!(owner_status("closed", true, true), "hidden");
        assert_eq!(owner_status("expired", true, true), "hidden");
        // An open row past its time reads expired before the hourly job runs.
        assert_eq!(owner_status("open", false, true), "expired");
        assert_eq!(owner_status("expired", false, true), "expired");
        // Closed stays closed, even past its time.
        assert_eq!(owner_status("closed", false, true), "closed");
        assert_eq!(owner_status("closed", false, false), "closed");
        assert_eq!(owner_status("open", false, false), "open");
    }
}
