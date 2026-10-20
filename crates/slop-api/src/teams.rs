//! Teams (DESIGN §14).
//!
//! - **Player teams:** anyone creates one; others join (open teams) or are invited. Members share
//!   **picks** (apps they recommend) and **game nights**, and see Private Slop shared with the team.
//! - **Official teams:** developer studios, verified through their **GitHub organization**:
//!   1. the person setting it up is an **owner** of the organization;
//!   2. the organization has GitHub's **verified domain** badge, or a DNS TXT record on the
//!      studio's domain (`_slop-team.<domain>` = `slop-team=<token>`);
//!   3. **two-factor authentication** is required for every member of the organization;
//!   4. the team's apps are published **from repositories inside the organization** (only those
//!      are shown as the team's);
//!   5. if they take donations, Stripe has verified the studio's account as a **business**.
//!
//!   Slop reads the organization through its GitHub App installed there. Every check is re-run
//!   daily; while any fails, the badge is off. Official teams can never receive Private Slop.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog::{self, AppSummary},
    error::{ApiError, ApiResult, is_unique_violation},
    notify::{self, Notice},
    policy::ViewerPolicy,
    private::parse_user,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/teams", get(list).post(create))
        .route("/me/teams", get(mine))
        .route("/teams/{slug}", get(page).patch(update))
        .route("/teams/{slug}/join", post(join))
        .route("/teams/{slug}/leave", post(leave))
        .route("/teams/{slug}/invites", post(invite))
        .route("/teams/{slug}/invites/accept", post(accept_invite))
        .route("/teams/{slug}/invites/decline", post(decline_invite))
        .route(
            "/teams/{slug}/members/{user_id}",
            delete(remove_member).put(set_role),
        )
        .route("/teams/{slug}/picks", post(add_pick))
        .route("/teams/{slug}/picks/{id}", delete(remove_pick))
        .route("/teams/{slug}/events", post(add_event))
        .route("/teams/{slug}/events/{id}", delete(remove_event))
        .route("/teams/{slug}/verify", post(verify))
}

fn text(field: &'static str, value: &str, min: usize, max: usize) -> ApiResult<String> {
    let v = value.trim();
    let n = v.chars().count();
    if n < min || n > max {
        return Err(ApiError::invalid(
            field,
            format!("{min} to {max} characters"),
        ));
    }
    if v.chars().any(|c| c.is_control() && c != '\n') {
        return Err(ApiError::invalid(
            field,
            "contains characters that aren't allowed",
        ));
    }
    Ok(v.to_string())
}

#[derive(Serialize, sqlx::FromRow)]
pub struct TeamCard {
    slug: String,
    name: String,
    kind: String,
    about: String,
    join_policy: String,
    members: i64,
    verified: bool,
}

const CARD: &str = "select t.slug, t.name, t.kind, t.about, t.join_policy,
        (select count(*) from team_members m where m.team_id = t.id) as members,
        t.verified_at is not null as verified
     from teams t";

/// Teams anyone can find: open player teams and verified official teams.
async fn list(State(state): State<AppState>) -> ApiResult<Json<Vec<TeamCard>>> {
    Ok(Json(
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{CARD} where t.hidden_reason is null
               and ((t.kind = 'player' and t.join_policy = 'open') or (t.kind = 'official' and t.verified_at is not null))
             order by t.kind desc, (select count(*) from team_members m where m.team_id = t.id) desc, t.name limit 100"
        )))
        .fetch_all(&state.db)
        .await?,
    ))
}

#[derive(Serialize)]
struct Mine {
    teams: Vec<TeamCard>,
    /// Invitations waiting for an answer.
    invites: Vec<TeamCard>,
}

