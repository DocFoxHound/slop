// SPDX-License-Identifier: AGPL-3.0-or-later
//! Catalog: categories, tags, store listings and makers' app drafts (DESIGN §5, §8).
//!
//! Makers can create and edit drafts. Moving a draft into the store happens through Publishing
//! (repo link, build, scans), which isn't built yet.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    config::{CatalogRules, Config},
    error::{ApiError, ApiResult, is_unique_violation},
    policy::ViewerPolicy,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/categories", get(list_categories))
        .route("/apps", get(list_apps).post(create_app))
        .route("/apps/{slug}", get(get_app).patch(update_app))
        .route("/apps/{slug}/similar", get(similar_apps))
        .route(
            "/apps/{slug}/sandbox-report",
            put(report_sandbox).delete(unreport_sandbox),
        )
        .route("/me/apps", get(my_apps))
        .route("/repos/license", get(detect_license))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "app_kind", rename_all = "snake_case")]
pub enum AppKind {
    Game,
    App,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "app_stage", rename_all = "snake_case")]
pub enum AppStage {
    ComingSoon,
    Beta,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "app_status", rename_all = "snake_case")]
pub enum AppStatus {
    Draft,
    Listed,
    Delisted,
}

// ---------------------------------------------------------------------------------------------
// Categories

