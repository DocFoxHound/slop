//! Catalog: categories, tags, store listings and makers' app drafts (DESIGN §5, §8).
//!
//! Makers can create and edit drafts. Moving a draft into the store happens through Publishing
//! (repo link, build, scans), which isn't built yet.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
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
    qb.push("a.status = 'listed' and a.min_age <= ")
        .push_bind(policy.max_min_age)
        .push(" and not ((a.stage = 'coming_soon' and a.stage_entered_at < now() - make_interval(days => ")
        .push_bind(rules.coming_soon_days)
        .push(")) or (a.stage = 'beta' and a.stage_entered_at < now() - make_interval(days => ")
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
        "select a.slug, a.title, a.summary, a.kind, a.stage, a.status, a.min_age, a.updated_at,
                coalesce(array(select t.slug from app_tags at join tags t on t.id = at.tag_id
                               where at.app_id = a.id order by t.slug), '{}') as tags,
                (select ms.thumbnail->>'url' from media_sets ms where ms.id = a.media_set_id) as thumbnail_url,
                (select count(*) from votes v where v.app_id = a.id) as vote_count,
                (select case when count(*) >= 5 and sum(v.weight) > 0
                             then round(100 * coalesce(sum(v.weight) filter (where v.vote = 1), 0) / sum(v.weight))::int4 end
                 from votes v where v.app_id = a.id) as not_slop_percent,
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
    pub tags: Vec<String>,
    /// The maker's store tile, if they set one (media.rs).
    thumbnail_url: Option<String>,
    /// Current 👍/👎 votes (votes.rs).
    pub vote_count: i64,
    /// Weighted % Not Slop; null below votes::MIN_VOTES.
    pub not_slop_percent: Option<i32>,
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
}

#[derive(Deserialize)]
struct ListQuery {
    q: Option<String>,
    tag: Option<String>,
    category: Option<String>,
    stage: Option<AppStage>,
    kind: Option<AppKind>,
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
        qb.push(" where ");
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
    is_mine: bool,
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
    qb.push(") from apps a where a.slug = ").push_bind(slug);
    let row: Option<(Uuid, Uuid, bool)> = qb.build_query_as().fetch_optional(&state.db).await?;
    let (id, maker_id, store_visible) = row.ok_or(ApiError::NotFound)?;
    let is_mine = user.is_some_and(|u| u.id == maker_id);
    if !store_visible && !is_mine {
        return Err(ApiError::NotFound);
    }
    Ok((id, is_mine))
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
                a.stage_entered_at, a.last_commit_at, (",
    );
    push_store_visible(&mut qb, rules, policy);
    qb.push(") as store_visible from apps a join users u on u.id = a.maker_id where a.slug = ")
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
        is_mine,
    })
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
    tags: Vec<String>,
    platforms: Vec<String>,
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
    tags: Option<Vec<String>>,
    platforms: Option<Vec<String>>,
}

const MAX_TAGS: usize = 10;

fn validate_slug(slug: &str) -> ApiResult<()> {
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

fn validate_text(
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
    let invalid = || ApiError::invalid("repo_url", "use the https:// address of the repository");
    let url = Url::parse(value.trim()).map_err(|_| invalid())?;
    if url.scheme() != "https"
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
fn validate_license(value: &str) -> ApiResult<String> {
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

fn validate_min_age(value: i16) -> ApiResult<i16> {
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
    validate_slug(&req.slug)?;
    let title = validate_text("title", &req.title, 80, false)?;
    let summary = validate_text("summary", &req.summary, 200, false)?;
    let description = validate_text("description_md", &req.description_md, 20_000, true)?;
    let repo_url = validate_repo_url(&req.repo_url)?;
    let license = validate_license(&req.license_spdx)?;
    let min_age = validate_min_age(req.min_age)?;
    let platforms = validate_platforms(&req.platforms)?;
    let tag_ids = resolve_tags(&state.db, &req.tags).await?;

    let app_id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into apps (id, slug, title, summary, description_md, kind, stage, maker_id, repo_url, license_spdx, min_age,
                           platforms)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
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
        let repo_url = validate_repo_url(repo_url)?;
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
    if let Some(min_age) = changes.min_age {
        qb.push(", min_age = ")
            .push_bind(validate_min_age(min_age)?);
    }
    if let Some(platforms) = &changes.platforms {
        qb.push(", platforms = ")
            .push_bind(validate_platforms(platforms)?);
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
