// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fix bounties (plan phase 12; PAYMENTS §5.4).
//!
//! - **Anyone signed in opens one** on a public app: what they want fixed or added, in one
//!   currency. It's public, reportable, and expires after 180 days without a delivery.
//! - **Pledging saves a card, nothing more.** The backer goes through Stripe Checkout in setup
//!   mode on Slop's platform account; Slop keeps only Stripe's IDs. A pledge can be withdrawn
//!   until a delivery is confirmed.
//! - **Delivering:** the app's maker, or the maker of a community build of it, claims an approved
//!   release delivers it. They need a donation account (their own Stripe account) to be paid.
//! - **Confirming:** the app's maker accepts or declines. If they made the claim themselves, or
//!   don't answer within 14 days, the backers vote: more than half of the pledged amount decides.
//! - **Paying:** each pledge is charged off-session as a **direct charge on the builder's own
//!   Stripe account** (the card is cloned there), with Slop's 5% as the application fee, and
//!   recorded in `payments`. Slop never holds the money. A refused card marks that pledge failed
//!   and tells its backer; the rest still pay. Stripe being down leaves it due for the next run.
//! - Without Stripe keys, pledges and charges are simulated where People payments are.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog,
    config::StripeConfig,
    error::{ApiError, ApiResult},
    notify::{self, Notice},
    people::stripe::{self, Mode, call, fee},
    policy::ViewerPolicy,
};

/// Smallest and largest pledge, in the currency's minor unit.
pub const MIN_PLEDGE: i64 = 100;
pub const MAX_PLEDGE: i64 = 50_000;
/// Open bounties one person may have at once.
const MAX_OPEN_PER_USER: i64 = 10;
/// After this long without the app maker's answer, the backers decide a claim.
pub const MAKER_ANSWER: Duration = Duration::days(14);
/// How long backers have to vote before a majority settles a claim (unless every one of them
/// has voted), so the pledges' owners can look at the delivery first.
pub const VOTING_WINDOW: Duration = Duration::days(3);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/bounties", get(list_for_app).post(open))
        .route("/bounties", get(list_all))
        .route("/bounties/{id}", get(detail))
        .route("/bounties/{id}/pledge", post(pledge).delete(withdraw))
        .route("/bounties/{id}/pledge/check", post(check_pledge))
        .route("/bounties/{id}/claims", post(claim))
        .route("/bounties/{id}/claims/{claim}/answer", post(answer))
        .route("/bounties/{id}/claims/{claim}/vote", post(vote))
        .route(
            "/bounties/{id}/claims/{claim}/withdraw",
            post(withdraw_claim),
        )
}

// ---------------------------------------------------------------------------------------------
// Reading

#[derive(Serialize, sqlx::FromRow)]
pub struct Summary {
    id: Uuid,
    app_slug: String,
    app_title: String,
    title: String,
    body: String,
    currency: String,
    status: String,
    /// Pledges that count: saved cards waiting (active), or being or already charged.
    backers: i64,
    pledged_minor: i64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

const SUMMARY: &str = "select b.id, a.slug as app_slug, a.title as app_title, b.title, b.body, b.currency, b.status,
        (select count(*) from bounty_pledges p where p.bounty_id = b.id and p.status in ('active', 'due', 'paid')) as backers,
        (select coalesce(sum(p.amount_minor), 0)::int8 from bounty_pledges p
         where p.bounty_id = b.id and p.status in ('active', 'due', 'paid')) as pledged_minor,
        b.created_at, b.expires_at
    from bounties b join apps a on a.id = b.app_id";

async fn list_for_app(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Summary>>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SUMMARY} where b.app_id = $1 and b.status <> 'hidden'
         order by b.status in ('open', 'claimed') desc, pledged_minor desc, b.created_at desc limit 50"
    )))
    .bind(app_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct AllQuery {
    offset: Option<i64>,
}

/// Open bounties across the store, biggest first: paid work for builders.
async fn list_all(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Query(q): Query<AllQuery>,
) -> ApiResult<Json<Vec<Summary>>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let mut qb = sqlx::QueryBuilder::new(SUMMARY);
    qb.push(" where b.status in ('open', 'claimed') and ");
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(" order by pledged_minor desc, b.created_at desc limit 50 offset ")
        .push_bind(q.offset.unwrap_or(0).clamp(0, 10_000));
    Ok(Json(qb.build_query_as().fetch_all(&state.db).await?))
}

#[derive(Serialize, sqlx::FromRow)]
struct Claim {
    id: Uuid,
    builder: String,
    app_slug: String,
    app_title: String,
    version: String,
    note: String,
    status: String,
    decided_by: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    /// Backers' votes so far, by amount pledged.
    accept_minor: i64,
    reject_minor: i64,
}