async fn mine(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Mine>> {
    Ok(Json(Mine {
        teams: sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{CARD} join team_members me on me.team_id = t.id and me.user_id = $1 order by t.name"
        )))
        .bind(user.id)
        .fetch_all(&state.db)
        .await?,
        invites: sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{CARD} join team_invites i on i.team_id = t.id and i.user_id = $1 where t.hidden_reason is null order by i.created_at"
        )))
        .bind(user.id)
        .fetch_all(&state.db)
        .await?,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    slug: String,
    name: String,
    kind: String,
    #[serde(default)]
    about: String,
    #[serde(default)]
    join_policy: Option<String>,
    /// Official teams: the GitHub organization.
    github_org: Option<String>,
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let slug = b.slug.trim().to_ascii_lowercase();
    if !(3..=40).contains(&slug.len())
        || !slug.split('-').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
    {
        return Err(ApiError::invalid(
            "slug",
            "3 to 40 lowercase letters, numbers and single dashes",
        ));
    }
    let name = text("name", &b.name, 1, 60)?;
    let about = text("about", &b.about, 0, 2000)?;
    let (kind, org, policy) = match b.kind.as_str() {
        "player" => {
            let policy = match b.join_policy.as_deref() {
                Some("open") => "open",
                None | Some("invite") => "invite",
                Some(_) => return Err(ApiError::invalid("join_policy", "open or invite")),
            };
            (b.kind.as_str(), None, policy)
        }
        "official" => {
            let org = b.github_org.as_deref().map(str::trim).unwrap_or("");
            if org.is_empty()
                || org.len() > 39
                || !org.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                return Err(ApiError::invalid(
                    "github_org",
                    "your GitHub organization's name, as in github.com/<name>",
                ));
            }
            (b.kind.as_str(), Some(org.to_string()), "invite")
        }
        _ => return Err(ApiError::invalid("kind", "player or official")),
    };
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into teams (id, slug, name, kind, about, join_policy, created_by, github_org, dns_token)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(&slug)
    .bind(&name)
    .bind(kind)
    .bind(&about)
    .bind(policy)
    .bind(user.id)
    .bind(&org)
    .bind(org.as_ref().map(|_| crate::tokens::new_token()[..24].to_string()))
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        if is_unique_violation(&err) {
            ApiError::Conflict("that address or organization already has a team".into())
        } else {
            err.into()
        }
    })?;
    sqlx::query("insert into team_members (team_id, user_id, role) values ($1, $2, 'owner')")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "slug": slug }))))
}

struct Team {
    id: Uuid,
    name: String,
    kind: String,
    join_policy: String,
    github_org: Option<String>,
    hidden: bool,
    /// The viewer's role, if a member.
    role: Option<String>,
}

async fn load(db: &PgPool, slug: &str, viewer: Option<Uuid>) -> ApiResult<Team> {
    #[allow(clippy::type_complexity)] // one row of the query below
    let row: Option<(
        Uuid,
        String,
        String,
        String,
        Option<String>,
        bool,
        Option<String>,
    )> = sqlx::query_as(
        "select t.id, t.name, t.kind, t.join_policy, t.github_org, t.hidden_reason is not null,
                (select m.role from team_members m where m.team_id = t.id and m.user_id = $2)
         from teams t where t.slug = $1",
    )
    .bind(slug)
    .bind(viewer)
    .fetch_optional(db)
    .await?;
    let (id, name, kind, join_policy, github_org, hidden, role) = row.ok_or(ApiError::NotFound)?;
    if hidden && role.is_none() {
        return Err(ApiError::NotFound);
    }
    Ok(Team {
        id,
        name,
        kind,
        join_policy,
        github_org,
        hidden,
        role,
    })
}