#[derive(Serialize, Deserialize, sqlx::FromRow)]
struct TagRef {
    slug: String,
    name: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct Category {
    slug: String,
    name: String,
    description: String,
    #[sqlx(json)]
    tags: Vec<TagRef>,
}

async fn list_categories(State(state): State<AppState>) -> ApiResult<Json<Vec<Category>>> {
    let categories = sqlx::query_as(
        "select c.slug, c.name, c.description,
                coalesce((select json_agg(json_build_object('slug', t.slug, 'name', t.name) order by t.name)
                          from tags t where t.category_id = c.id), '[]') as tags
         from categories c order by c.position",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(categories))
}

// ---------------------------------------------------------------------------------------------
// Store listing and search

/// Appends the conditions that put an app in the store for this viewer: listed, allowed by the
/// viewer's age policy, and not past its stage deadline (DESIGN §8: missed deadline = delisted).
pub(crate) fn push_store_visible(
    qb: &mut QueryBuilder<Postgres>,
    rules: CatalogRules,
    policy: ViewerPolicy,
) {
    push_store_visible_as(qb, rules, policy, "a");
}

/// `push_store_visible` for an `apps` row under another alias (static SQL).
pub(crate) fn push_store_visible_as(
    qb: &mut QueryBuilder<Postgres>,
    rules: CatalogRules,
    policy: ViewerPolicy,
    t: &'static str,
) {
    qb.push(format!("{t}.status = 'listed' and {t}.visibility = 'public' and {t}.min_age <= "))
        .push_bind(policy.max_min_age)
        .push(format!(
            " and not (({t}.stage = 'coming_soon' and {t}.stage_entered_at < now() - make_interval(days => "
        ))
        .push_bind(rules.coming_soon_days)
        .push(format!(
            ")) or ({t}.stage = 'beta' and {t}.stage_entered_at < now() - make_interval(days => "
        ))
        .push_bind(rules.beta_days)
        .push(")))");
}

/// Columns shared by every app summary query. `a` is `apps`.
pub(crate) fn push_summary_columns(qb: &mut QueryBuilder<Postgres>, config: &Config) {
    push_summary_columns_with(qb, config, "");
}

/// Like `push_summary_columns`, with `extra` columns (static SQL, e.g. `", l.added_at"`) added
/// to the select list.
pub(crate) fn push_summary_columns_with(
    qb: &mut QueryBuilder<Postgres>,
    config: &Config,
    extra: &'static str,
) {
    let rules = config.catalog;
    qb.push(
        "select a.slug, a.title, a.summary, a.kind, a.stage, a.status, a.min_age, a.updated_at, a.content_labels,
                (select p.players from pulse_counts p where p.app_id = a.id and a.kind = 'game'
                   and p.bucket > now() - interval '15 minutes' order by p.bucket desc limit 1) as players_now,
                coalesce(array(select t.slug from app_tags at join tags t on t.id = at.tag_id
                               where at.app_id = a.id order by t.slug), '{}') as tags,
                (select ms.thumbnail->>'url' from media_sets ms where ms.id = a.media_set_id) as thumbnail_url,
                (select count(*) from votes v where v.app_id = a.id) as vote_count,
                (select case when count(*) >= 5 and sum(v.weight) > 0
                             then round(100 * coalesce(sum(v.weight) filter (where v.vote = 1), 0) / sum(v.weight))::int4 end
                 from votes v where v.app_id = a.id) as not_slop_percent,
                exists (select 1 from jobs j where j.app_id = a.id and j.credited_at is not null) as made_with_people,
                (select count(*) from interests i where i.app_id = a.id) as interested,
                (select p.slug from apps p where p.id = a.parent_app_id) as branch_of,
                (a.stage <> 'released' and a.last_commit_at is not null
                 and a.last_commit_at < now() - make_interval(days => ",
    )
    .push_bind(rules.dead_after_days)
    .push(
        ")) as is_dead,
         case a.stage
             when 'coming_soon' then a.stage_entered_at + make_interval(days => ",
    )
    .push_bind(rules.coming_soon_days)
    .push(") when 'beta' then a.stage_entered_at + make_interval(days => ")
    .push_bind(rules.beta_days)
    .push(") end as stage_deadline")
    .push(extra)
    .push(" from apps a");
}

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct AppSummary {
    pub slug: String,
    pub title: String,
    summary: String,
    pub kind: AppKind,
    pub stage: AppStage,
    #[serde(skip_serializing_if = "is_listed")]
    status: AppStatus,
    pub min_age: i16,
    /// Themes players may want to know about (DESIGN §5): romance, suggestive, violence…
    pub content_labels: Vec<String>,
    /// Games: players in Slop Pulse's latest count, if it's recent.
    pub players_now: Option<i32>,
    pub tags: Vec<String>,
    /// The maker's store tile, if they set one (media.rs).
    thumbnail_url: Option<String>,
    /// Current 👍/👎 votes (votes.rs).
    pub vote_count: i64,
    /// Weighted % Not Slop; null below votes::MIN_VOTES.
    pub not_slop_percent: Option<i32>,
    /// Has a completed, fully paid People job credited (DESIGN §15.4): the "Made with people" badge.
    pub made_with_people: bool,
    /// People who marked it Interested (DESIGN §5): the demand signal for upcoming apps.
    pub interested: i64,
    /// A community branch of this app (DESIGN §9).
    pub branch_of: Option<String>,
    /// No commits for a while in Coming soon or Beta. Shown, but never recommended.
    pub is_dead: bool,
    /// When a Coming soon or Beta app must reach its next stage. `None` once released.
    #[serde(with = "time::serde::rfc3339::option")]
    stage_deadline: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
}

fn is_listed(status: &AppStatus) -> bool {
    *status == AppStatus::Listed
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Sort {
    Recent,
    Title,
    Relevance,
    /// Most players at once over the last 7 days (Slop Pulse), then recently updated.
    Trending,
    /// Most people Interested ("Most anticipated", DESIGN §5).
    Anticipated,
    /// Best rated (weighted % Not Slop, with enough votes), then most votes.
    Rating,
    /// Most added to libraries this calendar month ("Popular this month", DESIGN §5), then most
    /// players this month.
    Popular,
    /// Best weighted % Not Slop among votes cast or changed in the last 30 days ("Most Not Slop
    /// right now").
    NotSlopNow,
    /// Most weighted 👎 Slop votes in the last 30 days ("Fresh slop", for fun).
    FreshSlop,
    /// Most written reviews showing.
    Reviewed,
    /// Released in the last 60 days and in few libraries so far, in an order that changes every
    /// day, so each new app gets seen ("New and barely seen", DESIGN §5).
    BarelySeen,
}

#[derive(Deserialize)]
struct ListQuery {
    q: Option<String>,
    tag: Option<String>,
    category: Option<String>,
    stage: Option<AppStage>,
    kind: Option<AppKind>,
    /// Only apps with the "Made with people" badge.
    #[serde(default)]
    made_with_people: bool,
    /// Community branches instead of apps (DESIGN §9): the store lists branches only here.
    #[serde(default)]
    community: bool,
    /// Runs on this platform ("windows" or "linux"), directly or through a community build
    /// that adds it (DESIGN §5).
    platform: Option<String>,
    /// Only Coming soon and Beta (`true`) or only released (`false`).
    upcoming: Option<bool>,
    /// At least this weighted % Not Slop (apps without enough votes don't match).
    min_not_slop: Option<i32>,
    /// Has every one of these accessibility features (comma-separated, label.rs `FEATURES`).
    accessibility: Option<String>,
    sort: Option<Sort>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Serialize)]
struct AppPage {
    apps: Vec<AppSummary>,
    next_offset: Option<i64>,
}

const MAX_PAGE: i64 = 50;

/// "Barely seen": in fewer libraries than this.
const BARELY_SEEN_LIBRARIES: i64 = 25;

/// Most players at once over the last 7 days (Slop Pulse). `a` is `apps`.
pub(crate) const PLAYERS_THIS_WEEK: &str = "(select coalesce(max(p.players), 0) from pulse_counts p
     where p.app_id = a.id and p.bucket > now() - interval '7 days')";

async fn list_apps(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<AppPage>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let rules = state.config.catalog;
    let limit = query.limit.unwrap_or(24).clamp(1, MAX_PAGE);
    let offset = query.offset.unwrap_or(0).clamp(0, 10_000);
    let search = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    if search.is_some_and(|q| q.chars().count() > 100) {
        return Err(ApiError::invalid("q", "search is too long"));
    }

    let mut qb = QueryBuilder::new("");
    push_summary_columns(&mut qb, &state.config);
    qb.push(" where ");
    push_store_visible(&mut qb, rules, policy);
    qb.push(if query.community {
        // Community builds you can run; mods show on their game's page.
        " and a.parent_app_id is not null and a.branch_kind <> 'addon'"
    } else {
        " and a.parent_app_id is null"
    });
    if let Some(q) = search {
        qb.push(" and a.search @@ websearch_to_tsquery('english', ")
            .push_bind(q)
            .push(")");
    }
    if let Some(tag) = &query.tag {
        qb.push(" and exists (select 1 from app_tags at join tags t on t.id = at.tag_id where at.app_id = a.id and t.slug = ")
            .push_bind(tag)
            .push(")");
    }
    if let Some(category) = &query.category {
        qb.push(
            " and exists (select 1 from app_tags at join tags t on t.id = at.tag_id
                          join categories c on c.id = t.category_id where at.app_id = a.id and c.slug = ",
        )
        .push_bind(category)
        .push(")");
    }
    if let Some(stage) = query.stage {
        qb.push(" and a.stage = ").push_bind(stage);
    }
    if let Some(kind) = query.kind {
        qb.push(" and a.kind = ").push_bind(kind);
    }
    if let Some(platform) = query.platform.as_deref() {
        if !["windows", "linux"].contains(&platform) {
            return Err(ApiError::invalid("platform", "use windows or linux"));
        }
        // The app itself, or one of its community builds this viewer could see in the store.
        qb.push(" and (")
            .push_bind(platform)
            .push(" = any(a.platforms) or exists (select 1 from apps b where b.parent_app_id = a.id and ")
            .push_bind(platform)
            .push(" = any(b.platforms) and ");
        push_store_visible_as(&mut qb, rules, policy, "b");
        qb.push("))");
    }
    match query.upcoming {
        Some(true) => {
            qb.push(" and a.stage <> 'released'");
        }
        Some(false) => {
            qb.push(" and a.stage = 'released'");
        }
        None => {}
    }
    if let Some(min) = query.min_not_slop {
        qb.push(
            " and (select count(*) >= 5 and sum(v.weight) > 0
                          and 100 * coalesce(sum(v.weight) filter (where v.vote = 1), 0) / nullif(sum(v.weight), 0) >= ",
        )
        .push_bind(f64::from(min.clamp(0, 100)))
        .push(" from votes v where v.app_id = a.id)");
    }
    if let Some(list) = query.accessibility.as_deref().filter(|l| !l.is_empty()) {
        let wanted: Vec<String> = list.split(',').map(|f| f.trim().to_string()).collect();
        qb.push(" and a.accessibility @> ")
            .push_bind(crate::label::validate_accessibility(&wanted).map_err(|_| {
                ApiError::invalid("accessibility", "unknown accessibility feature")
            })?);
    }
    if query.made_with_people {
        qb.push(" and exists (select 1 from jobs j where j.app_id = a.id and j.credited_at is not null)");
    }
    match (
        query.sort.unwrap_or(if search.is_some() {
            Sort::Relevance
        } else {
            Sort::Recent
        }),
        search,
    ) {
        (Sort::Relevance, Some(q)) => {
            qb.push(" order by ts_rank(a.search, websearch_to_tsquery('english', ")
                .push_bind(q)
                .push(")) desc, a.updated_at desc");
        }
        (Sort::Title, _) => {
            qb.push(" order by lower(a.title), a.slug");
        }
        (Sort::Rating, _) => {
            qb.push(
                " order by (select case when count(*) >= 5 then sum(v.weight) filter (where v.vote = 1) / nullif(sum(v.weight), 0) end
                            from votes v where v.app_id = a.id) desc nulls last,
                           (select count(*) from votes v where v.app_id = a.id) desc, a.slug",
            );
        }
        (Sort::Anticipated, _) => {
            qb.push(
                " order by (select count(*) from interests i where i.app_id = a.id) desc, a.updated_at desc, a.slug",
            );
        }
        (Sort::Popular, _) => {
            qb.push(
                " order by (select count(*) from library_entries l
                            where l.app_id = a.id and l.added_at >= date_trunc('month', now())) desc,
                           (select coalesce(max(p.players), 0) from pulse_counts p
                            where p.app_id = a.id and p.bucket >= date_trunc('month', now())) desc, a.updated_at desc, a.slug",
            );
        }
        (Sort::NotSlopNow, _) => {
            qb.push(" and (select count(*) from votes v where v.app_id = a.id and v.updated_at > now() - interval '30 days') >= ")
                .push_bind(crate::votes::MIN_VOTES)
                .push(
                    " order by (select sum(v.weight) filter (where v.vote = 1) / nullif(sum(v.weight), 0)
                                from votes v where v.app_id = a.id and v.updated_at > now() - interval '30 days') desc nulls last,
                               (select count(*) from votes v where v.app_id = a.id) desc, a.slug",
                );
        }
        (Sort::FreshSlop, _) => {
            qb.push(
                " and exists (select 1 from votes v where v.app_id = a.id and v.vote = -1 and v.updated_at > now() - interval '30 days')
                  order by (select sum(v.weight) from votes v
                            where v.app_id = a.id and v.vote = -1 and v.updated_at > now() - interval '30 days') desc, a.slug",
            );
        }
        (Sort::Reviewed, _) => {
            qb.push(
                " order by (select count(*) from app_reviews r where r.app_id = a.id and r.hidden_at is null) desc,
                           a.updated_at desc, a.slug",
            );
        }
        (Sort::BarelySeen, _) => {
            qb.push(
                " and a.stage = 'released' and a.stage_entered_at > now() - interval '60 days'
                  and (select count(*) from library_entries l where l.app_id = a.id) < ",
            )
            .push_bind(BARELY_SEEN_LIBRARIES)
            .push(" order by md5(a.id::text || current_date::text), a.slug");
        }
        (Sort::Trending, _) => {
            qb.push(" order by ")
                .push(PLAYERS_THIS_WEEK)
                .push(" desc, a.updated_at desc, a.slug");
        }
        _ => {
            qb.push(" order by a.updated_at desc, a.slug");
        }
    }
    // Fetch one extra row to know whether there's another page.
    qb.push(" limit ")
        .push_bind(limit + 1)
        .push(" offset ")
        .push_bind(offset);

    let mut apps: Vec<AppSummary> = qb.build_query_as().fetch_all(&state.db).await?;
    let next_offset = (apps.len() as i64 > limit).then_some(offset + limit);
    apps.truncate(limit as usize);
    Ok(Json(AppPage { apps, next_offset }))
}

// ---------------------------------------------------------------------------------------------
// Similar apps, for an app's page

#[derive(Serialize)]
struct Similar {
    /// Shares the most tags, nudged by how much it's played.
    like_this: Vec<AppSummary>,
    /// The same kind (game or app), most played this week.
    popular: Vec<AppSummary>,
    /// The same kind and few players yet: a different handful each day.
    discover: Vec<AppSummary>,
}

const SIMILAR_EACH: i64 = 6;

/// Three shelves for an app's page. Each app appears once, never the app itself, and never a
/// dead project (DESIGN §8: dead projects are never recommended).
async fn similar_apps(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Similar>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, _) = visible_app(&state, &slug, user.as_ref(), policy).await?;
    let kind: AppKind = sqlx::query_scalar("select kind from apps where id = $1")
        .bind(app_id)
        .fetch_one(&state.db)
        .await?;
    let rules = state.config.catalog;