#[derive(Serialize)]
struct Detail {
    #[serde(flatten)]
    summary: Summary,
    claims: Vec<Claim>,
    /// The viewer's pledge: amount and status.
    my_pledge: Option<MyPledge>,
    /// The viewer makes the app, so they answer claims.
    is_app_maker: bool,
    /// The viewer's vote on the pending claim, if they can vote.
    can_vote: bool,
    my_vote: Option<bool>,
    /// Payments run on Stripe, are simulated, or are off on this server.
    payments: &'static str,
}

#[derive(Serialize, sqlx::FromRow)]
struct MyPledge {
    amount_minor: i32,
    status: String,
    failure: Option<String>,
}

async fn load_visible(
    state: &AppState,
    id: Uuid,
    user: Option<&CurrentUser>,
    headers: &HeaderMap,
) -> ApiResult<(Uuid, String, Uuid)> {
    let row: Option<(String, Uuid, String)> = sqlx::query_as(
        "select a.slug, a.maker_id, b.status from bounties b join apps a on a.id = b.app_id where b.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (slug, maker, status) = row.ok_or(ApiError::NotFound)?;
    if status == "hidden" {
        return Err(ApiError::NotFound);
    }
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user, headers).await?;
    let (app_id, _) = catalog::visible_app(state, &slug, user, policy).await?;
    Ok((app_id, status, maker))
}

async fn detail(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Detail>> {
    let (_, _, maker) = load_visible(&state, id, user.as_ref(), &headers).await?;
    let summary: Summary =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("{SUMMARY} where b.id = $1")))
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    let claims: Vec<Claim> = sqlx::query_as(
        "select c.id, u.display_name as builder, a.slug as app_slug, a.title as app_title, r.version, c.note,
                c.status, c.decided_by, c.created_at,
                coalesce((select sum(p.amount_minor) from bounty_votes v join bounty_pledges p
                          on p.bounty_id = c.bounty_id and p.user_id = v.user_id and p.status = 'active'
                          where v.claim_id = c.id and v.accept), 0)::int8 as accept_minor,
                coalesce((select sum(p.amount_minor) from bounty_votes v join bounty_pledges p
                          on p.bounty_id = c.bounty_id and p.user_id = v.user_id and p.status = 'active'
                          where v.claim_id = c.id and not v.accept), 0)::int8 as reject_minor
         from bounty_claims c join users u on u.id = c.claimed_by join apps a on a.id = c.app_id
         join releases r on r.id = c.release_id
         where c.bounty_id = $1 order by c.created_at desc",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let viewer = user.as_ref().map(|u| u.id);
    let my_pledge: Option<MyPledge> = sqlx::query_as(
        "select amount_minor, status, failure from bounty_pledges where bounty_id = $1 and user_id = $2
         order by created_at desc limit 1",
    )
    .bind(id)
    .bind(viewer)
    .fetch_optional(&state.db)
    .await?;
    let pending = claims.iter().find(|c| c.status == "pending").map(|c| c.id);
    let (can_vote, my_vote) = match (viewer, pending) {
        (Some(v), Some(claim)) => {
            // The vote's own rule (COUNTED is a constant): never the deliverer, the app's maker
            // or a pledge made after the delivery.
            let counted: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "select exists (select 1 from bounty_pledges p where p.bounty_id = $1 and p.user_id = $3 and {COUNTED})"
            )))
            .bind(id)
            .bind(claim)
            .bind(v)
            .fetch_one(&state.db)
            .await?;
            let can = counted && backers_decide(&state.db, claim).await?;
            let mine: Option<bool> = sqlx::query_scalar(
                "select accept from bounty_votes where claim_id = $1 and user_id = $2",
            )
            .bind(claim)
            .bind(v)
            .fetch_optional(&state.db)
            .await?;
            (can, mine)
        }
        _ => (false, None),
    };
    Ok(Json(Detail {
        summary,
        claims,
        my_pledge,
        is_app_maker: viewer == Some(maker),
        can_vote,
        my_vote,
        payments: stripe::mode(&state).name(),
    }))
}

// ---------------------------------------------------------------------------------------------
// Opening and pledging

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewBounty {
    title: String,
    body: String,
    currency: String,
}