impl Team {
    fn is_admin(&self) -> bool {
        matches!(self.role.as_deref(), Some("owner" | "admin"))
    }
    fn admin(self) -> ApiResult<Self> {
        if self.is_admin() {
            Ok(self)
        } else if self.role.is_some() {
            Err(ApiError::Conflict(
                "only the team's owner and admins can do that".into(),
            ))
        } else {
            Err(ApiError::NotFound)
        }
    }
    fn member(self) -> ApiResult<Self> {
        if self.role.is_some() {
            Ok(self)
        } else {
            Err(ApiError::NotFound)
        }
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct Member {
    user_id: Uuid,
    name: String,
    role: String,
    #[serde(with = "time::serde::rfc3339")]
    joined_at: OffsetDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
struct Pick {
    id: Uuid,
    #[sqlx(flatten)]
    #[serde(flatten)]
    app: AppSummary,
    note: String,
    by: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct Event {
    id: Uuid,
    title: String,
    #[serde(with = "time::serde::rfc3339")]
    starts_at: OffsetDateTime,
    app_slug: Option<String>,
    app_title: Option<String>,
    by: String,
    mine: bool,
}

#[derive(Serialize)]
struct Page {
    slug: String,
    name: String,
    kind: String,
    about: String,
    join_policy: String,
    github_org: Option<String>,
    verified: bool,
    /// Official teams: each check and whether it passes (shown to members; the badge to all).
    checks: Value,
    #[serde(with = "time::serde::rfc3339::option")]
    checked_at: Option<OffsetDateTime>,
    /// For the DNS check, to members.
    dns: Option<Value>,
    /// The viewer's role, if a member.
    role: Option<String>,
    invited: bool,
    /// Members are listed to members, and for verified official teams to everyone.
    members: Vec<Member>,
    member_count: i64,
    /// Official: the apps the studio publishes from its organization.
    apps: Vec<AppSummary>,
    picks: Vec<Pick>,
    events: Vec<Event>,
    /// Private Slop shared with this player team (members only).
    shared: Vec<AppSummary>,
    hidden: bool,
}

async fn page(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Page>> {
    let viewer = user.as_ref().map(|u| u.id);
    let t = load(&state.db, &slug, viewer).await?;
    let (about, verified, checks, checked_at, dns_domain, dns_token): (String, bool, Value, Option<OffsetDateTime>, Option<String>, Option<String>) =
        sqlx::query_as(
            "select about, verified_at is not null, checks, checked_at, dns_domain, dns_token from teams where id = $1",
        )
        .bind(t.id)
        .fetch_one(&state.db)
        .await?;
    let invited: bool =
        match viewer {
            Some(v) => sqlx::query_scalar(
                "select exists (select 1 from team_invites where team_id = $1 and user_id = $2)",
            )
            .bind(t.id)
            .bind(v)
            .fetch_one(&state.db)
            .await?,
            None => false,
        };
    // Invite-only player teams are for their members (and the people invited).
    if t.kind == "player" && t.join_policy == "invite" && t.role.is_none() && !invited {
        return Err(ApiError::NotFound);
    }
    let is_member = t.role.is_some();
    let show_members = is_member
        || (t.kind == "official" && verified)
        || (t.kind == "player" && t.join_policy == "open");
    let members: Vec<Member> = if show_members {
        sqlx::query_as(
            "select m.user_id, u.display_name as name, m.role, m.joined_at from team_members m join users u on u.id = m.user_id
             where m.team_id = $1 and u.closed_at is null
             order by case m.role when 'owner' then 0 when 'admin' then 1 else 2 end, u.display_name",
        )
        .bind(t.id)
        .fetch_all(&state.db)
        .await?
    } else {
        Vec::new()
    };
    let member_count: i64 =
        sqlx::query_scalar("select count(*) from team_members where team_id = $1")
            .bind(t.id)
            .fetch_one(&state.db)
            .await?;
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let apps = match &t.github_org {
        Some(org) => team_apps(&state, t.id, org, policy).await?,
        None => Vec::new(),
    };
    let (picks, events, shared) = if is_member && t.kind == "player" {
        let mut qb = QueryBuilder::new("");
        catalog::push_summary_columns_with(
            &mut qb,
            &state.config,
            ", p.id, p.note, (select u.display_name from users u where u.id = p.user_id) as by",
        );
        qb.push(" join team_picks p on p.app_id = a.id where p.team_id = ")
            .push_bind(t.id)
            .push(" and ");
        catalog::push_store_visible(&mut qb, state.config.catalog, policy);
        qb.push(" order by p.created_at desc limit 50");
        let picks: Vec<Pick> = qb.build_query_as().fetch_all(&state.db).await?;
        let events: Vec<Event> = sqlx::query_as(
            "select e.id, e.title, e.starts_at, a.slug as app_slug, a.title as app_title, u.display_name as by,
                    e.created_by = $2 as mine
             from team_events e join users u on u.id = e.created_by left join apps a on a.id = e.app_id
             where e.team_id = $1 and e.starts_at > now() - interval '6 hours' order by e.starts_at limit 20",
        )
        .bind(t.id)
        .bind(viewer)
        .fetch_all(&state.db)
        .await?;
        let mut qb = QueryBuilder::new("");
        catalog::push_summary_columns(&mut qb, &state.config);
        qb.push(" join app_team_shares s on s.app_id = a.id where s.team_id = ")
            .push_bind(t.id)
            .push(" and a.visibility = 'private' and exists (select 1 from releases r where r.app_id = a.id and r.status = 'approved')")
            .push(" order by a.title");
        let shared: Vec<AppSummary> = qb.build_query_as().fetch_all(&state.db).await?;
        (picks, events, shared)
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    Ok(Json(Page {
        slug,
        name: t.name.clone(),
        kind: t.kind.clone(),
        about,
        join_policy: t.join_policy.clone(),
        github_org: t.github_org.clone(),
        verified,
        checks: if is_member { checks } else { json!([]) },
        checked_at,
        dns: (is_member && t.kind == "official").then(|| {
            json!({ "domain": dns_domain, "record": "_slop-team.<your domain>", "value": format!("slop-team={}", dns_token.unwrap_or_default()) })
        }),
        role: t.role.clone(),
        invited,
        members,
        member_count,
        apps,
        picks,
        events,
        shared,
        hidden: t.hidden,
    }))
}

/// An official team's apps: in the store, from repositories inside its organization, published by
/// a member (DESIGN §14 check 4).
async fn team_apps(
    state: &AppState,
    team_id: Uuid,
    org: &str,
    policy: ViewerPolicy,
) -> sqlx::Result<Vec<AppSummary>> {
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns(&mut qb, &state.config);
    qb.push(" where lower(a.repo_url) like ")
        .push_bind(format!("https://github.com/{}/%", org.to_ascii_lowercase()))
        .push(" and a.maker_id in (select user_id from team_members where team_id = ")
        .push_bind(team_id)
        .push(") and a.parent_app_id is null and ");
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(" order by a.title");
    qb.build_query_as().fetch_all(&state.db).await
}

/// For an app's page: the verified official team that publishes it, if any.
pub async fn official_team_of(db: &PgPool, app_id: Uuid) -> sqlx::Result<Option<(String, String)>> {
    sqlx::query_as(
        "select t.slug, t.name from teams t join apps a on a.id = $1
         where t.kind = 'official' and t.verified_at is not null and t.hidden_reason is null
           and lower(a.repo_url) like 'https://github.com/' || lower(t.github_org) || '/%'
           and exists (select 1 from team_members m where m.team_id = t.id and m.user_id = a.maker_id)
         limit 1",
    )
    .bind(app_id)
    .fetch_optional(db)
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    name: Option<String>,
    about: Option<String>,
    join_policy: Option<String>,
    /// Official teams: a website domain to prove with a DNS record instead of GitHub's badge.
    dns_domain: Option<String>,
}

async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<UpdateBody>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?.admin()?;
    if let Some(name) = &b.name {
        sqlx::query("update teams set name = $2 where id = $1")
            .bind(t.id)
            .bind(text("name", name, 1, 60)?)
            .execute(&state.db)
            .await?;
    }
    if let Some(about) = &b.about {
        sqlx::query("update teams set about = $2 where id = $1")
            .bind(t.id)
            .bind(text("about", about, 0, 2000)?)
            .execute(&state.db)
            .await?;
    }
    if let Some(p) = &b.join_policy {
        if t.kind != "player" || !["open", "invite"].contains(&p.as_str()) {
            return Err(ApiError::invalid(
                "join_policy",
                "player teams: open or invite",
            ));
        }
        sqlx::query("update teams set join_policy = $2 where id = $1")
            .bind(t.id)
            .bind(p)
            .execute(&state.db)
            .await?;
    }
    if let Some(d) = &b.dns_domain {
        let d = d.trim().trim_end_matches('.').to_ascii_lowercase();
        if t.kind != "official"
            || (!d.is_empty()
                && (!d.contains('.')
                    || d.len() > 253
                    || !d
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')))
        {
            return Err(ApiError::invalid(
                "dns_domain",
                "your studio's website domain, like studio.example",
            ));
        }
        sqlx::query("update teams set dns_domain = nullif($2, '') where id = $1")
            .bind(t.id)
            .bind(&d)
            .execute(&state.db)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn join(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?;
    if t.kind != "player" || t.join_policy != "open" || t.hidden {
        return Err(ApiError::Conflict("this team is by invitation".into()));
    }
    sqlx::query(
        "insert into team_members (team_id, user_id) values ($1, $2) on conflict do nothing",
    )
    .bind(t.id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn leave(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?.member()?;
    let mut tx = state.db.begin().await?;
    if t.role.as_deref() == Some("owner") {
        let others: i64 = sqlx::query_scalar("select count(*) from team_members where team_id = $1 and role = 'owner' and user_id <> $2")
            .bind(t.id)
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?;
        let members: i64 =
            sqlx::query_scalar("select count(*) from team_members where team_id = $1")
                .bind(t.id)
                .fetch_one(&mut *tx)
                .await?;
        if others == 0 && members > 1 {
            return Err(ApiError::Conflict(
                "make someone else an owner before you leave".into(),
            ));
        }
    }
    sqlx::query("delete from team_members where team_id = $1 and user_id = $2")
        .bind(t.id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    // The last one out ends the team.
    sqlx::query("delete from teams where id = $1 and not exists (select 1 from team_members where team_id = $1)")
        .bind(t.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteBody {
    /// Their profile link or account ID.
    user: String,
}

async fn invite(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<InviteBody>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?.admin()?;
    let to =
        parse_user(&b.user).ok_or_else(|| ApiError::invalid("user", "paste their profile link"))?;
    let ok: bool = sqlx::query_scalar(
        "select exists (select 1 from users where id = $1 and closed_at is null and suspended_at is null)
            and not exists (select 1 from team_members where team_id = $2 and user_id = $1)",
    )
    .bind(to)
    .bind(t.id)
    .fetch_one(&state.db)
    .await?;
    if !ok {
        return Err(ApiError::invalid(
            "user",
            "no Slop account with that link, or they're already in the team",
        ));
    }
    let mut tx = state.db.begin().await?;
    let added = sqlx::query("insert into team_invites (team_id, user_id, invited_by) values ($1, $2, $3) on conflict do nothing")
        .bind(t.id)
        .bind(to)
        .bind(user.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if added > 0 {
        notify::send(
            &mut tx,
            Notice {
                user_id: to,
                category: "teams",
                title: format!("{} invited you to {}", user.display_name, t.name),
                body: String::new(),
                link: format!("/teams/{slug}"),
                collapse_key: None,
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn accept_invite(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?;
    let mut tx = state.db.begin().await?;
    let n = sqlx::query("delete from team_invites where team_id = $1 and user_id = $2")
        .bind(t.id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    sqlx::query(
        "insert into team_members (team_id, user_id) values ($1, $2) on conflict do nothing",
    )
    .bind(t.id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn decline_invite(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?;
    sqlx::query("delete from team_invites where team_id = $1 and user_id = $2")
        .bind(t.id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, member)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?.admin()?;
    let role: Option<String> =
        sqlx::query_scalar("select role from team_members where team_id = $1 and user_id = $2")
            .bind(t.id)
            .bind(member)
            .fetch_optional(&state.db)
            .await?;
    match role.as_deref() {
        None => return Err(ApiError::NotFound),
        Some("owner") => return Err(ApiError::Conflict("owners leave by themselves".into())),
        Some("admin") if t.role.as_deref() != Some("owner") => {
            return Err(ApiError::Conflict(
                "only an owner can remove an admin".into(),
            ));
        }
        _ => {}
    }
    sqlx::query("delete from team_members where team_id = $1 and user_id = $2")
        .bind(t.id)
        .bind(member)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleBody {
    role: String,
}

async fn set_role(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, member)): Path<(String, Uuid)>,
    Json(b): Json<RoleBody>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?;
    if t.role.as_deref() != Some("owner") {
        return if t.role.is_some() {
            Err(ApiError::Conflict("only an owner can change roles".into()))
        } else {
            Err(ApiError::NotFound)
        };
    }
    if !["owner", "admin", "member"].contains(&b.role.as_str()) {
        return Err(ApiError::invalid("role", "owner, admin or member"));
    }
    if member == user.id {
        return Err(ApiError::invalid(
            "role",
            "ask another owner to change your role",
        ));
    }
    let n = sqlx::query("update team_members set role = $3 where team_id = $1 and user_id = $2")
        .bind(t.id)
        .bind(member)
        .bind(&b.role)
        .execute(&state.db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PickBody {
    app_slug: String,
    #[serde(default)]
    note: String,
}

async fn add_pick(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(b): Json<PickBody>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?.member()?;
    if t.kind != "player" {
        return Err(ApiError::Conflict("picks are for player teams".into()));
    }
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, b.app_slug.trim(), Some(&user), policy).await?;
    if catalog::is_private(&state.db, app_id).await? {
        return Err(ApiError::invalid(
            "app_slug",
            "pick apps from the store; share Private Slop from its page",
        ));
    }
    sqlx::query(
        "insert into team_picks (id, team_id, app_id, user_id, note) values ($1, $2, $3, $4, $5)
         on conflict (team_id, app_id) do update set note = excluded.note, user_id = excluded.user_id",
    )
    .bind(Uuid::now_v7())
    .bind(t.id)
    .bind(app_id)
    .bind(user.id)
    .bind(text("note", &b.note, 0, 280)?)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_pick(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?.member()?;
    let n = sqlx::query(
        "delete from team_picks where id = $1 and team_id = $2 and (user_id = $3 or $4)",
    )
    .bind(id)
    .bind(t.id)
    .bind(user.id)
    .bind(t.is_admin())
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventBody {
    title: String,
    #[serde(with = "time::serde::rfc3339")]
    starts_at: OffsetDateTime,
    app_slug: Option<String>,
}

/// A game night: every other member is told.
async fn add_event(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(b): Json<EventBody>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?.member()?;
    if t.kind != "player" {
        return Err(ApiError::Conflict(
            "game nights are for player teams".into(),
        ));
    }
    let title = text("title", &b.title, 1, 100)?;
    let now = OffsetDateTime::now_utc();
    if b.starts_at < now - time::Duration::hours(1) || b.starts_at > now + time::Duration::days(365)
    {
        return Err(ApiError::invalid("starts_at", "a time in the coming year"));
    }
    let app_id = match b
        .app_slug
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => {
            let policy =
                ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
            Some(
                catalog::visible_app(&state, s, Some(&user), policy)
                    .await?
                    .0,
            )
        }
        None => None,
    };
    let mut tx = state.db.begin().await?;
    sqlx::query("insert into team_events (id, team_id, app_id, title, starts_at, created_by) values ($1, $2, $3, $4, $5, $6)")
        .bind(Uuid::now_v7())
        .bind(t.id)
        .bind(app_id)
        .bind(&title)
        .bind(b.starts_at)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let members: Vec<Uuid> =
        sqlx::query_scalar("select user_id from team_members where team_id = $1 and user_id <> $2")
            .bind(t.id)
            .bind(user.id)
            .fetch_all(&mut *tx)
            .await?;
    for m in members {
        notify::send(
            &mut tx,
            Notice {
                user_id: m,
                category: "teams",
                title: format!("{}: {title}", t.name),
                body: format!(
                    "Game night on {} (UTC), set by {}.",
                    b.starts_at.date(),
                    user.display_name
                ),
                link: format!("/teams/{slug}"),
                collapse_key: None,
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::CREATED)
}

async fn remove_event(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let t = load(&state.db, &slug, Some(user.id)).await?.member()?;
    let n = sqlx::query(
        "delete from team_events where id = $1 and team_id = $2 and (created_by = $3 or $4)",
    )
    .bind(id)
    .bind(t.id)
    .bind(user.id)
    .bind(t.is_admin())
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Official team verification

#[derive(Serialize, Debug, PartialEq)]
pub struct Check {
    pub check: &'static str,
    pub ok: bool,
    pub detail: String,
}

pub(crate) async fn github_get(
    state: &AppState,
    token: &str,
    path: &str,
) -> anyhow::Result<Option<Value>> {
    let url = state.config.releases.github_api_base.join(path)?;
    let res = state
        .http
        .get(url)
        .bearer_auth(token)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "slop-api")
        .send()
        .await?;
    if res.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(res.error_for_status()?.json().await?))
}

/// Whether `_slop-team.<domain>` has the team's TXT record (DNS over HTTPS, JSON).
async fn dns_has(state: &AppState, domain: &str, value: &str) -> anyhow::Result<bool> {
    let mut url = state.config.doh_url.clone();
    url.query_pairs_mut()
        .append_pair("name", &format!("_slop-team.{domain}"))
        .append_pair("type", "TXT");
    let body: Value = state
        .http
        .get(url)
        .header("accept", "application/dns-json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(body["Answer"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["data"].as_str())
        .any(|d| d.trim_matches('"') == value))
}

/// Runs every check for an official team. The person checked is its longest-standing owner.
pub async fn run_checks(state: &AppState, team_id: Uuid) -> anyhow::Result<Vec<Check>> {
    let (org, creator, dns_domain, dns_token): (String, Option<Uuid>, Option<String>, Option<String>) = sqlx::query_as(
        "select t.github_org,
                (select m.user_id from team_members m where m.team_id = t.id and m.role = 'owner' order by m.joined_at limit 1),
                t.dns_domain, t.dns_token
         from teams t where t.id = $1 and t.kind = 'official'",
    )
    .bind(team_id)
    .fetch_one(&state.db)
    .await?;
    let mut out = Vec::new();
    let Some(token) = crate::github_app::org_token(state, &org).await? else {
        out.push(Check {
            check: "app_installed",
            ok: false,
            detail: format!(
                "Install Slop's GitHub App on the {org} organization so Slop can check it."
            ),
        });
        return Ok(out);
    };
    // 1. The owner owns the organization on GitHub too.
    let subject: Option<String> = sqlx::query_scalar(
        "select provider_subject from identities where user_id = $1 and provider = 'github' limit 1",
    )
    .bind(creator)
    .fetch_optional(&state.db)
    .await?;
    let login = match &subject {
        Some(id) => github_get(state, &token, &format!("user/{id}"))
            .await?
            .and_then(|u| u["login"].as_str().map(String::from)),
        None => None,
    };
    let owner = match &login {
        Some(login) => github_get(state, &token, &format!("orgs/{org}/memberships/{login}"))
            .await?
            .is_some_and(|m| m["role"] == "admin" && m["state"] == "active"),
        None => false,
    };
    out.push(Check {
        check: "owner",
        ok: owner,
        detail: match &login {
            Some(l) if owner => format!("@{l} is an owner of {org}."),
            Some(l) => format!("@{l} isn't an owner of {org} on GitHub."),
            None => {
                "The team's owner signs in with GitHub; Slop couldn't find their GitHub account."
                    .into()
            }
        },
    });
    let info = github_get(state, &token, &format!("orgs/{org}"))
        .await?
        .unwrap_or(Value::Null);
    // 2. A verified domain: GitHub's badge, or the DNS record.
    let github_verified = info["is_verified"].as_bool() == Some(true);
    let dns_ok = match (&dns_domain, &dns_token) {
        (Some(d), Some(t)) if !github_verified => dns_has(state, d, &format!("slop-team={t}"))
            .await
            .unwrap_or(false),
        _ => false,
    };
    out.push(Check {
        check: "domain",
        ok: github_verified || dns_ok,
        detail: if github_verified {
            "GitHub shows the organization's domain as verified.".into()
        } else if dns_ok {
            format!("The DNS record on {} matches.", dns_domain.clone().unwrap_or_default())
        } else {
            "Verify a domain on GitHub, or add the DNS record Slop shows you to your studio's domain.".into()
        },
    });
    // 3. Two-factor authentication required for every member.
    let two_factor = info["two_factor_requirement_enabled"].as_bool() == Some(true);
    out.push(Check {
        check: "two_factor",
        ok: two_factor,
        detail: if two_factor {
            "Two-factor authentication is required for every member.".into()
        } else {
            "Require two-factor authentication in the organization's settings (Slop's GitHub App needs to read the organization's administration settings).".into()
        },
    });
    // 4. Apps come from the organization's repositories: only those show as the team's.
    let outside: i64 = sqlx::query_scalar(
        "select count(*) from apps a where a.status = 'listed' and a.parent_app_id is null
           and a.maker_id in (select user_id from team_members where team_id = $1)
           and lower(a.repo_url) not like 'https://github.com/' || lower($2) || '/%'",
    )
    .bind(team_id)
    .bind(&org)
    .fetch_one(&state.db)
    .await?;
    out.push(Check {
        check: "repositories",
        ok: true,
        detail: if outside == 0 {
            format!("The team's apps come from repositories in github.com/{org}.")
        } else {
            format!("Only apps from github.com/{org} show as the team's; {outside} of the members' apps are elsewhere and don't.")
        },
    });
    // 5. Donations: the studio's Stripe account is a business.
    let donating: Vec<String> = sqlx::query_scalar(
        "select distinct d.stripe_account_id from apps a join donation_accounts d on d.user_id = a.maker_id
         where a.donations_on and a.maker_id in (select user_id from team_members where team_id = $1)
           and lower(a.repo_url) like 'https://github.com/' || lower($2) || '/%'",
    )
    .bind(team_id)
    .bind(&org)
    .fetch_all(&state.db)
    .await?;
    let mut business = true;
    if let crate::people::stripe::Mode::Stripe(stripe) = crate::people::stripe::mode(state) {
        for acct in &donating {
            let a = crate::people::stripe::call(
                state,
                stripe,
                reqwest::Method::GET,
                &format!("v1/accounts/{acct}"),
                None,
                &[],
            )
            .await?;
            business &= a["business_type"] == "company" || a["business_type"] == "non_profit";
        }
    }
    if !donating.is_empty() {
        out.push(Check {
            check: "business",
            ok: business,
            detail: if business {
                "Stripe verified the studio's donation account as a business.".into()
            } else {
                "The studio takes donations, so its Stripe account must be verified as a business."
                    .into()
            },
        });
    }
    Ok(out)
}

/// Runs the checks and sets or clears the badge.
pub async fn recheck(state: &AppState, team_id: Uuid) -> anyhow::Result<bool> {
    let checks = run_checks(state, team_id).await?;
    let ok = checks.iter().all(|c| c.ok);
    let was: bool = sqlx::query_scalar(
        "update teams t set checks = $2, checked_at = now(),
                verified_at = case when $3 then coalesce(t.verified_at, now()) end
         from (select verified_at is not null as was from teams where id = $1) old
         where t.id = $1 returning old.was",
    )
    .bind(team_id)
    .bind(serde_json::to_value(&checks)?)
    .bind(ok)
    .fetch_one(&state.db)
    .await?;
    if was && !ok {
        let owners: Vec<Uuid> = sqlx::query_scalar(
            "select user_id from team_members where team_id = $1 and role = 'owner'",
        )
        .bind(team_id)
        .fetch_all(&state.db)
        .await?;
        let mut tx = state.db.begin().await?;
        for o in owners {
            notify::send(
                &mut tx,
                Notice {
                    user_id: o,
                    category: "teams",
                    title: "Your team's official badge was removed".into(),
                    body: "A verification check stopped passing. See which on the team page."
                        .into(),
                    link: "/teams".into(),
                    collapse_key: None,
                },
            )
            .await?;
        }
        tx.commit().await?;
    }
    Ok(ok)
}

async fn verify(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let t = load(&state.db, &slug, Some(user.id)).await?.admin()?;
    if t.kind != "official" {
        return Err(ApiError::Conflict(
            "only official teams are verified".into(),
        ));
    }
    let ok = recheck(&state, t.id).await?;
    let checks: Value = sqlx::query_scalar("select checks from teams where id = $1")
        .bind(t.id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(json!({ "verified": ok, "checks": checks })))
}

/// Daily: re-checks every official team (DESIGN §14: the badge goes when a check stops passing).
pub async fn run(state: AppState) {
    loop {
        let teams: Vec<Uuid> = sqlx::query_scalar(
            "select id from teams where kind = 'official' and (checked_at is null or checked_at < now() - interval '1 day')",
        )
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
        for id in teams {
            if let Err(err) = recheck(&state, id).await {
                tracing::warn!(team = %id, error = ?err, "official team check failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}