    // Everything shown so far, so the shelves don't repeat each other.
    let mut seen: Vec<String> = vec![slug.clone()];
    let base = |qb: &mut QueryBuilder<Postgres>, seen: &[String]| {
        push_summary_columns(qb, &state.config);
        qb.push(" where a.parent_app_id is null and ");
        push_store_visible(qb, rules, policy);
        qb.push(" and not (a.stage <> 'released' and a.last_commit_at is not null and a.last_commit_at < now() - make_interval(days => ")
            .push_bind(rules.dead_after_days)
            .push(")) and a.slug <> all(")
            .push_bind(seen.to_vec())
            .push(")");
    };

    // Tag overlap as a cosine (sharing 3 of 3 tags beats 3 of 12), a little for being the same
    // kind, and a little for being played, so a popular match wins a tie.
    let mut qb = QueryBuilder::new("");
    base(&mut qb, &seen);
    qb.push(" and exists (select 1 from app_tags x join app_tags y on y.tag_id = x.tag_id where x.app_id = a.id and y.app_id = ")
        .push_bind(app_id)
        .push(") order by (select count(*) from app_tags x join app_tags y on y.tag_id = x.tag_id where x.app_id = a.id and y.app_id = ")
        .push_bind(app_id)
        .push(")::float8 / sqrt(greatest(1, (select count(*) from app_tags x where x.app_id = a.id)) * greatest(1, (select count(*) from app_tags y where y.app_id = ")
        .push_bind(app_id)
        .push("))) + case when a.kind = ")
        .push_bind(kind)
        .push(" then 0.25 else 0 end + ln(1 + ")
        .push(PLAYERS_THIS_WEEK)
        .push(") * 0.05 desc, a.updated_at desc limit ")
        .push_bind(SIMILAR_EACH);
    let like_this: Vec<AppSummary> = qb.build_query_as().fetch_all(&state.db).await?;
    seen.extend(like_this.iter().map(|a| a.slug.clone()));

    let mut qb = QueryBuilder::new("");
    base(&mut qb, &seen);
    qb.push(" and a.stage <> 'coming_soon' and a.kind = ")
        .push_bind(kind)
        .push(" and ")
        .push(PLAYERS_THIS_WEEK)
        .push(" > 0 order by ")
        .push(PLAYERS_THIS_WEEK)
        .push(" desc, a.updated_at desc limit ")
        .push_bind(SIMILAR_EACH);
    let popular: Vec<AppSummary> = qb.build_query_as().fetch_all(&state.db).await?;
    seen.extend(popular.iter().map(|a| a.slug.clone()));

    // Least played first, shuffled per day and per page so every app gets its turn.
    let mut qb = QueryBuilder::new("");
    base(&mut qb, &seen);
    qb.push(" and a.kind = ")
        .push_bind(kind)
        .push(" order by least(")
        .push(PLAYERS_THIS_WEEK)
        .push(", 10), md5(a.id::text || current_date::text || ")
        .push_bind(&slug)
        .push(") limit ")
        .push_bind(SIMILAR_EACH);
    let discover: Vec<AppSummary> = qb.build_query_as().fetch_all(&state.db).await?;

    Ok(Json(Similar {
        like_this,
        popular,
        discover,
    }))
}

// ---------------------------------------------------------------------------------------------
// Store page

#[derive(Serialize)]
struct Maker {
    display_name: String,
}