async fn open(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(b): Json<NewBounty>,
) -> ApiResult<(StatusCode, Json<Summary>)> {
    crate::limits::check(&state, user.id, crate::limits::Action::Bounty)?;
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, &slug, Some(&user), policy).await?;
    if catalog::is_private(&state.db, app_id).await? {
        return Err(ApiError::Conflict("Private Slop has no bounties".into()));
    }
    let title = b.title.trim();
    let body = b.body.trim();
    if title.is_empty() || title.chars().count() > 100 {
        return Err(ApiError::invalid("title", "1 to 100 characters"));
    }
    if body.is_empty() || body.chars().count() > 2000 {
        return Err(ApiError::invalid("body", "1 to 2,000 characters"));
    }
    if !crate::people::CURRENCIES.contains(&b.currency.as_str()) {
        return Err(ApiError::invalid("currency", "usd, eur or gbp"));
    }
    let open: i64 = sqlx::query_scalar(
        "select count(*) from bounties where created_by = $1 and status in ('open', 'claimed')",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    if open >= MAX_OPEN_PER_USER {
        return Err(ApiError::Conflict(format!(
            "you have {MAX_OPEN_PER_USER} open bounties already"
        )));
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into bounties (id, app_id, created_by, title, body, currency) values ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(app_id)
    .bind(user.id)
    .bind(title)
    .bind(body)
    .bind(&b.currency)
    .execute(&state.db)
    .await?;
    let s = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SUMMARY} where b.id = $1")))
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok((StatusCode::CREATED, Json(s)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPledge {
    amount_minor: i64,
    /// Pledging is for adults, like donating (PAYMENTS §4.4).
    #[serde(default)]
    confirm_adult: bool,
    /// Needed after a few unfinished pledges (challenge.rs), as for donations.
    #[serde(default)]
    challenge: Option<crate::challenge::Answer>,
}

#[derive(Serialize)]
struct PledgeStarted {
    /// Stripe Checkout, to save the card; absent when simulated (the pledge is active at once).
    url: Option<String>,
    status: String,
}

async fn pledge(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<NewPledge>,
) -> ApiResult<Json<PledgeStarted>> {
    // Saving a card is a way to test stolen ones on Slop's own Stripe account: the same limits
    // as donations (security review, 2026-10-22).
    crate::limits::check(&state, user.id, crate::limits::Action::Donation)?;
    let (_, status, _) = load_visible(&state, id, Some(&user), &headers).await?;
    // Not while a delivery waits for a decision: a pledge made then could buy the vote on it.
    if status != "open" {
        return Err(ApiError::Conflict(if status == "claimed" {
            "a delivery is waiting for a decision: pledge again once it's settled".into()
        } else {
            "this bounty isn't taking pledges".into()
        }));
    }
    if !(MIN_PLEDGE..=MAX_PLEDGE).contains(&b.amount_minor) {
        return Err(ApiError::invalid("amount_minor", "1.00 to 500.00"));
    }
    crate::donations::require_adult(&state, &user, b.confirm_adult).await?;
    let unfinished: i64 = sqlx::query_scalar(
        "select count(*) from bounty_pledges where user_id = $1
           and status in ('saving', 'withdrawn', 'released') and created_at > now() - interval '1 day'",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    if unfinished >= crate::donations::UNFINISHED_BEFORE_CHALLENGE {
        crate::challenge::check(&state.db, user.id, b.challenge.as_ref()).await?;
    }
    let mode = stripe::mode(&state);
    if matches!(mode, Mode::Off) {
        return Err(ApiError::Unavailable(
            "payments aren't set up on this server",
        ));
    }
    let pledge_id = Uuid::now_v7();
    sqlx::query(
        "insert into bounty_pledges (id, bounty_id, user_id, amount_minor, status, simulated)
         values ($1, $2, $3, $4, $5, $6)",
    )
    .bind(pledge_id)
    .bind(id)
    .bind(user.id)
    .bind(b.amount_minor as i32)
    .bind(if matches!(mode, Mode::Simulated) {
        "active"
    } else {
        "saving"
    })
    .bind(matches!(mode, Mode::Simulated))
    .execute(&state.db)
    .await
    .map_err(|e| {
        if crate::error::is_unique_violation(&e) {
            ApiError::Conflict(
                "you've pledged to this bounty already; withdraw it to change the amount".into(),
            )
        } else {
            e.into()
        }
    })?;
    let Mode::Stripe(stripe) = mode else {
        return Ok(Json(PledgeStarted {
            url: None,
            status: "active".into(),
        }));
    };
    let url = setup_checkout(&state, stripe, &user, id, pledge_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(PledgeStarted {
        url: Some(url),
        status: "saving".into(),
    }))
}

/// The backer's Stripe customer on Slop's platform account, made once.
async fn customer(
    state: &AppState,
    stripe: &StripeConfig,
    user: &CurrentUser,
) -> anyhow::Result<String> {
    if let Some(id) = sqlx::query_scalar::<_, String>(
        "select customer_id from stripe_customers where user_id = $1",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?
    {
        return Ok(id);
    }
    let made = call(
        state,
        stripe,
        reqwest::Method::POST,
        "v1/customers",
        None,
        &[("metadata[slop_user]", user.id.to_string())],
    )
    .await?;
    let id = made["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Stripe gave no customer"))?
        .to_string();
    sqlx::query("insert into stripe_customers (user_id, customer_id) values ($1, $2) on conflict (user_id) do nothing")
        .bind(user.id)
        .bind(&id)
        .execute(&state.db)
        .await?;
    Ok(
        sqlx::query_scalar("select customer_id from stripe_customers where user_id = $1")
            .bind(user.id)
            .fetch_one(&state.db)
            .await?,
    )
}

async fn setup_checkout(
    state: &AppState,
    stripe: &StripeConfig,
    user: &CurrentUser,
    bounty: Uuid,
    pledge: Uuid,
) -> anyhow::Result<String> {
    let customer = customer(state, stripe, user).await?;
    let currency: String = sqlx::query_scalar("select currency from bounties where id = $1")
        .bind(bounty)
        .fetch_one(&state.db)
        .await?;
    let base = state.config.public_url.as_str().trim_end_matches('/');
    let session = call(
        state,
        stripe,
        reqwest::Method::POST,
        "v1/checkout/sessions",
        None,
        &[
            ("mode", "setup".into()),
            ("customer", customer),
            ("currency", currency),
            ("payment_method_types[0]", "card".into()),
            ("metadata[bounty_pledge_id]", pledge.to_string()),
            ("success_url", format!("{base}/bounties/{bounty}?pledged=1")),
            ("cancel_url", format!("{base}/bounties/{bounty}")),
        ],
    )
    .await?;
    let id = session["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Stripe gave no session"))?;
    sqlx::query("update bounty_pledges set checkout_session_id = $2 where id = $1")
        .bind(pledge)
        .bind(id)
        .execute(&state.db)
        .await?;
    session["url"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("Stripe gave no checkout link"))
}

/// Reads a pledge's Checkout session from Stripe: once the card is saved, the pledge is active.
/// Called when the backer comes back, and from the webhook.
pub async fn sync_pledge(
    state: &AppState,
    stripe: &StripeConfig,
    session_id: &str,
) -> anyhow::Result<()> {
    let session = call(
        state,
        stripe,
        reqwest::Method::GET,
        &format!("v1/checkout/sessions/{session_id}"),
        None,
        &[("expand[]", "setup_intent".into())],
    )
    .await?;
    let method = session["setup_intent"]["payment_method"].as_str();
    if session["status"] != "complete" || method.is_none() {
        return Ok(());
    }
    sqlx::query(
        "update bounty_pledges set status = 'active', payment_method_id = $2
         where checkout_session_id = $1 and status = 'saving'",
    )
    .bind(session_id)
    .bind(method)
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn check_pledge(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let session: Option<String> = sqlx::query_scalar(
        "select checkout_session_id from bounty_pledges
         where bounty_id = $1 and user_id = $2 and status = 'saving' and checkout_session_id is not null",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    if let (Some(session), Mode::Stripe(stripe)) = (session, stripe::mode(&state)) {
        sync_pledge(&state, stripe, &session)
            .await
            .map_err(ApiError::Internal)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn withdraw(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let row: Option<Option<String>> = sqlx::query_scalar(
        "update bounty_pledges p set status = 'withdrawn' from bounties b
         where p.bounty_id = $1 and p.user_id = $2 and p.status in ('saving', 'active')
           and b.id = p.bounty_id and b.status in ('open', 'claimed')
         returning p.payment_method_id",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    let Some(method) = row else {
        return Err(ApiError::Conflict(
            "there's no pledge to withdraw, or its delivery is already confirmed".into(),
        ));
    };
    if let (Some(pm), Mode::Stripe(stripe)) = (method, stripe::mode(&state)) {
        detach(&state, stripe, &pm).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Forgets a saved card at Stripe (best effort: Slop stops using it either way).
async fn detach(state: &AppState, stripe: &StripeConfig, payment_method: &str) {
    if let Err(e) = call(
        state,
        stripe,
        reqwest::Method::POST,
        &format!("v1/payment_methods/{payment_method}/detach"),
        None,
        &[],
    )
    .await
    {
        tracing::warn!(error = %e, "detaching a pledge's card");
    }
}

// ---------------------------------------------------------------------------------------------
// Claims and confirmation

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewClaim {
    /// The app it's delivered in: the bounty's app, or a community build of it.
    app_slug: String,
    note: String,
}

async fn claim(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(b): Json<NewClaim>,
) -> ApiResult<StatusCode> {
    let row: Option<(Uuid, String, Uuid, String)> = sqlx::query_as(
        "select b.app_id, b.status, a.maker_id, b.title from bounties b join apps a on a.id = b.app_id where b.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (bounty_app, status, app_maker, title) = row.ok_or(ApiError::NotFound)?;
    if status != "open" {
        return Err(ApiError::Conflict(if status == "claimed" {
            "a delivery is already waiting for confirmation".into()
        } else {
            "this bounty is closed".into()
        }));
    }
    let note = b.note.trim();
    if note.is_empty() || note.chars().count() > 2000 {
        return Err(ApiError::invalid("note", "1 to 2,000 characters"));
    }
    let (app_id, _) = crate::updates::owned(&state, &b.app_slug, &user).await?;
    let fits: bool = sqlx::query_scalar(
        "select id = $2 or coalesce(parent_app_id = $2, false) from apps where id = $1",
    )
    .bind(app_id)
    .bind(bounty_app)
    .fetch_one(&state.db)
    .await?;
    if !fits {
        return Err(ApiError::invalid(
            "app_slug",
            "the bounty's app, or a community build of it",
        ));
    }
    let release: Option<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select r.id {} limit 1",
        crate::releases::SERVING
    )))
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let release = release.ok_or_else(|| {
        ApiError::Conflict("release it first: a delivery is an approved release".into())
    })?;
    let account: Option<String> = sqlx::query_scalar(
        "select stripe_account_id from donation_accounts where user_id = $1 and stripe_status = 'active'",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    let account = match (account, stripe::mode(&state)) {
        (Some(a), _) => a,
        (None, Mode::Simulated) => format!("acct_sim_{}", user.id.simple()),
        (None, _) => {
            return Err(ApiError::Conflict(
                "set up receiving donations first (Account → Donations): bounties are paid to the same Stripe account".into(),
            ));
        }
    };
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into bounty_claims (id, bounty_id, claimed_by, app_id, release_id, stripe_account_id, note)
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(user.id)
    .bind(app_id)
    .bind(release)
    .bind(&account)
    .bind(note)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        // Another delivery got in first (bounty_claims_one_pending).
        if crate::error::is_unique_violation(&e) {
            ApiError::Conflict("a delivery is already waiting for confirmation".into())
        } else {
            e.into()
        }
    })?;
    sqlx::query("update bounties set status = 'claimed' where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let mut tell: Vec<Uuid> = sqlx::query_scalar(
        "select user_id from bounty_pledges where bounty_id = $1 and status = 'active'",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    if app_maker != user.id {
        tell.push(app_maker);
    }
    for who in tell {
        notify::send(
            &mut tx,
            Notice {
                user_id: who,
                category: "apps",
                title: format!("A delivery for \"{title}\""),
                body: format!(
                    "{} says it's done. {}",
                    user.display_name,
                    note.chars().take(150).collect::<String>()
                ),
                link: format!("/bounties/{id}"),
                collapse_key: Some(format!("bounty:{id}")),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Whether the backers decide a claim: the app's maker made it, or hasn't answered in time.
async fn backers_decide(db: &sqlx::PgPool, claim: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select c.claimed_by = a.maker_id or c.created_at < now() - $2
         from bounty_claims c join bounties b on b.id = c.bounty_id join apps a on a.id = b.app_id
         where c.id = $1",
    )
    .bind(claim)
    .bind(MAKER_ANSWER)
    .fetch_one(db)
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    /// accepted or declined.
    decision: String,
}

/// The app's maker accepts or declines someone else's delivery.
async fn answer(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, claim)): Path<(Uuid, Uuid)>,
    Json(b): Json<Answer>,
) -> ApiResult<StatusCode> {
    if b.decision != "accepted" && b.decision != "declined" {
        return Err(ApiError::invalid("decision", "accepted or declined"));
    }
    let row: Option<(Uuid, Uuid)> = sqlx::query_as(
        "select a.maker_id, c.claimed_by from bounty_claims c join bounties b on b.id = c.bounty_id
         join apps a on a.id = b.app_id where c.id = $1 and c.bounty_id = $2 and c.status = 'pending'",
    )
    .bind(claim)
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (maker, claimant) = row.ok_or(ApiError::NotFound)?;
    if maker != user.id {
        return Err(ApiError::NotFound);
    }
    if claimant == user.id {
        return Err(ApiError::Conflict(
            "your backers decide your own delivery".into(),
        ));
    }
    decide(&state, id, claim, b.decision == "accepted", "maker").await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Vote {
    accept: bool,
}

/// A backer votes on a claim the backers decide; more than half the pledged amount settles it.
async fn vote(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, claim)): Path<(Uuid, Uuid)>,
    Json(b): Json<Vote>,
) -> ApiResult<StatusCode> {
    let pending: bool =
        sqlx::query_scalar("select exists (select 1 from bounty_claims where id = $1 and bounty_id = $2 and status = 'pending')")
            .bind(claim)
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    if !pending {
        return Err(ApiError::NotFound);
    }
    // COUNTED is a constant: nothing from the request is in the SQL.
    let backer: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select exists (select 1 from bounty_pledges p where p.bounty_id = $1 and p.user_id = $3 and {COUNTED})"
    )))
    .bind(id)
    .bind(claim)
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    if !backer {
        return Err(ApiError::Conflict(
            "only backers who pledged before the delivery vote, and never the one who delivered it or the app's maker".into(),
        ));
    }
    if !backers_decide(&state.db, claim).await? {
        return Err(ApiError::Conflict(
            "the app's maker decides first; backers vote if they don't answer in 14 days".into(),
        ));
    }
    sqlx::query(
        "insert into bounty_votes (claim_id, user_id, accept) values ($1, $2, $3)
         on conflict (claim_id, user_id) do update set accept = $3, created_at = now()",
    )
    .bind(claim)
    .bind(user.id)
    .bind(b.accept)
    .execute(&state.db)
    .await?;
    settle(&state, id, claim).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The pledges that count for a claim's vote (bound: $1 bounty, $2 claim): active, made before
/// the delivery, and not the deliverer's or the app maker's own, so nobody can pledge their way
/// to approving their own delivery (security review, 2026-10-22).
const COUNTED: &str = "p.status = 'active'
    and p.created_at < (select c.created_at from bounty_claims c where c.id = $2)
    and p.user_id <> (select c.claimed_by from bounty_claims c where c.id = $2)
    and p.user_id <> (select a.maker_id from bounties b join apps a on a.id = b.app_id where b.id = $1)";

/// Settles a claim the backers decide, once more than half the counted pledges agree, and either
/// every counted backer has voted or the voting window has passed.
async fn settle(state: &AppState, id: Uuid, claim: Uuid) -> ApiResult<()> {
    let (total, yes, no, unvoted, open_for): (i64, i64, i64, i64, Option<f64>) =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select coalesce(sum(p.amount_minor), 0)::int8,
                    coalesce(sum(p.amount_minor) filter (where v.accept), 0)::int8,
                    coalesce(sum(p.amount_minor) filter (where not v.accept), 0)::int8,
                    count(*) filter (where v.user_id is null),
                    (select extract(epoch from now() - c.created_at)::float8 from bounty_claims c where c.id = $2)
             from bounty_pledges p left join bounty_votes v on v.user_id = p.user_id and v.claim_id = $2
             where p.bounty_id = $1 and {COUNTED}"
        )))
        .bind(id)
        .bind(claim)
        .fetch_one(&state.db)
        .await?;
    let window_over = open_for.unwrap_or(0.0) >= VOTING_WINDOW.as_seconds_f64();
    if total == 0 || (unvoted > 0 && !window_over) {
        return Ok(());
    }
    if yes * 2 > total {
        decide(state, id, claim, true, "backers").await?;
    } else if no * 2 > total {
        decide(state, id, claim, false, "backers").await?;
    }
    Ok(())
}