#[derive(Serialize)]
struct AppDetail {
    #[serde(flatten)]
    summary: AppSummary,
    description_md: String,
    repo_url: String,
    license_spdx: String,
    /// The builds every release must include: windows, linux, linux-deb, linux-rpm.
    platforms: Vec<String>,
    maker: Maker,
    #[serde(with = "time::serde::rfc3339")]
    stage_entered_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    last_commit_at: Option<OffsetDateTime>,
    tag_details: Vec<TagRef>,
    /// Screenshots and videos, in the maker's order.
    gallery: Vec<crate::media::GalleryItem>,
    /// Creatives credited through People jobs.
    credits: Vec<crate::people::Credit>,
    is_mine: bool,
    /// The viewer marked it Interested.
    is_interested: bool,
    /// For a community branch: the app it branches from, and what it changes (DESIGN §9).
    branch: Option<crate::branches::BranchInfo>,
    /// For an app: its community builds in the store.
    community: Vec<crate::branches::CommunityBuild>,
    /// Private Slop (DESIGN §13): only its owner and the people it's shared with see it.
    private: bool,
    /// The verified official team that publishes it (DESIGN §14): (slug, name).
    official_team: Option<OfficialTeam>,
    /// What it connects to online (servers Slop can't check): own_servers, player_servers,
    /// peer_to_peer, other_services. Empty: it doesn't play online.
    online: Vec<String>,
    /// The maker's servers, by name (`play.example.com`, `api.example.com:7777`).
    online_hosts: Vec<String>,
    /// Sandbox mode: the maker's declaration, whether it's suggested, and player reports.
    #[serde(flatten)]
    sandbox: SandboxInfo,
    /// The viewer reported that it doesn't work in sandbox mode.
    sandbox_reported: bool,
    /// The folder it reads mods from, when its maker named one (DESIGN §9).
    mods_folder: Option<serde_json::Value>,
    /// Accessibility features the maker says it has (label.rs `FEATURES`).
    accessibility: Vec<String>,
    /// Takes bug reports from players (bug_reports.rs).
    bug_reports: bool,
    /// A browser demo running now (demos.rs): where to play it, and until when.
    demo: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct OfficialTeam {
    slug: String,
    name: String,
}

#[derive(sqlx::FromRow)]
struct AppExtra {
    id: Uuid,
    maker_id: Uuid,
    description_md: String,
    repo_url: String,
    license_spdx: String,
    platforms: Vec<String>,
    maker_name: String,
    stage_entered_at: OffsetDateTime,
    last_commit_at: Option<OffsetDateTime>,
    store_visible: bool,
    private: bool,
    online: Vec<String>,
    online_hosts: Vec<String>,
    mods_folder: Option<serde_json::Value>,
    accessibility: Vec<String>,
    bug_reports: bool,
}

/// Finds an app the viewer may see: one in the store for them, or their own. Returns its ID and
/// whether it's the viewer's. Anything else is "not found", so private apps can't be discovered.
pub async fn visible_app(
    state: &AppState,
    slug: &str,
    user: Option<&CurrentUser>,
    policy: ViewerPolicy,
) -> ApiResult<(Uuid, bool)> {
    let mut qb = QueryBuilder::new("select a.id, a.maker_id, (");
    push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(") or ");
    push_shared_with(&mut qb, user.map(|u| u.id));
    qb.push(" from apps a where a.slug = ").push_bind(slug);
    let row: Option<(Uuid, Uuid, bool)> = qb.build_query_as().fetch_optional(&state.db).await?;
    let (id, maker_id, store_visible) = row.ok_or(ApiError::NotFound)?;
    let is_mine = user.is_some_and(|u| u.id == maker_id);
    if !store_visible && !is_mine {
        return Err(ApiError::NotFound);
    }
    Ok((id, is_mine))
}

/// SQL condition: app `a` is Private Slop shared with the viewer, and has an approved release
/// (DESIGN §13: same requirements as public apps). Checked on every request (SECURITY §12).
pub(crate) fn push_shared_with(qb: &mut QueryBuilder<Postgres>, viewer: Option<Uuid>) {
    qb.push(
        "(a.visibility = 'private'
          and exists (select 1 from releases r where r.app_id = a.id and r.status = 'approved')
          and (a.frozen_at is null or a.frozen_by is not null or a.frozen_at > now() - interval '14 days')
          and (exists (select 1 from app_shares s where s.app_id = a.id and s.user_id = ",
    )
    .push_bind(viewer)
    .push(
        ")
               or exists (select 1 from app_team_shares ts join teams t on t.id = ts.team_id
                          join team_members m on m.team_id = t.id
                          where ts.app_id = a.id and t.kind = 'player' and m.user_id = ",
    )
    .push_bind(viewer)
    .push(")))");
}

/// Private Slop has no store page features: no voting, reviews, donations, Interested or branches.
pub async fn is_private<'c>(db: impl sqlx::PgExecutor<'c>, app_id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar("select visibility = 'private' from apps where id = $1")
        .bind(app_id)
        .fetch_one(db)
        .await
}

/// Loads an app's store page. Makers always see their own apps; everyone else only sees apps
/// that are in the store for them.
async fn load_detail(
    state: &AppState,
    slug: &str,
    user: Option<&CurrentUser>,
    policy: ViewerPolicy,
) -> ApiResult<AppDetail> {
    let rules = state.config.catalog;
    let mut qb = QueryBuilder::new(
        "select a.id, a.maker_id, a.description_md, a.repo_url, a.license_spdx, a.platforms, u.display_name as maker_name,
                a.stage_entered_at, a.last_commit_at, a.online, a.online_hosts, a.mods_folder, a.accessibility, a.bug_reports, (",
    );
    push_store_visible(&mut qb, rules, policy);
    qb.push(") or ");
    push_shared_with(&mut qb, user.map(|u| u.id));
    qb.push(" as store_visible, a.visibility = 'private' as private from apps a join users u on u.id = a.maker_id where a.slug = ")
        .push_bind(slug);
    let extra: AppExtra = qb
        .build_query_as()
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;

    let is_mine = user.is_some_and(|u| u.id == extra.maker_id);
    if !extra.store_visible && !is_mine {
        return Err(ApiError::NotFound);
    }

    let mut qb = QueryBuilder::new("");
    push_summary_columns(&mut qb, &state.config);
    qb.push(" where a.id = ").push_bind(extra.id);
    let summary: AppSummary = qb.build_query_as().fetch_one(&state.db).await?;
    let tag_details = sqlx::query_as(
        "select t.slug, t.name from app_tags at join tags t on t.id = at.tag_id where at.app_id = $1 order by t.slug",
    )
    .bind(extra.id)
    .fetch_all(&state.db)
    .await?;

    let community = crate::branches::community(state, extra.id, &extra.platforms, policy).await?;
    Ok(AppDetail {
        summary,
        description_md: extra.description_md,
        repo_url: extra.repo_url,
        license_spdx: extra.license_spdx,
        platforms: extra.platforms,
        maker: Maker {
            display_name: extra.maker_name,
        },
        stage_entered_at: extra.stage_entered_at,
        last_commit_at: extra.last_commit_at,
        tag_details,
        gallery: crate::media::gallery(state, extra.id).await?,
        credits: crate::people::credits(&state.db, extra.id).await?,
        is_mine,
        private: extra.private,
        online: extra.online,
        online_hosts: extra.online_hosts,
        mods_folder: extra.mods_folder,
        accessibility: extra.accessibility,
        bug_reports: extra.bug_reports,
        demo: match crate::demos::play_link(state, slug) {
            Some(url) => sqlx::query_scalar::<_, OffsetDateTime>(
                "select ends_at from demos where app_id = $1 and status = 'live' and ends_at > now()",
            )
            .bind(extra.id)
            .fetch_optional(&state.db)
            .await?
            .map(|ends| {
                serde_json::json!({ "url": url, "ends_at": ends.format(&time::format_description::well_known::Rfc3339).ok() })
            }),
            None => None,
        },
        sandbox: sandbox_info(&state.db, extra.id).await?,
        sandbox_reported: match user {
            Some(u) => sqlx::query_scalar(
                "select exists (select 1 from sandbox_reports where app_id = $1 and user_id = $2)",
            )
            .bind(extra.id)
            .bind(u.id)
            .fetch_one(&state.db)
            .await?,
            None => false,
        },
        official_team: crate::teams::official_team_of(&state.db, extra.id)
            .await?
            .map(|(slug, name)| OfficialTeam { slug, name }),
        branch: crate::branches::info(&state.db, extra.id).await?,
        community,
        is_interested: match user {
            Some(u) => {
                sqlx::query_scalar(
                    "select exists (select 1 from interests where user_id = $1 and app_id = $2)",
                )
                .bind(u.id)
                .bind(extra.id)
                .fetch_one(&state.db)
                .await?
            }
            None => false,
        },
    })
}

// ---------------------------------------------------------------------------------------------
// Sandbox mode (DESIGN §10): what players are told about running an app with no access to the
// rest of their computer.

/// Reports from this many players mark an app as having trouble in sandbox mode.
pub(crate) const SANDBOX_TROUBLE_REPORTS: i64 = 3;

#[derive(Serialize, sqlx::FromRow, Debug, Default)]
pub(crate) struct SandboxInfo {
    /// The maker says it keeps local saves only in SLOP_SAVE_DIR and works in sandbox mode.
    pub sandbox_ready: bool,
    /// ...but the source check of its newest release found saves elsewhere, so players aren't
    /// told it works in sandbox mode.
    pub sandbox_mismatch: bool,
    /// Sandbox mode is suggested: it plays online (servers Slop can't check), its newest
    /// release has networking code its store page doesn't declare, or it's a mod that goes
    /// online.
    pub sandbox_suggested: bool,
    /// Enough players reported it doesn't work in sandbox mode.
    pub sandbox_trouble: bool,
}

pub(crate) async fn sandbox_info(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<SandboxInfo> {
    sqlx::query_as(
        "with newest as (
             select r.repo_scan from releases r join apps a on a.id = r.app_id
             where r.app_id = $1 and r.status = 'approved' and r.signed_manifest is not null
             order by coalesce(r.id = a.pinned_release_id, false) desc, r.reviewed_at desc
             limit 1)
         select a.sandbox_ready,
                a.sandbox_ready and coalesce((select repo_scan ->> 'save_folder' = 'elsewhere' from newest), false)
                    as sandbox_mismatch,
                cardinality(a.online) > 0
                    or coalesce((select (repo_scan ->> 'online_undeclared')::boolean from newest), false)
                    or coalesce((select (repo_scan ->> 'mod_online')::boolean from newest), false)
                    as sandbox_suggested,
                (select count(*) from sandbox_reports s where s.app_id = a.id) >= $2 as sandbox_trouble
         from apps a where a.id = $1",
    )
    .bind(app_id)
    .bind(SANDBOX_TROUBLE_REPORTS)
    .fetch_one(db)
    .await
}

/// A player reports that the app doesn't work in sandbox mode. Taking it back is a DELETE.
async fn report_sandbox(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (app_id, _) = visible_app(&state, &slug, Some(&user), policy).await?;
    sqlx::query(
        "insert into sandbox_reports (app_id, user_id) values ($1, $2) on conflict do nothing",
    )
    .bind(app_id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unreport_sandbox(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (app_id, _) = visible_app(&state, &slug, Some(&user), policy).await?;
    sqlx::query("delete from sandbox_reports where app_id = $1 and user_id = $2")
        .bind(app_id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_app(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<AppDetail>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    Ok(Json(
        load_detail(&state, &slug, user.as_ref(), policy).await?,
    ))
}

async fn my_apps(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<AppSummary>>> {
    let mut qb = QueryBuilder::new("");
    push_summary_columns(&mut qb, &state.config);
    qb.push(" where a.maker_id = ")
        .push_bind(user.id)
        .push(" order by a.updated_at desc");
    Ok(Json(qb.build_query_as().fetch_all(&state.db).await?))
}

// ---------------------------------------------------------------------------------------------
// Licence detection, for the Publish form

#[derive(Deserialize)]
struct LicenseQuery {
    url: String,
}

#[derive(Serialize)]
struct DetectedLicense {
    /// SPDX ID, when the repository has a licence GitHub recognises.
    spdx: Option<String>,
    /// The licence's full name, or what GitHub found if it couldn't identify it.
    name: Option<String>,
    /// Why nothing was detected.
    note: Option<String>,
}

/// Asks GitHub which licence a repository has (its LICENSE file). Signed-in makers only, since
/// it uses the server's GitHub API allowance (and token, for private repositories).
async fn detect_license(
    State(state): State<AppState>,
    _user: CurrentUser,
    Query(q): Query<LicenseQuery>,
) -> ApiResult<Json<DetectedLicense>> {
    let none = |note: &str| {
        Json(DetectedLicense {
            spdx: None,
            name: None,
            note: Some(note.to_string()),
        })
    };
    let Ok(repo_url) = validate_repo_url(&q.url) else {
        return Ok(none("enter the repository's https:// address first"));
    };
    let Some(repo) = slop_provenance::github_repo_path(&repo_url) else {
        return Ok(none(
            "licences can only be detected for GitHub repositories so far",
        ));
    };

    #[derive(Deserialize)]
    struct GithubLicense {
        license: Option<LicenseInfo>,
    }
    #[derive(Deserialize)]
    struct LicenseInfo {
        spdx_id: Option<String>,
        name: Option<String>,
    }
    let url = state
        .config
        .releases
        .github_api_base
        .join(&format!("repos/{repo}/license"))
        .map_err(|e| ApiError::Internal(e.into()))?;
    let res = crate::releases::github_get(&state, url)
        .await
        .send()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    match res.status().as_u16() {
        200 => {}
        404 => {
            return Ok(none(
                "no licence file found (or the repository isn't visible to Slop)",
            ));
        }
        code => {
            return Ok(none(&format!(
                "GitHub answered {code}; pick the licence yourself"
            )));
        }
    }
    let found: GithubLicense = res.json().await.map_err(|e| ApiError::Internal(e.into()))?;
    let info = found.license;
    // GitHub says NOASSERTION when there's a licence file it can't identify.
    let spdx = info
        .as_ref()
        .and_then(|l| l.spdx_id.clone())
        .filter(|id| id != "NOASSERTION" && validate_license(id).is_ok());
    let name = info.and_then(|l| l.name);
    let note = spdx.is_none().then(|| {
        "there's a licence file, but GitHub couldn't tell which licence it is".to_string()
    });
    Ok(Json(DetectedLicense { spdx, name, note }))
}

// ---------------------------------------------------------------------------------------------
// Makers: create and edit drafts

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewApp {
    slug: String,
    title: String,
    summary: String,
    #[serde(default)]
    description_md: String,
    kind: AppKind,
    stage: AppStage,
    repo_url: String,
    license_spdx: String,
    min_age: i16,
    #[serde(default)]
    content_labels: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    platforms: Vec<String>,
    /// "public" (the default) or "private" (Private Slop, DESIGN §13).
    #[serde(default)]
    visibility: Option<String>,
    /// A private repository outside GitHub: a read-only token for it (repo_tokens.rs).
    #[serde(default)]
    repo_token: Option<crate::repo_tokens::Given>,
    #[serde(default)]
    online: Vec<String>,
    #[serde(default)]
    online_hosts: Vec<String>,
    /// Works in sandbox mode: local saves only in SLOP_SAVE_DIR.
    #[serde(default)]
    sandbox_ready: bool,
    /// The folder it reads mods from (DESIGN §9); none: it reads SLOP_MODS, or has no mods.
    #[serde(default)]
    mods_folder: Option<slop_manifest::ModsFolder>,
    /// Accessibility features it has (label.rs `FEATURES`).
    #[serde(default)]
    accessibility: Vec<String>,
    /// Takes bug reports from players (bug_reports.rs); on unless turned off.
    #[serde(default = "yes")]
    bug_reports: bool,
}

fn yes() -> bool {
    true
}

/// Fields a maker can change. Slug, kind and stage aren't editable here: stage changes go
/// through Publishing, since each stage has requirements (DESIGN §8).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppChanges {
    title: Option<String>,
    summary: Option<String>,
    description_md: Option<String>,
    repo_url: Option<String>,
    license_spdx: Option<String>,
    min_age: Option<i16>,
    content_labels: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    platforms: Option<Vec<String>>,
    online: Option<Vec<String>>,
    online_hosts: Option<Vec<String>>,
    sandbox_ready: Option<bool>,
    /// `null` clears it.
    #[serde(default, deserialize_with = "present")]
    mods_folder: Option<Option<slop_manifest::ModsFolder>>,
    accessibility: Option<Vec<String>>,
    /// Takes bug reports from players (bug_reports.rs).
    bug_reports: Option<bool>,
}

/// A field that was sent, even as `null`: `Some(None)`.
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

fn validate_mods_folder(
    f: Option<slop_manifest::ModsFolder>,
) -> ApiResult<Option<serde_json::Value>> {
    match f {
        None => Ok(None),
        Some(f) if f.is_valid() => Ok(Some(serde_json::to_value(f).expect("serialises"))),
        Some(_) => Err(ApiError::invalid(
            "mods_folder",
            "a folder inside the game's files or its save folder, like mods or Content/Paks/~mods",
        )),
    }
}

/// Content labels makers can set (DESIGN §5); the database checks the same list.
pub const CONTENT_LABELS: [&str; 6] = [
    "romance",
    "suggestive",
    "violence",
    "horror",
    "strong_language",
    "gambling_themes",
];

pub(crate) fn validate_content_labels(labels: &[String]) -> ApiResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for l in labels {
        if !CONTENT_LABELS.contains(&l.as_str()) {
            return Err(ApiError::invalid(
                "content_labels",
                "use romance, suggestive, violence, horror, strong_language or gambling_themes",
            ));
        }
        if !out.contains(l) {
            out.push(l.clone());
        }
    }
    Ok(out)
}

/// What an app can connect to online; the database checks the same list.
pub const ONLINE: [&str; 4] = [
    "own_servers",
    "player_servers",
    "peer_to_peer",
    "other_services",
];

pub(crate) fn validate_online(kinds: &[String]) -> ApiResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for k in kinds {
        if !ONLINE.contains(&k.as_str()) {
            return Err(ApiError::invalid(
                "online",
                "use own_servers, player_servers, peer_to_peer or other_services",
            ));
        }
        if !out.contains(k) {
            out.push(k.clone());
        }
    }
    Ok(out)
}

/// Server names: `host` or `host:port`, lowercase, with a dot (`play.example.com`).
pub(crate) fn validate_online_hosts(hosts: &[String]) -> ApiResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for h in hosts {
        let h = h.trim().to_ascii_lowercase();
        if h.is_empty() {
            continue;
        }
        let (name, port) = h.split_once(':').unwrap_or((&h, ""));
        let ok = name.len() <= 253
            && name.contains('.')
            && !name.starts_with(['.', '-'])
            && !name.ends_with(['.', '-'])
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            && (port.is_empty() || port.parse::<u16>().is_ok_and(|p| p > 0));
        if !ok {
            return Err(ApiError::invalid(
                "online_hosts",
                format!("{h:?} isn't a server name like play.example.com or play.example.com:7777"),
            ));
        }
        if !out.contains(&h) {
            out.push(h);
        }
    }
    if out.len() > 10 {
        return Err(ApiError::invalid("online_hosts", "list up to 10 servers"));
    }
    Ok(out)
}

const MAX_TAGS: usize = 10;

pub(crate) fn validate_slug(slug: &str) -> ApiResult<()> {
    let ok = (3..=60).contains(&slug.len())
        && slug.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    if !ok {
        return Err(ApiError::invalid(
            "slug",
            "use 3 to 60 lowercase letters, numbers and single dashes",
        ));
    }
    Ok(())
}

pub(crate) fn validate_text(
    field: &'static str,
    value: &str,
    max: usize,
    allow_empty: bool,
) -> ApiResult<String> {
    let value = value.trim();
    let len = value.chars().count();
    if (!allow_empty && len == 0) || len > max {
        let min = if allow_empty { 0 } else { 1 };
        return Err(ApiError::invalid(
            field,
            format!("use {min} to {max} characters"),
        ));
    }
    if value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t' && c != '\r')
    {
        return Err(ApiError::invalid(
            field,
            "contains characters that aren't allowed",
        ));
    }
    Ok(value.to_string())
}

/// Public `https` repo URLs only (DESIGN §8: GitHub, GitLab, Bitbucket or self-hosted).
pub(crate) fn validate_repo_url(value: &str) -> ApiResult<String> {
    validate_repo_url_with(value, false)
}

/// As `validate_repo_url`; servers set up for local testing (plain-http downloads) also take
/// http, for test Git servers on the same machine.
pub(crate) fn validate_repo_url_in(state: &AppState, value: &str) -> ApiResult<String> {
    validate_repo_url_with(value, state.config.releases.allow_http_downloads)
}

fn validate_repo_url_with(value: &str, allow_http: bool) -> ApiResult<String> {
    let invalid = || ApiError::invalid("repo_url", "use the https:// address of the repository");
    let url = Url::parse(value.trim()).map_err(|_| invalid())?;
    if !(url.scheme() == "https" || (allow_http && url.scheme() == "http"))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().trim_matches('/').is_empty()
        || url.as_str().len() > 300
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// An SPDX licence expression's characters. Whether the licence is OSI-approved is checked by the
/// licence scan (LEGAL-GUIDANCE IP-2), which runs against the repo itself.
pub(crate) fn validate_license(value: &str) -> ApiResult<String> {
    let value = value.trim();
    let ok = (1..=64).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " .-+()".contains(c));
    if !ok {
        return Err(ApiError::invalid(
            "license_spdx",
            "use an SPDX licence ID, such as MIT or GPL-3.0-only",
        ));
    }
    Ok(value.to_string())
}

/// Windows and/or Linux; Linux comes as a .tar.gz plus at least one package format.
pub(crate) fn validate_platforms(value: &[String]) -> ApiResult<Vec<String>> {
    let invalid = |msg: &str| ApiError::invalid("platforms", msg.to_string());
    const ORDER: [&str; 4] = ["windows", "linux", "linux-deb", "linux-rpm"];
    if let Some(bad) = value.iter().find(|p| !ORDER.contains(&p.as_str())) {
        return Err(invalid(&format!("unknown platform: {bad}")));
    }
    let has = |p: &str| value.iter().any(|v| v == p);
    if !has("windows") && !has("linux") {
        return Err(invalid("choose Windows, Linux or both"));
    }
    if has("linux") && !has("linux-deb") && !has("linux-rpm") {
        return Err(invalid("Linux apps need a .deb or .rpm package, or both"));
    }
    if !has("linux") && (has("linux-deb") || has("linux-rpm")) {
        return Err(invalid("packages come with Linux"));
    }
    Ok(ORDER
        .iter()
        .filter(|p| has(p))
        .map(|p| p.to_string())
        .collect())
}

pub(crate) fn validate_min_age(value: i16) -> ApiResult<i16> {
    if ![0, 3, 7, 12, 16, 18].contains(&value) {
        return Err(ApiError::invalid("min_age", "use 0, 3, 7, 12, 16 or 18"));
    }
    Ok(value)
}

/// Resolves tag slugs to IDs, rejecting unknown tags and duplicates.
async fn resolve_tags(db: &sqlx::PgPool, tags: &[String]) -> ApiResult<Vec<i16>> {
    if tags.len() > MAX_TAGS {
        return Err(ApiError::invalid(
            "tags",
            format!("use at most {MAX_TAGS} tags"),
        ));
    }
    let mut unique = tags.to_vec();
    unique.sort();
    unique.dedup();
    if unique.len() != tags.len() {
        return Err(ApiError::invalid("tags", "a tag is listed twice"));
    }
    let found: Vec<(i16, String)> =
        sqlx::query_as("select id, slug from tags where slug = any($1)")
            .bind(&unique)
            .fetch_all(db)
            .await?;
    if let Some(missing) = unique.iter().find(|s| !found.iter().any(|(_, f)| f == *s)) {
        return Err(ApiError::invalid("tags", format!("unknown tag: {missing}")));
    }
    Ok(found.into_iter().map(|(id, _)| id).collect())
}

async fn set_tags(tx: &mut sqlx::PgConnection, app_id: Uuid, tag_ids: &[i16]) -> ApiResult<()> {
    sqlx::query("delete from app_tags where app_id = $1")
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("insert into app_tags (app_id, tag_id) select $1, unnest($2::smallint[])")
        .bind(app_id)
        .bind(tag_ids)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

async fn create_app(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Json(req): Json<NewApp>,
) -> ApiResult<(StatusCode, Json<AppDetail>)> {
    crate::limits::check(&state, user.id, crate::limits::Action::NewApp)?;
    validate_slug(&req.slug)?;
    let title = validate_text("title", &req.title, 80, false)?;
    let summary = validate_text("summary", &req.summary, 200, false)?;
    let description = validate_text("description_md", &req.description_md, 20_000, true)?;
    let repo_url = validate_repo_url_in(&state, &req.repo_url)?;
    let license = validate_license(&req.license_spdx)?;
    let min_age = validate_min_age(req.min_age)?;
    let content_labels = validate_content_labels(&req.content_labels)?;
    let platforms = validate_platforms(&req.platforms)?;
    let online = validate_online(&req.online)?;
    let online_hosts = validate_online_hosts(&req.online_hosts)?;
    let mods_folder = validate_mods_folder(req.mods_folder.clone())?;
    let tag_ids = resolve_tags(&state.db, &req.tags).await?;
    crate::legal::require(&state.db, user.id, "publisher").await?;
    crate::passkeys::require(&state, &user).await?;
    if let Some(t) = &req.repo_token {
        crate::repo_tokens::verify_given(&state, &repo_url, t).await?;
    }
    crate::repo_owner::check_with(&state, user.id, &repo_url, None, req.repo_token.as_ref())
        .await?;

    let visibility = match req.visibility.as_deref() {
        None | Some("public") => "public",
        Some("private") => "private",
        Some(_) => return Err(ApiError::invalid("visibility", "public or private")),
    };

    let app_id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into apps (id, slug, title, summary, description_md, kind, stage, maker_id, repo_url, license_spdx, min_age,
                           platforms, visibility, content_labels, online, online_hosts, sandbox_ready, mods_folder,
                           accessibility, bug_reports)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)",
    )
    .bind(app_id)
    .bind(&req.slug)
    .bind(&title)
    .bind(&summary)
    .bind(&description)
    .bind(req.kind)
    .bind(req.stage)
    .bind(user.id)
    .bind(&repo_url)
    .bind(&license)
    .bind(min_age)
    .bind(&platforms)
    .bind(visibility)
    .bind(&content_labels)
    .bind(&online)
    .bind(&online_hosts)
    .bind(req.sandbox_ready)
    .bind(&mods_folder)
    .bind(crate::label::validate_accessibility(&req.accessibility)?)
    .bind(req.bug_reports)
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        if is_unique_violation(&err) {
            ApiError::invalid("slug", "that address is taken")
        } else {
            err.into()
        }
    })?;
    set_tags(&mut tx, app_id, &tag_ids).await?;
    if let (Some(t), Some(key)) = (&req.repo_token, state.config.releases.repo_token_key) {
        crate::repo_tokens::store(&mut tx, &key, app_id, &repo_url, t, user.id).await?;
    }
    tx.commit().await?;

    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let detail = load_detail(&state, &req.slug, Some(&user), policy).await?;
    Ok((StatusCode::CREATED, Json(detail)))
}

async fn update_app(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(changes): Json<AppChanges>,
) -> ApiResult<Json<AppDetail>> {
    let mut tx = state.db.begin().await?;
    let owner: Option<(Uuid, Uuid, String)> =
        sqlx::query_as("select id, maker_id, repo_url from apps where slug = $1 for update")
            .bind(&slug)
            .fetch_optional(&mut *tx)
            .await?;
    // Someone else's app looks the same as a missing one, so drafts can't be discovered.
    let (app_id, current_repo) = match owner {
        Some((id, maker_id, repo)) if maker_id == user.id => (id, repo),
        _ => return Err(ApiError::NotFound),
    };

    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("update apps set updated_at = now()");
    if let Some(title) = &changes.title {
        qb.push(", title = ")
            .push_bind(validate_text("title", title, 80, false)?);
    }
    if let Some(summary) = &changes.summary {
        qb.push(", summary = ")
            .push_bind(validate_text("summary", summary, 200, false)?);
    }
    if let Some(description) = &changes.description_md {
        qb.push(", description_md = ").push_bind(validate_text(
            "description_md",
            description,
            20_000,
            true,
        )?);
    }
    if let Some(repo_url) = &changes.repo_url {
        let repo_url = validate_repo_url_in(&state, repo_url)?;
        if !crate::manage::same_repo(&repo_url, &current_repo) {
            crate::repo_owner::check(&state, user.id, &repo_url, Some(app_id)).await?;
        }
        // Once players have the app, moving it is checked first (manage.rs).
        if crate::manage::repo_locked(&mut tx, app_id).await?
            && !crate::manage::same_repo(&repo_url, &current_repo)
        {
            return Err(ApiError::Conflict(
                "players already have this app, so moving it to another repository is reviewed \
                 first: request the move from Manage"
                    .into(),
            ));
        }
        qb.push(", repo_url = ").push_bind(repo_url);
    }
    if let Some(license) = &changes.license_spdx {
        qb.push(", license_spdx = ")
            .push_bind(validate_license(license)?);
    }
    // Lowering the rating below what the last approved release says goes through a release,
    // which a person reviews (AGE-7); raising it is immediate.
    if changes.min_age.is_some() || changes.content_labels.is_some() {
        let released: Option<(i16, Vec<String>)> = sqlx::query_as(
            "select min_age, content_labels from releases
             where app_id = $1 and status = 'approved' and min_age is not null
             order by reviewed_at desc limit 1",
        )
        .bind(app_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((age, labels)) = released {
            let lower_age = changes.min_age.is_some_and(|a| a < age);
            let dropped = changes
                .content_labels
                .as_ref()
                .is_some_and(|new| labels.iter().any(|l| !new.contains(l)));
            if lower_age || dropped {
                return Err(ApiError::invalid(
                    if lower_age {
                        "min_age"
                    } else {
                        "content_labels"
                    },
                    "your latest release has this rating: to lower it, submit a release with the new rating, and a person reviews it",
                ));
            }
        }
    }
    // A branch is rated at least as the original, with its content labels (DESIGN §9).
    if (changes.min_age.is_some() || changes.content_labels.is_some())
        && let Some(floor) = crate::branches::rating_floor(&mut tx, app_id).await?
    {
        if let Some(min_age) = changes.min_age {
            floor.check_age(min_age)?;
        }
        if let Some(labels) = &changes.content_labels {
            floor.check_labels(labels)?;
        }
    }
    if let Some(min_age) = changes.min_age {
        qb.push(", min_age = ")
            .push_bind(validate_min_age(min_age)?);
    }
    if let Some(platforms) = &changes.platforms {
        let is_mod: bool = sqlx::query_scalar(
            "select branch_kind is not distinct from 'addon' from apps where id = $1",
        )
        .bind(app_id)
        .fetch_one(&mut *tx)
        .await?;
        if is_mod {
            return Err(ApiError::invalid(
                "platforms",
                "a mod is one .zip for every platform",
            ));
        }
        qb.push(", platforms = ")
            .push_bind(validate_platforms(platforms)?);
    }
    if let Some(online) = &changes.online {
        qb.push(", online = ").push_bind(validate_online(online)?);
    }
    if let Some(hosts) = &changes.online_hosts {
        qb.push(", online_hosts = ")
            .push_bind(validate_online_hosts(hosts)?);
    }
    if let Some(ready) = changes.sandbox_ready {
        qb.push(", sandbox_ready = ").push_bind(ready);
    }
    if let Some(on) = changes.bug_reports {
        qb.push(", bug_reports = ").push_bind(on);
    }
    if let Some(features) = &changes.accessibility {
        qb.push(", accessibility = ")
            .push_bind(crate::label::validate_accessibility(features)?);
    }
    if let Some(folder) = &changes.mods_folder {
        qb.push(", mods_folder = ")
            .push_bind(validate_mods_folder(folder.clone())?);
    }
    if let Some(labels) = &changes.content_labels {
        qb.push(", content_labels = ")
            .push_bind(validate_content_labels(labels)?);
    }
    qb.push(" where id = ").push_bind(app_id);
    qb.build().execute(&mut *tx).await?;

    if let Some(tags) = &changes.tags {
        let tag_ids = resolve_tags(&state.db, tags).await?;
        set_tags(&mut tx, app_id, &tag_ids).await?;
    }
    tx.commit().await?;

    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    Ok(Json(load_detail(&state, &slug, Some(&user), policy).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platforms() {
        let v =
            |p: &[&str]| validate_platforms(&p.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(
            v(&["linux-rpm", "windows", "linux"]).unwrap(),
            ["windows", "linux", "linux-rpm"]
        );
        assert!(v(&["windows"]).is_ok());
        assert!(v(&[]).is_err());
        assert!(v(&["linux"]).is_err(), "Linux needs a package");
        assert!(v(&["windows", "linux-deb"]).is_err(), "packages need Linux");
        assert!(v(&["mac"]).is_err());
    }

    #[test]
    fn online_hosts() {
        let ok = validate_online_hosts(&[
            " Play.Example.com ".into(),
            "api.example.com:7777".into(),
            "play.example.com".into(),
            "".into(),
        ])
        .unwrap();
        assert_eq!(ok, ["play.example.com", "api.example.com:7777"]);
        for bad in [
            "localhost",
            "http://x.com",
            "x.com/path",
            "x.com:0",
            "x.com:99999",
            "-x.com",
        ] {
            assert!(validate_online_hosts(&[bad.into()]).is_err(), "{bad}");
        }
        assert!(validate_online(&["own_servers".into(), "bogus".into()]).is_err());
    }

    #[test]
    fn slugs() {
        for ok in ["abc", "pixel-dungeon-2", "a1b"] {
            assert!(validate_slug(ok).is_ok(), "{ok}");
        }
        for bad in ["ab", "Abc", "a--b", "-abc", "abc-", "a_b", "ünï"] {
            assert!(validate_slug(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn repo_urls() {
        assert_eq!(
            validate_repo_url("https://github.com/a/b/").unwrap(),
            "https://github.com/a/b"
        );
        for bad in [
            "http://github.com/a/b",
            "https://github.com",
            "https://user:pw@github.com/a/b",
            "https://github.com/a/b?x=1",
            "git@github.com:a/b.git",
            "javascript:alert(1)",
        ] {
            assert!(validate_repo_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn licences() {
        assert!(validate_license("MIT OR Apache-2.0").is_ok());
        assert!(validate_license("GPL-3.0-or-later").is_ok());
        assert!(validate_license("<script>").is_err());
        assert!(validate_license("").is_err());
    }
}