/// Claims whose voting window has just passed, settled by the charger's loop.
pub async fn settle_due(state: &AppState) -> anyhow::Result<()> {
    let due: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "select c.bounty_id, c.id from bounty_claims c where c.status = 'pending'
           and c.created_at < now() - $1",
    )
    .bind(VOTING_WINDOW)
    .fetch_all(&state.db)
    .await?;
    for (id, claim) in due {
        if backers_decide(&state.db, claim).await? {
            settle(state, id, claim)
                .await
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
    }
    Ok(())
}

async fn withdraw_claim(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, claim)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let done = sqlx::query(
        "update bounty_claims set status = 'withdrawn', decided_at = now()
         where id = $1 and bounty_id = $2 and claimed_by = $3 and status = 'pending'",
    )
    .bind(claim)
    .bind(id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    sqlx::query("update bounties set status = 'open' where id = $1 and status = 'claimed'")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Settles a claim. Accepted: the bounty is paid out (charge_due charges each pledge).
async fn decide(state: &AppState, id: Uuid, claim: Uuid, accept: bool, by: &str) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "update bounty_claims c set status = $3, decided_by = $4, decided_at = now()
         from bounties b where c.id = $1 and c.bounty_id = $2 and c.status = 'pending' and b.id = c.bounty_id
         returning c.claimed_by, b.title",
    )
    .bind(claim)
    .bind(id)
    .bind(if accept { "accepted" } else { "declined" })
    .bind(by)
    .fetch_optional(&mut *tx)
    .await?;
    let (builder, title) = row.ok_or(ApiError::NotFound)?;
    if accept {
        sqlx::query("update bounties set status = 'paying' where id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "update bounty_pledges set status = 'due' where bounty_id = $1 and status = 'active'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        // Pledges still at Checkout never saved a card: they can't be charged.
        sqlx::query("update bounty_pledges set status = 'released' where bounty_id = $1 and status = 'saving'")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("update bounties set status = 'open' where id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    notify::send(
        &mut tx,
        Notice {
            user_id: builder,
            category: "apps",
            title: if accept {
                format!("Your delivery for \"{title}\" was accepted")
            } else {
                format!("Your delivery for \"{title}\" was declined")
            },
            body: if accept {
                "The backers' pledges are being charged and paid to your Stripe account.".into()
            } else {
                "The bounty is open again.".into()
            },
            link: format!("/bounties/{id}"),
            collapse_key: Some(format!("bounty:{id}")),
        },
    )
    .await?;
    tx.commit().await?;
    if accept {
        state.bounty_wakeup.notify_one();
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Charging, and expiry

/// Runs forever: charges due pledges and expires old bounties.
pub async fn run_charger(state: AppState) {
    loop {
        if let Err(err) = settle_due(&state).await {
            tracing::error!(error = ?err, "bounty votes");
        }
        if let Err(err) = charge_due(&state).await {
            tracing::error!(error = ?err, "bounty charger");
        }
        tokio::select! {
            _ = state.bounty_wakeup.notified() => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
        }
    }
}

/// Charges every due pledge once, marks finished bounties paid, and expires old ones. Returns
/// how many pledges it charged.
pub async fn charge_due(state: &AppState) -> anyhow::Result<usize> {
    // One charger at a time, across every running copy of the API (a deploy overlaps the old
    // and new ones briefly): the lock lives with this connection and ends with it.
    let mut lock = state.db.acquire().await?;
    let mine: bool = sqlx::query_scalar("select pg_try_advisory_lock(7341001)")
        .fetch_one(&mut *lock)
        .await?;
    if !mine {
        return Ok(0);
    }
    let result = charge_due_locked(state).await;
    let _ = sqlx::query("select pg_advisory_unlock(7341001)")
        .execute(&mut *lock)
        .await;
    result
}

async fn charge_due_locked(state: &AppState) -> anyhow::Result<usize> {
    // (pledge, bounty, backer, amount, currency, card, builder's account, title, simulated)
    type Due = (
        Uuid,
        Uuid,
        Uuid,
        i32,
        String,
        Option<String>,
        String,
        String,
        bool,
    );
    let due: Vec<Due> = sqlx::query_as(
        "select p.id, p.bounty_id, p.user_id, p.amount_minor, b.currency, p.payment_method_id, c.stripe_account_id,
                b.title, p.simulated
         from bounty_pledges p join bounties b on b.id = p.bounty_id
         join bounty_claims c on c.bounty_id = b.id and c.status = 'accepted'
         where p.status = 'due'
           -- Payouts wait while the delivering app's donations are paused (donation_holds.rs).
           and not exists (select 1 from donation_holds h where h.app_id = c.app_id and h.lifted_at is null)
         limit 100",
    )
    .fetch_all(&state.db)
    .await?;
    let mut charged = 0;
    for (pledge, bounty, backer, amount, currency, method, account, title, simulated) in due {
        let amount = i64::from(amount);
        let slop_fee = fee(amount);
        let result: anyhow::Result<(String, bool)> = match (stripe::mode(state), simulated, &method)
        {
            (_, true, _) | (Mode::Simulated, _, _) => {
                Ok((format!("pi_sim_{}", Uuid::new_v4().simple()), true))
            }
            (Mode::Stripe(stripe), false, Some(pm)) => charge(
                state, stripe, backer, pm, &account, amount, slop_fee, &currency, &title, pledge,
            )
            .await
            .map(|id| (id, false)),
            _ => Err(Refused("no saved card, or payments are off".into()).into()),
        };
        if let Err(e) = &result
            && !refused(e)
        {
            // Stripe or the network failing isn't the backer's card: the pledge stays due and
            // the next run tries again (the idempotency keys stop a retry charging twice).
            tracing::warn!(error = %e, %pledge, "charging a pledge failed; trying again later");
            continue;
        }
        let mut tx = state.db.begin().await?;
        match result {
            Ok((intent, simulated)) => {
                sqlx::query(
                    "update bounty_pledges set status = 'paid', payment_intent_id = $2, charged_at = now() where id = $1",
                )
                .bind(pledge)
                .bind(&intent)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "insert into payments (id, kind, bounty_pledge_id, stripe_account_id, stripe_object_id, payer_id,
                                           amount_minor, slop_fee_minor, currency, simulated)
                     values ($1, 'bounty', $2, $3, $4, $5, $6, $7, $8, $9)",
                )
                .bind(Uuid::now_v7())
                .bind(pledge)
                .bind(&account)
                .bind(&intent)
                .bind(backer)
                .bind(amount as i32)
                .bind(slop_fee as i32)
                .bind(&currency)
                .bind(simulated)
                .execute(&mut *tx)
                .await?;
                charged += 1;
            }
            Err(e) => {
                sqlx::query(
                    "update bounty_pledges set status = 'failed', failure = $2 where id = $1",
                )
                .bind(pledge)
                .bind(e.to_string().chars().take(300).collect::<String>())
                .execute(&mut *tx)
                .await?;
                notify::send(
                    &mut tx,
                    Notice {
                        user_id: backer,
                        category: "donations",
                        title: format!("Your pledge to \"{title}\" couldn't be charged"),
                        body: "Your card was refused, so nothing was paid. The fix was delivered; you can donate to its maker instead.".into(),
                        link: format!("/bounties/{bounty}"),
                        collapse_key: None,
                    },
                )
                .await?;
            }
        }
        // The last pledge settled: the bounty is paid.
        sqlx::query(
            "update bounties set status = 'paid' where id = $1 and status = 'paying'
               and not exists (select 1 from bounty_pledges where bounty_id = $1 and status = 'due')",
        )
        .bind(bounty)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    // Expired without a delivery: release the saved cards.
    let expired: Vec<Option<String>> = sqlx::query_scalar(
        "with ended as (
             update bounties set status = 'expired' where status = 'open' and expires_at < now() returning id)
         update bounty_pledges p set status = 'released' from ended
         where p.bounty_id = ended.id and p.status in ('saving', 'active')
         returning p.payment_method_id",
    )
    .fetch_all(&state.db)
    .await?;
    if let Mode::Stripe(stripe) = stripe::mode(state) {
        for pm in expired.into_iter().flatten() {
            detach(state, stripe, &pm).await;
        }
    }
    Ok(charged)
}

/// One pledge, as a direct charge on the builder's Stripe account: the saved card is cloned
/// there, then charged off-session with Slop's fee as the application fee.
#[allow(clippy::too_many_arguments)]
async fn charge(
    state: &AppState,
    stripe: &StripeConfig,
    backer: Uuid,
    payment_method: &str,
    account: &str,
    amount: i64,
    slop_fee: i64,
    currency: &str,
    title: &str,
    pledge: Uuid,
) -> anyhow::Result<String> {
    let customer: String =
        sqlx::query_scalar("select customer_id from stripe_customers where user_id = $1")
            .bind(backer)
            .fetch_one(&state.db)
            .await?;
    let clone = stripe::call_once(
        state,
        stripe,
        reqwest::Method::POST,
        "v1/payment_methods",
        Some(account),
        &[
            ("customer", customer),
            ("payment_method", payment_method.to_string()),
        ],
        Some(&format!("bounty-pledge-{pledge}-card")),
    )
    .await?;
    let cloned = clone["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Stripe gave no card"))?;
    let intent = stripe::call_once(
        state,
        stripe,
        reqwest::Method::POST,
        "v1/payment_intents",
        Some(account),
        &[
            ("amount", amount.to_string()),
            ("currency", currency.to_string()),
            ("payment_method", cloned.to_string()),
            ("confirm", "true".into()),
            ("off_session", "true".into()),
            ("application_fee_amount", slop_fee.to_string()),
            ("description", format!("Slop bounty: {title}")),
            ("metadata[bounty_pledge_id]", pledge.to_string()),
        ],
        Some(&format!("bounty-pledge-{pledge}")),
    )
    .await?;
    if intent["status"] != "succeeded" {
        return Err(Refused(format!(
            "the payment is {}",
            intent["status"].as_str().unwrap_or("unknown")
        ))
        .into());
    }
    intent["id"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("Stripe gave no payment"))
}

/// A pledge that can't be charged for good: the card was refused, or the payment couldn't
/// complete without the backer.
#[derive(Debug)]
struct Refused(String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// Whether a failed charge is the card's (Stripe refused it: 402 card_declined, or another 4xx
/// that a retry won't change) rather than Stripe or the network failing (5xx, 429, 409, no
/// answer), which the next run retries.
fn refused(e: &anyhow::Error) -> bool {
    if e.downcast_ref::<Refused>().is_some() {
        return true;
    }
    e.downcast_ref::<stripe::StripeError>().is_some_and(|s| {
        s.status.is_client_error()
            && s.status != reqwest::StatusCode::TOO_MANY_REQUESTS
            && s.status != reqwest::StatusCode::CONFLICT
    })
}
