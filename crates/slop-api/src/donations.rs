// SPDX-License-Identifier: AGPL-3.0-or-later
//! Donations and monthly server donations (DESIGN §12; PAYMENTS §4–6, §8; LEGAL §3;
//! LEGAL-GUIDANCE PAY-1 to PAY-17).
//!
//! - **Straight to the maker.** Every donation is a direct charge on the maker's own Stripe
//!   account with `application_fee_amount` = Slop's 5% of the donation (PAY-1). Slop never
//!   holds the money; refunds are the maker's, through Stripe, as the maker (PAY-3).
//! - **The donor pays the card fees**, as one all-in total shown first (PAY-10): the addition
//!   comes from one published rate per currency, the same for every payment method (PAY-11).
//!   Where a country's `donor_covers_fees` is off, the donor's amount is the total and
//!   processing comes out of what the maker receives (PAY-12).
//! - **Nothing in return.** Donors are never shown publicly; their name and a message reach the
//!   maker only if they choose (PAY-6). Nothing that apps can call says who donated (PAY-5, see
//!   the test at the bottom), and no page looks different to a donor.
//! - **Server donations** repeat monthly (Stripe Billing on the maker's account) for apps that
//!   declare servers. The donor confirms the exact terms, which are saved (PAY-14); cancelling
//!   is two clicks and immediate, with a confirmation email (PAY-15); reminders go before the
//!   first renewal and yearly (PAY-16); shutting the servers down cancels them all (PAY-17).
//!
//! - **Shared with the original.** A one-off donation to a community build or mod is shared with
//!   the original app's maker: 30% goes to them as its own direct donation, paid first, then the
//!   rest to the branch's author. Two payments, never a split one (PAYMENTS §5.3); the branch's
//!   payment can't start until a matching share is paid. When the original doesn't take
//!   donations, or the donor made it, it all goes to the author. Server donations aren't shared:
//!   they pay for the branch's own servers.
//!
//! Without Stripe keys, donations are simulated where People payments are (sandbox, development).

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState, admin,
    auth::session::{CurrentUser, MaybeUser},
    catalog,
    config::StripeConfig,
    error::{ApiError, ApiResult},
    notify::{self, Notice},
    people::stripe::{self, Mode, call, fee},
    policy::ViewerPolicy,
};

pub const MIN_MINOR: i64 = 100;
pub const MAX_MINOR: i64 = 50_000;
/// Donations one account may start in an hour (card testing, PAYMENTS §9).
const STARTS_PER_HOUR: i64 = 10;
const MONTH: Duration = Duration::days(30);
/// The original maker's share of a donation to a community build or mod, in percent.
pub const ORIGINAL_SHARE: i64 = 30;
/// How long a paid share can be followed by the branch's donation.
const SHARE_WINDOW: &str = "2 hours";

/// The original's share for a donation of `branch_minor` to the branch, so that the share is
/// 30% of the two together.
pub fn share_for(branch_minor: i64) -> i64 {
    ceil_div(branch_minor * ORIGINAL_SHARE, 100 - ORIGINAL_SHARE)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/donate", get(info).post(start))
        .route("/apps/{slug}/donate/quote", get(quote_route))
        .route("/apps/{slug}/donations", get(summary))
        .route(
            "/apps/{slug}/donations/settings",
            get(settings).put(save_settings),
        )
        .route("/apps/{slug}/servers/shut-down", post(servers_shut_down))
        .route("/me/donation-account", get(my_account).post(onboard))
        .route("/me/donation-account/refresh", post(refresh_account))
        .route("/me/donation-account/change", post(request_change))
        .route("/me/donation-account/change/cancel", post(cancel_change))
        .route("/me/vat", axum::routing::put(set_vat))
        .route("/donations/challenge", post(crate::challenge::issue_route))
        .route("/me/donations", get(my_donations))
        .route("/me/donations/{id}/check", post(check))
        .route("/me/server-donations/{id}/cancel", post(cancel))
        .route("/donations/{id}/refund", post(refund))
        .route("/me/fee-statements", get(fee_statements))
}

// ---------------------------------------------------------------------------------------------
// Amounts

#[derive(Clone, Copy, Debug, sqlx::FromRow)]
pub struct Rate {
    pub percent_bp: i32,
    pub fixed_minor: i32,
}

/// What a donation of `donation` (for the maker, before Slop's fee) costs the donor and gives
/// the maker. With `covers`, the total is chosen so that total − processing(total) = donation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Quote {
    pub donation: i64,
    /// What the donor pays: the first and largest number shown (PAY-10).
    pub total: i64,
    pub slop_fee: i64,
    /// The processing addition (covers), or the estimate that comes out of the maker's share.
    pub processing: i64,
    pub maker_receives: i64,
    pub donor_covers_fees: bool,
}

/// Division rounding up, for positive amounts.
fn ceil_div(a: i64, b: i64) -> i64 {
    (a + b - 1) / b
}

pub fn quote(donation: i64, rate: Rate, covers: bool) -> Quote {
    let bp = i64::from(rate.percent_bp);
    let fixed = i64::from(rate.fixed_minor);
    let slop_fee = fee(donation);
    if covers {
        // Smallest total whose processing estimate leaves at least the donation.
        let total = ceil_div((donation + fixed) * 10_000, 10_000 - bp);
        Quote {
            donation,
            total,
            slop_fee,
            processing: total - donation,
            maker_receives: donation - slop_fee,
            donor_covers_fees: true,
        }
    } else {
        let processing = ceil_div(donation * bp, 10_000) + fixed;
        Quote {
            donation,
            total: donation,
            slop_fee,
            processing,
            maker_receives: donation - slop_fee - processing,
            donor_covers_fees: false,
        }
    }
}

/// Four amounts around the maker's suggestion.
/// The share paid to the original is at least 30% of it and the branch's donation together.
pub fn share_for_minor_ok(share: i64, branch_minor: i64) -> bool {
    share * (100 - ORIGINAL_SHARE) >= branch_minor * ORIGINAL_SHARE
}

pub fn presets(suggested: i64) -> Vec<i64> {
    let round = |v: i64| ((v + 25) / 50 * 50).clamp(MIN_MINOR, MAX_MINOR);
    let mut out = vec![
        round(suggested / 2),
        round(suggested),
        round(suggested * 2),
        round(suggested * 5),
    ];
    out.dedup();
    out
}

pub fn money(minor: i64, currency: &str) -> String {
    let symbol = match currency {
        "eur" => "€",
        "gbp" => "£",
        _ => "$",
    };
    format!("{symbol}{}.{:02}", minor / 100, minor % 100)
}

async fn rate(conn: &mut PgConnection, currency: &str) -> sqlx::Result<Rate> {
    sqlx::query_as("select percent_bp, fixed_minor from processing_rates where currency = $1")
        .bind(currency)
        .fetch_one(conn)
        .await
}

async fn covers(db: &PgPool, user: Option<&CurrentUser>) -> sqlx::Result<bool> {
    Ok(match user {
        Some(u) => sqlx::query_scalar(
            "select donor_covers_fees from country_policies where country_code = $1",
        )
        .bind(&u.country_code)
        .fetch_optional(db)
        .await?
        .unwrap_or(true),
        None => true,
    })
}

/// What the donor confirms for a server donation, saved with it (PAY-14).
pub fn consent_text(
    q: &Quote,
    currency: &str,
    title: &str,
    maker: &str,
    today: time::Date,
) -> String {
    format!(
        "I'm starting a monthly server donation to {title} by {maker}: {total} every month until I cancel \
         ({maker} receives {receives}). The first payment is today, {today}, then the same day each month. \
         I can cancel at any time in two clicks from my library or account page, and no further payments are \
         taken. Donating never changes my access to the app or its servers.",
        total = money(q.total, currency),
        receives = money(q.maker_receives, currency),
    )
}

// ---------------------------------------------------------------------------------------------
// The donate box

struct Target {
    app_id: Uuid,
    slug: String,
    title: String,
    maker_id: Uuid,
    maker_name: String,
    on: bool,
    suggested: i64,
    currency: String,
    runs_servers: bool,
    servers_statement: Option<String>,
    servers_shut_down: bool,
    account: Option<(String, String)>,
    /// For a community build or mod: the original app.
    parent: Option<Uuid>,
    /// Staff have paused donations ('review') or turned them off ('off'): donation_holds.rs.
    held: Option<String>,
}

impl Target {
    /// It can take a donation now: donations on and the maker's Stripe account active.
    fn taking(&self, mode: &Mode) -> bool {
        !matches!(mode, Mode::Off)
            && self.on
            && self.held.is_none()
            && self.account.as_ref().is_some_and(|(_, s)| s == "active")
    }
}

/// The original's donate details, when a donation to this branch is shared with it: its
/// donations are on and the donor isn't its maker.
async fn shared_with(
    state: &AppState,
    t: &Target,
    user: Option<&CurrentUser>,
) -> ApiResult<Option<Target>> {
    let Some(parent) = t.parent else {
        return Ok(None);
    };
    let slug: String = sqlx::query_scalar("select slug from apps where id = $1")
        .bind(parent)
        .fetch_one(&state.db)
        .await?;
    let Ok(p) = target_by_slug(state, &slug).await else {
        return Ok(None);
    };
    let donor_made_it = user.is_some_and(|u| u.id == p.maker_id);
    Ok((p.taking(&stripe::mode(state)) && !donor_made_it).then_some(p))
}

async fn target(
    state: &AppState,
    user: Option<&CurrentUser>,
    headers: &HeaderMap,
    slug: &str,
) -> ApiResult<Target> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user, headers).await?;
    let (app_id, _) = catalog::visible_app(state, slug, user, policy).await?;
    load_target(state, app_id).await
}

/// A public app's donate details by its address, without the viewer's store rules: for the
/// original of a branch the viewer is looking at.
async fn target_by_slug(state: &AppState, slug: &str) -> ApiResult<Target> {
    let id: Uuid = sqlx::query_scalar("select id from apps where slug = $1")
        .bind(slug)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    load_target(state, id).await
}

async fn load_target(state: &AppState, app_id: Uuid) -> ApiResult<Target> {
    #[allow(clippy::type_complexity)]
    let row: (String, String, Uuid, String, bool, i32, String, bool, Option<String>, bool, Option<String>, Option<String>, Option<Uuid>, Option<String>) =
        sqlx::query_as(
            "select a.slug, a.title, a.maker_id, u.display_name, a.donations_on, a.donation_suggested_minor,
                    a.donation_currency, a.runs_servers, a.servers_statement, a.servers_shut_down_at is not null,
                    d.stripe_account_id, d.stripe_status, a.parent_app_id,
                    (select h.kind from donation_holds h where h.app_id = a.id and h.lifted_at is null)
             from apps a join users u on u.id = a.maker_id left join donation_accounts d on d.user_id = a.maker_id
             where a.id = $1 and a.visibility = 'public'",
        )
        .bind(app_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Target {
        app_id,
        slug: row.0,
        title: row.1,
        maker_id: row.2,
        maker_name: row.3,
        on: row.4,
        suggested: i64::from(row.5),
        currency: row.6,
        runs_servers: row.7,
        servers_statement: row.8,
        servers_shut_down: row.9,
        account: row.10.zip(row.11),
        parent: row.12,
        held: row.13,
    })
}

#[derive(Serialize)]
struct Info {
    /// Donations are open for this app.
    on: bool,
    /// Why not, in words, when they aren't.
    off_reason: Option<&'static str>,
    currency: String,
    presets: Vec<Quote>,
    min_minor: i64,
    max_minor: i64,
    donor_covers_fees: bool,
    /// Server donations, when the app declares servers.
    servers: Option<Servers>,
    maker: String,
    /// What stops this viewer donating (sign in, their own app, …).
    cant: Option<&'static str>,
    simulated: bool,
    /// For a community build or mod: the original's share of one-off donations.
    shared: Option<SharedInfo>,
}

#[derive(Serialize)]
struct SharedInfo {
    slug: String,
    title: String,
    maker: String,
    percent: i64,
}

#[derive(Serialize)]
struct Servers {
    statement: String,
    /// The terms the donor confirms, for the suggested amount (the quote has them per amount).
    example_consent: String,
}

async fn info(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Info>> {
    let t = target(&state, user.as_ref(), &headers, &slug).await?;
    let mode = stripe::mode(&state);
    let off_reason = if matches!(mode, Mode::Off) {
        Some("Donations aren't switched on on this server yet.")
    } else if t.held.is_some() {
        Some("Donations to this app are paused.")
    } else if !t.on {
        Some("The maker hasn't turned on donations.")
    } else if t.account.as_ref().is_none_or(|(_, s)| s != "active") {
        Some("The maker's Stripe account isn't ready to receive donations yet.")
    } else {
        None
    };
    let covers = covers(&state.db, user.as_ref()).await?;
    let mut conn = state.db.acquire().await?;
    let r = rate(&mut conn, &t.currency).await?;
    let cant = match &user {
        None => Some("Sign in to donate."),
        Some(u) if u.id == t.maker_id => Some("It's your own app."),
        Some(_) => None,
    };
    let suggested_quote = quote(t.suggested, r, covers);
    let shared = shared_with(&state, &t, user.as_ref())
        .await?
        .map(|p| SharedInfo {
            slug: p.slug,
            title: p.title,
            maker: p.maker_name,
            percent: ORIGINAL_SHARE,
        });
    Ok(Json(Info {
        shared,
        on: off_reason.is_none(),
        off_reason,
        presets: presets(t.suggested)
            .into_iter()
            .map(|d| quote(d, r, covers))
            .collect(),
        min_minor: MIN_MINOR,
        max_minor: MAX_MINOR,
        donor_covers_fees: covers,
        servers: (t.runs_servers && !t.servers_shut_down).then(|| Servers {
            statement: t.servers_statement.clone().unwrap_or_default(),
            example_consent: consent_text(
                &suggested_quote,
                &t.currency,
                &t.title,
                &t.maker_name,
                OffsetDateTime::now_utc().date(),
            ),
        }),
        maker: t.maker_name,
        currency: t.currency,
        cant,
        simulated: matches!(mode, Mode::Simulated),
    }))
}

#[derive(Deserialize)]
struct QuoteQuery {
    amount_minor: i64,
}

#[derive(Serialize)]
struct QuoteWithTerms {
    #[serde(flatten)]
    quote: Quote,
    currency: String,
    /// The server-donation terms for this amount, when the app has servers.
    consent: Option<String>,
}

async fn quote_route(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(q): Query<QuoteQuery>,
) -> ApiResult<Json<QuoteWithTerms>> {
    if !(MIN_MINOR..=MAX_MINOR).contains(&q.amount_minor) {
        return Err(ApiError::invalid("amount_minor", "from 1 to 500"));
    }
    let t = target(&state, user.as_ref(), &headers, &slug).await?;
    let covers = covers(&state.db, user.as_ref()).await?;
    let mut conn = state.db.acquire().await?;
    let qt = quote(q.amount_minor, rate(&mut conn, &t.currency).await?, covers);
    Ok(Json(QuoteWithTerms {
        consent: (t.runs_servers && !t.servers_shut_down).then(|| {
            consent_text(
                &qt,
                &t.currency,
                &t.title,
                &t.maker_name,
                OffsetDateTime::now_utc().date(),
            )
        }),
        quote: qt,
        currency: t.currency,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartBody {
    amount_minor: i64,
    #[serde(default)]
    monthly: bool,
    /// PAY-6: share the donor's name (and message) with the maker. Off unless ticked.
    #[serde(default)]
    share_name: bool,
    message: Option<String>,
    /// The server-donation terms, confirmed with an unticked box (PAY-13, PAY-14).
    #[serde(default)]
    consent: bool,
    #[serde(default)]
    confirm_adult: bool,
    /// Needed after a few unfinished checkouts (challenge.rs).
    challenge: Option<crate::challenge::Answer>,
    /// This is the original's share of a donation to that branch (its address); `rest_minor` is
    /// what the donor will give the branch next.
    share_for: Option<String>,
    rest_minor: Option<i64>,
    /// A donation to a branch: the paid share to the original it follows.
    share_donation: Option<Uuid>,
}

/// Unfinished checkouts in a day after which starting another needs a proof-of-work check.
const UNFINISHED_BEFORE_CHALLENGE: i64 = 3;

#[derive(Serialize)]
struct Started {
    /// Stripe Checkout; none when the payment was simulated (it's done).
    url: Option<String>,
    id: Uuid,
    done: bool,
}

async fn require_adult(state: &AppState, user: &CurrentUser, confirm: bool) -> ApiResult<()> {
    let confirmed: bool =
        sqlx::query_scalar("select adult_confirmed_at is not null from users where id = $1")
            .bind(user.id)
            .fetch_one(&state.db)
            .await?;
    if confirmed {
        return Ok(());
    }
    if !confirm {
        return Err(ApiError::invalid(
            "confirm_adult",
            "donating is for adults: confirm you're 18 or older",
        ));
    }
    sqlx::query("update users set adult_confirmed_at = now() where id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(())
}

async fn start(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(b): Json<StartBody>,
) -> ApiResult<Json<Started>> {
    crate::limits::check(&state, user.id, crate::limits::Action::Donation)?;
    let t = target(&state, Some(&user), &headers, &slug).await?;
    if user.id == t.maker_id {
        return Err(ApiError::Conflict(
            "you can't donate to your own app".into(),
        ));
    }
    let mode = stripe::mode(&state);
    let account = match (&mode, &t.account) {
        (Mode::Off, _) => {
            return Err(ApiError::Unavailable(
                "donations aren't switched on on this server",
            ));
        }
        (_, Some((acct, status))) if t.on && t.held.is_none() && status == "active" => acct.clone(),
        _ => return Err(ApiError::Conflict("this app isn't taking donations".into())),
    };
    if !(MIN_MINOR..=MAX_MINOR).contains(&b.amount_minor) {
        return Err(ApiError::invalid("amount_minor", "from 1 to 500"));
    }
    require_adult(&state, &user, b.confirm_adult).await?;
    // The original's share, paid first, for a donation to one of its branches.
    let share_for: Option<(Uuid, String, i64)> = match (&b.share_for, b.monthly) {
        (Some(_), true) => {
            return Err(ApiError::invalid(
                "share_for",
                "server donations aren't shared",
            ));
        }
        (Some(branch_slug), false) => {
            let branch: Option<(Uuid, Option<Uuid>)> =
                sqlx::query_as("select id, parent_app_id from apps where slug = $1")
                    .bind(branch_slug)
                    .fetch_optional(&state.db)
                    .await?;
            let Some((branch_id, Some(parent))) = branch else {
                return Err(ApiError::invalid(
                    "share_for",
                    "that isn't a community build or mod",
                ));
            };
            if parent != t.app_id {
                return Err(ApiError::invalid(
                    "share_for",
                    "that isn't one of this app's branches",
                ));
            }
            let rest = b
                .rest_minor
                .filter(|r| (MIN_MINOR..=MAX_MINOR).contains(r))
                .ok_or_else(|| {
                    ApiError::invalid("rest_minor", "what you'll give the branch, from 1 to 500")
                })?;
            Some((branch_id, branch_slug.clone(), rest))
        }
        (None, _) => None,
    };
    // A donation to a branch follows a paid share to the original, unless there's none to pay.
    let share_donation = if !b.monthly && shared_with(&state, &t, Some(&user)).await?.is_some() {
        let id = b.share_donation.ok_or_else(|| {
            ApiError::invalid(
                "share_donation",
                format!("{ORIGINAL_SHARE}% of a donation to a community build or mod goes to the original's maker first"),
            )
        })?;
        let share: Option<i32> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "select s.donation_minor from donations s
             where s.id = $1 and s.donor_id = $2 and s.share_for = $3 and s.status = 'paid'
               and s.created_at > now() - interval '{SHARE_WINDOW}'
               and not exists (select 1 from donations x where x.share_donation_id = s.id)"
        )))
        .bind(id)
        .bind(user.id)
        .bind(t.app_id)
        .fetch_optional(&state.db)
        .await?;
        let Some(share) = share else {
            return Err(ApiError::invalid(
                "share_donation",
                "the share for the original isn't paid yet (or was already used)",
            ));
        };
        if share_for_minor_ok(i64::from(share), b.amount_minor) {
            Some(id)
        } else {
            return Err(ApiError::invalid(
                "amount_minor",
                format!(
                    "the original's share must be at least {ORIGINAL_SHARE}% of the two together"
                ),
            ));
        }
    } else {
        None
    };
    let allowed: bool =
        sqlx::query_scalar("select payments_allowed from country_policies where country_code = $1")
            .bind(&user.country_code)
            .fetch_optional(&state.db)
            .await?
            .unwrap_or(false);
    if !allowed {
        return Err(ApiError::Conflict(
            "donations aren't available in your country yet".into(),
        ));
    }
    let recent: i64 = sqlx::query_scalar(
        "select (select count(*) from donations where donor_id = $1 and kind = 'one_off' and created_at > now() - interval '1 hour')
              + (select count(*) from server_donations where donor_id = $1 and created_at > now() - interval '1 hour')",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    if recent >= STARTS_PER_HOUR {
        return Err(ApiError::Conflict(
            "that's a lot of donations in an hour; try again later".into(),
        ));
    }
    let unfinished: i64 = sqlx::query_scalar(
        "select (select count(*) from donations where donor_id = $1 and kind = 'one_off'
                   and status in ('pending', 'expired') and created_at > now() - interval '1 day')
              + (select count(*) from server_donations where donor_id = $1 and started_at is null
                   and created_at > now() - interval '1 day')",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    if unfinished >= UNFINISHED_BEFORE_CHALLENGE {
        crate::challenge::check(&state.db, user.id, b.challenge.as_ref()).await?;
    }
    let message = b
        .message
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty() && b.share_name)
        .map(String::from);
    if message.as_ref().is_some_and(|m| m.chars().count() > 500) {
        return Err(ApiError::invalid("message", "500 characters at most"));
    }
    let covers = covers(&state.db, Some(&user)).await?;
    let mut tx = state.db.begin().await?;
    let q = quote(b.amount_minor, rate(&mut tx, &t.currency).await?, covers);
    let id = Uuid::now_v7();

    if b.monthly {
        if !t.runs_servers || t.servers_shut_down {
            return Err(ApiError::Conflict(
                "this app doesn't take server donations".into(),
            ));
        }
        if !b.consent {
            return Err(ApiError::invalid(
                "consent",
                "confirm the monthly terms: it repeats every month until you cancel",
            ));
        }
        let consent = consent_text(
            &q,
            &t.currency,
            &t.title,
            &t.maker_name,
            OffsetDateTime::now_utc().date(),
        );
        sqlx::query(
            "insert into server_donations (id, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                           slop_fee_minor, total_minor, consent_text, simulated)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(id)
        .bind(t.app_id)
        .bind(user.id)
        .bind(&account)
        .bind(&t.currency)
        .bind(q.donation as i32)
        .bind(q.slop_fee as i32)
        .bind(q.total as i32)
        .bind(&consent)
        .bind(matches!(mode, Mode::Simulated))
        .execute(&mut *tx)
        .await
        .map_err(|err| {
            if crate::error::is_unique_violation(&err) {
                ApiError::Conflict(
                    "you already have a server donation for this app; change it from your account page".into(),
                )
            } else {
                err.into()
            }
        })?;
    } else {
        sqlx::query(
            "insert into donations (id, kind, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                    slop_fee_minor, processing_minor, total_minor, donor_covers_fees, share_name,
                                    message, simulated, share_for, share_donation_id)
             values ($1, 'one_off', $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(id)
        .bind(t.app_id)
        .bind(user.id)
        .bind(&account)
        .bind(&t.currency)
        .bind(q.donation as i32)
        .bind(q.slop_fee as i32)
        .bind(q.processing as i32)
        .bind(q.total as i32)
        .bind(covers)
        .bind(b.share_name)
        .bind(&message)
        .bind(matches!(mode, Mode::Simulated))
        .bind(share_for.as_ref().map(|s| s.0))
        .bind(share_donation)
        .execute(&mut *tx)
        .await
        .map_err(|err| {
            if crate::error::is_unique_violation(&err) {
                ApiError::invalid("share_donation", "that share was already used")
            } else {
                err.into()
            }
        })?;
    }

    match mode {
        Mode::Simulated => {
            let now = OffsetDateTime::now_utc();
            if b.monthly {
                activate(&mut tx, id, None, now).await?;
                monthly_paid(
                    &mut tx,
                    id,
                    &format!("sim_inv_{}", Uuid::new_v4().simple()),
                    now,
                )
                .await?;
            } else {
                donation_paid(
                    &mut tx,
                    id,
                    &format!("sim_ch_{}", Uuid::new_v4().simple()),
                    now,
                )
                .await?;
            }
            tx.commit().await?;
            Ok(Json(Started {
                url: None,
                id,
                done: true,
            }))
        }
        Mode::Stripe(stripe) => {
            tx.commit().await?;
            // After the original's share, back to the branch's page for the second payment.
            let back = share_for
                .as_ref()
                .map(|(_, slug, rest)| format!("app/{slug}?shared={id}&rest={rest}"));
            let url = checkout(
                &state, stripe, &t, &account, id, &q, b.monthly, user.id, back,
            )
            .await?;
            Ok(Json(Started {
                url: Some(url),
                id,
                done: false,
            }))
        }
        Mode::Off => unreachable!("checked above"),
    }
}

#[allow(clippy::too_many_arguments)]
async fn checkout(
    state: &AppState,
    stripe: &StripeConfig,
    t: &Target,
    account: &str,
    id: Uuid,
    q: &Quote,
    monthly: bool,
    donor: Uuid,
    back: Option<String>,
) -> anyhow::Result<String> {
    let base = &state.config.public_url;
    let success = match back {
        Some(path) => format!("{base}{path}"),
        None => format!("{base}app/{}?donated={id}", t.slug),
    };
    let cancel = format!("{base}app/{}", t.slug);
    // PAY-7: Stripe sees "donation", never "purchase" or "order".
    let mut form: Vec<(&str, String)> = vec![
        ("line_items[0][quantity]", "1".into()),
        ("line_items[0][price_data][currency]", t.currency.clone()),
        (
            "line_items[0][price_data][unit_amount]",
            q.total.to_string(),
        ),
        ("success_url", success),
        ("cancel_url", cancel),
        ("client_reference_id", id.to_string()),
        ("metadata[donor]", donor.to_string()),
    ];
    if monthly {
        // The fee is 5% of the donation, as a share of the total charged each month.
        let percent = (q.slop_fee as f64 * 100.0 / q.total as f64 * 100.0).round() / 100.0;
        form.extend([
            ("mode", "subscription".into()),
            (
                "line_items[0][price_data][recurring][interval]",
                "month".into(),
            ),
            (
                "line_items[0][price_data][product_data][name]",
                format!("Monthly server donation to {}", t.title),
            ),
            (
                "subscription_data[application_fee_percent]",
                format!("{percent:.2}"),
            ),
            (
                "subscription_data[description]",
                format!("Monthly server donation to {}", t.title),
            ),
            (
                "subscription_data[metadata][server_donation_id]",
                id.to_string(),
            ),
            ("metadata[server_donation_id]", id.to_string()),
        ]);
    } else {
        form.extend([
            ("mode", "payment".into()),
            (
                "line_items[0][price_data][product_data][name]",
                format!("Donation to {}", t.title),
            ),
            (
                "payment_intent_data[application_fee_amount]",
                q.slop_fee.to_string(),
            ),
            (
                "payment_intent_data[description]",
                format!("Donation to {}", t.title),
            ),
            ("payment_intent_data[metadata][donation_id]", id.to_string()),
            ("metadata[donation_id]", id.to_string()),
        ]);
    }
    let session = call(
        state,
        stripe,
        reqwest::Method::POST,
        "v1/checkout/sessions",
        Some(account),
        &form,
    )
    .await?;
    let sid = session["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Stripe gave no session"))?;
    let table = if monthly {
        "server_donations"
    } else {
        "donations"
    };
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "update {table} set checkout_session_id = $2 where id = $1"
    )))
    .bind(id)
    .bind(sid)
    .execute(&state.db)
    .await?;
    session["url"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("Stripe gave no checkout link"))
}

// ---------------------------------------------------------------------------------------------
// Recording payments (from Stripe's API, or simulated)

/// Marks a one-off donation paid and records the payment. Idempotent.
async fn donation_paid(
    tx: &mut PgConnection,
    id: Uuid,
    object_id: &str,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    #[allow(clippy::type_complexity)] // one row of the query below
    let row: Option<(Uuid, Uuid, String, String, i32, i32, bool, bool, String, String)> = sqlx::query_as(
        "update donations d set status = 'paid', paid_at = $3, stripe_object_id = $2
         from apps a where a.id = d.app_id and d.id = $1 and d.status = 'pending'
         returning d.donor_id, a.maker_id, d.stripe_account_id, d.currency, d.total_minor, d.slop_fee_minor,
                   d.simulated, d.share_name, a.title, a.slug",
    )
    .bind(id)
    .bind(object_id)
    .bind(at)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((donor, maker, account, currency, total, fee_minor, simulated, shared, title, slug)) =
        row
    else {
        return Ok(());
    };
    record_payment(
        tx, id, donor, &account, object_id, total, fee_minor, &currency, simulated, at,
    )
    .await?;
    thank(tx, id, donor, maker, &title, &slug, shared).await
}

#[allow(clippy::too_many_arguments)]
async fn record_payment(
    tx: &mut PgConnection,
    donation: Uuid,
    payer: Uuid,
    account: &str,
    object_id: &str,
    total: i32,
    fee_minor: i32,
    currency: &str,
    simulated: bool,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query(
        "insert into payments (id, kind, donation_id, stripe_account_id, stripe_object_id, payer_id, amount_minor,
                               slop_fee_minor, currency, simulated, created_at)
         values ($1, 'donation', $2, $3, $4, $5, $6, $7, $8, $9, $10) on conflict (stripe_object_id) do nothing",
    )
    .bind(Uuid::now_v7())
    .bind(donation)
    .bind(account)
    .bind(object_id)
    .bind(payer)
    .bind(total)
    .bind(fee_minor)
    .bind(currency)
    .bind(simulated)
    .bind(at)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The donor's thank-you and receipt link; the maker hears about new donations (merged while
/// unread), by name only when the donor shared it.
async fn thank(
    tx: &mut PgConnection,
    donation: Uuid,
    donor: Uuid,
    maker: Uuid,
    title: &str,
    slug: &str,
    shared: bool,
) -> sqlx::Result<()> {
    let (amount, currency, name): (i32, String, String) = sqlx::query_as(
        "select d.donation_minor, d.currency, u.display_name from donations d join users u on u.id = d.donor_id
         where d.id = $1",
    )
    .bind(donation)
    .fetch_one(&mut *tx)
    .await?;
    notify::send(
        tx,
        Notice {
            user_id: donor,
            category: "donations",
            title: format!("Thank you for donating to {title}"),
            body: format!(
                "{} goes to the maker. Your receipt is on your account page.",
                money(i64::from(amount), &currency)
            ),
            link: "/account#donations".into(),
            collapse_key: None,
        },
    )
    .await?;
    notify::send(
        tx,
        Notice {
            user_id: maker,
            category: "donations",
            title: if shared {
                format!("{name} donated to {title}")
            } else {
                format!("New donations to {title}")
            },
            body: String::new(),
            link: format!("/publish/{slug}/manage#donations"),
            collapse_key: (!shared).then(|| format!("donations:{slug}")),
        },
    )
    .await
}

/// A server donation's subscription is running.
async fn activate(
    tx: &mut PgConnection,
    id: Uuid,
    subscription: Option<&str>,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query(
        "update server_donations set status = 'active', started_at = coalesce(started_at, $3),
                stripe_subscription_id = coalesce(stripe_subscription_id, $2),
                next_charge_at = coalesce(next_charge_at, $3 + $4)
         where id = $1 and status = 'pending'",
    )
    .bind(id)
    .bind(subscription)
    .bind(at)
    .bind(MONTH)
    .execute(tx)
    .await?;
    Ok(())
}

/// One month of a server donation was paid: its own donation row. Idempotent on the invoice.
async fn monthly_paid(
    tx: &mut PgConnection,
    server_donation: Uuid,
    object_id: &str,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    #[allow(clippy::type_complexity)] // one row of the query below
    let row: Option<(Uuid, Uuid, Uuid, String, String, i32, i32, i32, bool, String, String)> = sqlx::query_as(
        "select s.app_id, s.donor_id, a.maker_id, s.stripe_account_id, s.currency, s.donation_minor,
                s.slop_fee_minor, s.total_minor, s.simulated, a.title, a.slug
         from server_donations s join apps a on a.id = s.app_id where s.id = $1",
    )
    .bind(server_donation)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((
        app,
        donor,
        maker,
        account,
        currency,
        donation,
        fee_minor,
        total,
        simulated,
        title,
        slug,
    )) = row
    else {
        return Ok(());
    };
    let id = Uuid::now_v7();
    let inserted = sqlx::query(
        "insert into donations (id, kind, app_id, donor_id, server_donation_id, stripe_account_id, currency,
                                donation_minor, slop_fee_minor, processing_minor, total_minor, donor_covers_fees,
                                status, stripe_object_id, simulated, created_at, paid_at)
         values ($1, 'monthly', $2, $3, $4, $5, $6, $7, $8, $9 - $7, $9, $9 > $7, 'paid', $10, $11, $12, $12)
         on conflict (stripe_object_id) do nothing",
    )
    .bind(id)
    .bind(app)
    .bind(donor)
    .bind(server_donation)
    .bind(&account)
    .bind(&currency)
    .bind(donation)
    .bind(fee_minor)
    .bind(total)
    .bind(object_id)
    .bind(simulated)
    .bind(at)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(());
    }
    record_payment(
        tx, id, donor, &account, object_id, total, fee_minor, &currency, simulated, at,
    )
    .await?;
    thank(tx, id, donor, maker, &title, &slug, false).await
}

/// Reads a finished Checkout session for a donation or server donation (webhook or the
/// donor's "check" after coming back).
pub async fn sync_checkout(
    state: &AppState,
    stripe: &StripeConfig,
    acct: &str,
    session_id: &str,
) -> anyhow::Result<()> {
    let s = call(
        state,
        stripe,
        reqwest::Method::GET,
        &format!("v1/checkout/sessions/{session_id}"),
        Some(acct),
        &[("expand[]", "payment_intent".into())],
    )
    .await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = state.db.begin().await?;
    if let Some(id) = s["metadata"]["donation_id"]
        .as_str()
        .and_then(|v| v.parse::<Uuid>().ok())
    {
        if s["payment_status"] == "paid" {
            let charge = s["payment_intent"]["latest_charge"]
                .as_str()
                .unwrap_or(session_id);
            donation_paid(&mut tx, id, charge, now).await?;
        } else if s["status"] == "expired" {
            sqlx::query(
                "update donations set status = 'expired' where id = $1 and status = 'pending'",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
    } else if let Some(id) = s["metadata"]["server_donation_id"]
        .as_str()
        .and_then(|v| v.parse::<Uuid>().ok())
        && s["status"] == "complete"
    {
        activate(&mut tx, id, s["subscription"].as_str(), now).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// A monthly payment (`invoice.paid`) on a server donation's subscription.
pub async fn sync_invoice(
    state: &AppState,
    stripe: &StripeConfig,
    acct: &str,
    invoice_id: &str,
) -> anyhow::Result<()> {
    let inv = call(
        state,
        stripe,
        reqwest::Method::GET,
        &format!("v1/invoices/{invoice_id}"),
        Some(acct),
        &[],
    )
    .await?;
    if inv["status"] != "paid" {
        return Ok(());
    }
    // The subscription and its metadata (older and newer Stripe API shapes).
    let subscription = inv["subscription"]
        .as_str()
        .or_else(|| inv["parent"]["subscription_details"]["subscription"].as_str());
    let meta = if inv["subscription_details"]["metadata"].is_object() {
        &inv["subscription_details"]["metadata"]
    } else {
        &inv["parent"]["subscription_details"]["metadata"]
    };
    let by_meta = meta["server_donation_id"]
        .as_str()
        .and_then(|v| v.parse::<Uuid>().ok());
    let id: Option<Uuid> = match by_meta {
        Some(id) => Some(id),
        None => {
            sqlx::query_scalar("select id from server_donations where stripe_subscription_id = $1")
                .bind(subscription)
                .fetch_optional(&state.db)
                .await?
        }
    };
    let Some(id) = id else {
        return Ok(());
    };
    let paid_at = inv["status_transitions"]["paid_at"]
        .as_i64()
        .and_then(|t| OffsetDateTime::from_unix_timestamp(t).ok())
        .unwrap_or_else(OffsetDateTime::now_utc);
    let mut tx = state.db.begin().await?;
    activate(&mut tx, id, subscription, paid_at).await?;
    monthly_paid(&mut tx, id, invoice_id, paid_at).await?;
    sqlx::query("update server_donations set next_charge_at = greatest(next_charge_at, $2 + $3) where id = $1")
        .bind(id)
        .bind(paid_at)
        .bind(MONTH)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// A subscription ended on Stripe's side (cancelled there, or payments failed).
pub async fn subscription_ended(
    state: &AppState,
    stripe: &StripeConfig,
    acct: &str,
    sub_id: &str,
) -> anyhow::Result<()> {
    let sub = call(
        state,
        stripe,
        reqwest::Method::GET,
        &format!("v1/subscriptions/{sub_id}"),
        Some(acct),
        &[],
    )
    .await?;
    if sub["status"] != "canceled" {
        return Ok(());
    }
    sqlx::query(
        "update server_donations set status = 'cancelled', cancelled_at = now(), cancelled_by = 'stripe'
         where stripe_subscription_id = $1 and status <> 'cancelled'",
    )
    .bind(sub_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// A refund on a donation's charge (the maker's, from Slop's button or their Stripe dashboard).
pub async fn refund_recorded(db: &PgPool, charge: &str, refunded_total: i64) -> anyhow::Result<()> {
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, i32, String, String)> = sqlx::query_as(
        "select id, total_minor, currency, stripe_account_id from donations where stripe_object_id = $1 for update",
    )
    .bind(charge)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, total, currency, account)) = row else {
        return Ok(());
    };
    sqlx::query(
        "update donations set refunded_minor = $2,
                status = case when $2 >= total_minor then 'refunded' else 'partially_refunded' end
         where id = $1",
    )
    .bind(id)
    .bind(refunded_total as i32)
    .execute(&mut *tx)
    .await?;
    let original: Option<Uuid> = sqlx::query_scalar(
        "select id from payments where stripe_object_id = $1 and kind = 'donation'",
    )
    .bind(charge)
    .fetch_optional(&mut *tx)
    .await?;
    sqlx::query(
        "insert into payments (id, kind, refund_of, donation_id, stripe_account_id, stripe_object_id, amount_minor, currency)
         values ($1, 'refund', $2, $3, $4, $5, $6, $7) on conflict (stripe_object_id) do nothing",
    )
    .bind(Uuid::now_v7())
    .bind(original)
    .bind(id)
    .bind(account)
    .bind(format!("{charge}:refunded:{refunded_total}"))
    .bind(-(refunded_total.min(i64::from(total)) as i32))
    .bind(currency)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The donor's side

#[derive(Serialize, sqlx::FromRow)]
struct DonationRow {
    id: Uuid,
    kind: String,
    app_title: String,
    app_slug: String,
    currency: String,
    donation_minor: i32,
    total_minor: i32,
    status: String,
    refunded_minor: i32,
    share_name: bool,
    simulated: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    paid_at: Option<OffsetDateTime>,
}

#[derive(Serialize, sqlx::FromRow)]
struct ServerDonationRow {
    id: Uuid,
    app_title: String,
    app_slug: String,
    currency: String,
    donation_minor: i32,
    total_minor: i32,
    status: String,
    consent_text: String,
    simulated: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    next_charge_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    cancelled_at: Option<OffsetDateTime>,
    cancelled_by: Option<String>,
}

#[derive(Serialize)]
struct Mine {
    donations: Vec<DonationRow>,
    server_donations: Vec<ServerDonationRow>,
}

async fn my_donations(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Mine>> {
    Ok(Json(Mine {
        donations: sqlx::query_as(
            "select d.id, d.kind, a.title as app_title, a.slug as app_slug, d.currency, d.donation_minor, d.total_minor,
                    d.status, d.refunded_minor, d.share_name, d.simulated, d.created_at, d.paid_at
             from donations d join apps a on a.id = d.app_id
             where d.donor_id = $1 and d.status <> 'expired' order by d.created_at desc limit 200",
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await?,
        server_donations: sqlx::query_as(
            "select s.id, a.title as app_title, a.slug as app_slug, s.currency, s.donation_minor, s.total_minor, s.status,
                    s.consent_text, s.simulated, s.started_at, s.next_charge_at, s.cancelled_at, s.cancelled_by
             from server_donations s join apps a on a.id = s.app_id
             where s.donor_id = $1 and (s.status <> 'pending' or s.created_at > now() - interval '1 day')
             order by s.created_at desc",
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await?,
    }))
}

/// After coming back from Stripe: reads the session now rather than waiting for the webhook.
async fn check(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "select stripe_account_id, checkout_session_id from donations where id = $1 and donor_id = $2
         union all
         select stripe_account_id, checkout_session_id from server_donations where id = $1 and donor_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    let (acct, session) = row.ok_or(ApiError::NotFound)?;
    if let (Mode::Stripe(stripe), Some(session)) = (stripe::mode(&state), session) {
        sync_checkout(&state, stripe, &acct, &session).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Cancels a server donation: immediately, no reason asked (PAY-15).
async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let row: Option<(String, Option<String>, bool)> = sqlx::query_as(
        "select stripe_account_id, stripe_subscription_id, simulated from server_donations
         where id = $1 and donor_id = $2 and status in ('pending', 'active')",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    let (acct, sub, simulated) = row.ok_or(ApiError::NotFound)?;
    if let (Mode::Stripe(stripe), Some(sub), false) = (stripe::mode(&state), &sub, simulated) {
        call(
            &state,
            stripe,
            reqwest::Method::DELETE,
            &format!("v1/subscriptions/{sub}"),
            Some(&acct),
            &[],
        )
        .await?;
    }
    let mut tx = state.db.begin().await?;
    end_server_donations(&mut tx, &state.config, Some(id), None, "donor").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Marks server donations cancelled (one, or every one for an app) and confirms it to each donor,
/// by email whatever their email choices (PAY-15, PAY-17).
async fn end_server_donations(
    tx: &mut PgConnection,
    config: &crate::config::Config,
    id: Option<Uuid>,
    app: Option<Uuid>,
    by: &str,
) -> sqlx::Result<usize> {
    let ended: Vec<(Uuid, String, String, i32)> = sqlx::query_as(
        "update server_donations s set status = 'cancelled', cancelled_at = now(), cancelled_by = $3
         from apps a where a.id = s.app_id and s.status in ('pending', 'active')
           and (s.id = $1 or s.app_id = $2)
         returning s.donor_id, a.title, s.currency, s.total_minor",
    )
    .bind(id)
    .bind(app)
    .bind(by)
    .fetch_all(&mut *tx)
    .await?;
    for (donor, title, currency, total) in &ended {
        let (title_text, body) = if by == "servers_shut_down" {
            (
                format!("Your server donation to {title} has ended"),
                format!(
                    "The maker shut down {title}'s servers, so your monthly server donation of {} has been cancelled. No further payments will be taken.",
                    money(i64::from(*total), currency)
                ),
            )
        } else if by == "maker_disconnected" {
            (
                format!("Your server donation to {title} has ended"),
                format!(
                    "{title}'s maker stopped taking donations, so your monthly server donation of {} has been cancelled. No further payments will be taken.",
                    money(i64::from(*total), currency)
                ),
            )
        } else {
            (
                format!("Your server donation to {title} is cancelled"),
                format!(
                    "You cancelled your monthly server donation of {} to {title}. No further payments will be taken.",
                    money(i64::from(*total), currency)
                ),
            )
        };
        notify::send(
            tx,
            Notice {
                user_id: *donor,
                category: "donations",
                title: title_text.clone(),
                body: body.clone(),
                link: "/account#donations".into(),
                collapse_key: None,
            },
        )
        .await?;
        notify::email_user(tx, config, *donor, &title_text, &body).await?;
    }
    Ok(ended.len())
}

// ---------------------------------------------------------------------------------------------
// The maker's side

pub(crate) async fn my_app(
    state: &AppState,
    user: &CurrentUser,
    slug: &str,
) -> ApiResult<(Uuid, String)> {
    let row: Option<(Uuid, Uuid, String)> =
        sqlx::query_as("select id, maker_id, title from apps where slug = $1")
            .bind(slug)
            .fetch_optional(&state.db)
            .await?;
    match row {
        Some((id, maker, title)) if maker == user.id => Ok((id, title)),
        _ => Err(ApiError::NotFound),
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct Settings {
    donations_on: bool,
    donation_suggested_minor: i32,
    donation_currency: String,
    runs_servers: bool,
    servers_statement: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    servers_shut_down_at: Option<OffsetDateTime>,
    /// The maker's Stripe account: none, pending, active or restricted.
    account_status: String,
    active_server_donations: i64,
    /// Whether this server takes payments: stripe, simulated or off.
    #[sqlx(skip)]
    payments: &'static str,
}

async fn load_settings(state: &AppState, app_id: Uuid) -> ApiResult<Settings> {
    let mut s: Settings = sqlx::query_as(
        "select a.donations_on, a.donation_suggested_minor, a.donation_currency, a.runs_servers, a.servers_statement,
                a.servers_shut_down_at, coalesce(d.stripe_status, 'none') as account_status,
                (select count(*) from server_donations s where s.app_id = a.id and s.status = 'active') as active_server_donations
         from apps a left join donation_accounts d on d.user_id = a.maker_id where a.id = $1",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    s.payments = stripe::mode(state).name();
    Ok(s)
}

async fn settings(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Settings>> {
    let (app_id, _) = my_app(&state, &user, &slug).await?;
    Ok(Json(load_settings(&state, app_id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsBody {
    donations_on: bool,
    donation_suggested_minor: i32,
    donation_currency: String,
    runs_servers: bool,
    servers_statement: Option<String>,
}

async fn save_settings(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<SettingsBody>,
) -> ApiResult<Json<Settings>> {
    let (app_id, _) = my_app(&state, &user, &slug).await?;
    let current = load_settings(&state, app_id).await?;
    if b.donations_on && current.account_status != "active" {
        return Err(ApiError::Conflict(
            "connect your Stripe account first".into(),
        ));
    }
    if !(MIN_MINOR as i32..=MAX_MINOR as i32).contains(&b.donation_suggested_minor) {
        return Err(ApiError::invalid(
            "donation_suggested_minor",
            "from 1 to 500",
        ));
    }
    if !["usd", "eur", "gbp"].contains(&b.donation_currency.as_str()) {
        return Err(ApiError::invalid("donation_currency", "usd, eur or gbp"));
    }
    let statement = b
        .servers_statement
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if b.runs_servers && !(10..=300).contains(&statement.chars().count()) {
        return Err(ApiError::invalid(
            "servers_statement",
            "say what the servers are, and what donations help pay for (10 to 300 characters)",
        ));
    }
    if !b.runs_servers && current.active_server_donations > 0 {
        return Err(ApiError::Conflict(format!(
            "{} people give monthly for the servers: use \"The servers are shut down\" to end their donations first",
            current.active_server_donations
        )));
    }
    sqlx::query(
        "update apps set donations_on = $2, donation_suggested_minor = $3, donation_currency = $4, runs_servers = $5,
                servers_statement = nullif($6, ''),
                servers_shut_down_at = case when $5 then null else servers_shut_down_at end
         where id = $1",
    )
    .bind(app_id)
    .bind(b.donations_on)
    .bind(b.donation_suggested_minor)
    .bind(&b.donation_currency)
    .bind(b.runs_servers)
    .bind(&statement)
    .execute(&state.db)
    .await?;
    Ok(Json(load_settings(&state, app_id).await?))
}

/// PAY-17: the servers are gone, so every server donation to the app is cancelled and its donor told.
async fn servers_shut_down(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let (app_id, _) = my_app(&state, &user, &slug).await?;
    let subs: Vec<(String, Option<String>, bool)> = sqlx::query_as(
        "select stripe_account_id, stripe_subscription_id, simulated from server_donations
         where app_id = $1 and status in ('pending', 'active')",
    )
    .bind(app_id)
    .fetch_all(&state.db)
    .await?;
    if let Mode::Stripe(stripe) = stripe::mode(&state) {
        for (acct, sub, simulated) in &subs {
            if let (Some(sub), false) = (sub, simulated) {
                call(
                    &state,
                    stripe,
                    reqwest::Method::DELETE,
                    &format!("v1/subscriptions/{sub}"),
                    Some(acct),
                    &[],
                )
                .await?;
            }
        }
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("update apps set runs_servers = false, servers_shut_down_at = now() where id = $1")
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    let ended = end_server_donations(
        &mut tx,
        &state.config,
        None,
        Some(app_id),
        "servers_shut_down",
    )
    .await?;
    admin::audit(
        &mut tx,
        &user.display_name,
        "donations.servers_shut_down",
        &slug,
        json!({ "cancelled": ended }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "cancelled": ended })))
}

#[derive(Serialize)]
struct Summary {
    currency_totals: Vec<Total>,
    /// Months with at least 10 donations; quieter months are left out so no donor stands out.
    months: Vec<Month>,
    /// Shown only from 10 up, for the same reason.
    active_server_donations: Option<i64>,
    /// Donors who chose to share their name, with their message.
    shared: Vec<Shared>,
    /// Recent donations for refunding a mistake: amount and date only.
    recent: Vec<Recent>,
}

#[derive(Serialize, sqlx::FromRow)]
struct Total {
    currency: String,
    donations: i64,
    donated_minor: i64,
    slop_fee_minor: i64,
    refunded_minor: i64,
}

#[derive(Serialize, sqlx::FromRow)]
struct Month {
    month: String,
    currency: String,
    donations: i64,
    donated_minor: i64,
}

#[derive(Serialize, sqlx::FromRow)]
struct Shared {
    name: String,
    message: Option<String>,
    currency: String,
    donation_minor: i32,
    #[serde(with = "time::serde::rfc3339")]
    paid_at: OffsetDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
struct Recent {
    id: Uuid,
    kind: String,
    currency: String,
    total_minor: i32,
    status: String,
    refundable: bool,
    #[serde(with = "time::serde::rfc3339")]
    paid_at: OffsetDateTime,
}

/// Minimum group before a breakdown is shown (METRICS §3).
const MIN_GROUP: i64 = 10;

async fn summary(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Summary>> {
    let (app_id, _) = my_app(&state, &user, &slug).await?;
    let active: i64 = sqlx::query_scalar(
        "select count(*) from server_donations where app_id = $1 and status = 'active'",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(Summary {
        currency_totals: sqlx::query_as(
            "select currency, count(*) as donations, sum(donation_minor)::int8 as donated_minor,
                    sum(slop_fee_minor)::int8 as slop_fee_minor, sum(refunded_minor)::int8 as refunded_minor
             from donations where app_id = $1 and status <> 'pending' and status <> 'expired' group by currency",
        )
        .bind(app_id)
        .fetch_all(&state.db)
        .await?,
        months: sqlx::query_as(
            "select to_char(date_trunc('month', paid_at), 'YYYY-MM') as month, currency, count(*) as donations,
                    sum(donation_minor)::int8 as donated_minor
             from donations where app_id = $1 and paid_at is not null
             group by 1, 2 having count(*) >= $2 order by 1 desc limit 24",
        )
        .bind(app_id)
        .bind(MIN_GROUP)
        .fetch_all(&state.db)
        .await?,
        active_server_donations: (active >= MIN_GROUP).then_some(active),
        shared: sqlx::query_as(
            "select u.display_name as name, d.message, d.currency, d.donation_minor, d.paid_at
             from donations d join users u on u.id = d.donor_id
             where d.app_id = $1 and d.share_name and d.paid_at is not null order by d.paid_at desc limit 100",
        )
        .bind(app_id)
        .fetch_all(&state.db)
        .await?,
        recent: sqlx::query_as(
            "select id, kind, currency, total_minor, status, (kind = 'one_off' and status = 'paid') as refundable, paid_at
             from donations where app_id = $1 and paid_at > now() - interval '90 days' order by paid_at desc limit 100",
        )
        .bind(app_id)
        .fetch_all(&state.db)
        .await?,
    }))
}

/// The maker refunds a donation (a mistyped amount): through Stripe, on their own account, as
/// them (PAY-3).
async fn refund(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let row: Option<(String, Option<String>, i32, bool)> = sqlx::query_as(
        "select d.stripe_account_id, d.stripe_object_id, d.total_minor, d.simulated from donations d
         join apps a on a.id = d.app_id
         where d.id = $1 and a.maker_id = $2 and d.kind = 'one_off' and d.status = 'paid'",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    let (acct, charge, total, simulated) = row.ok_or(ApiError::NotFound)?;
    let charge = charge.ok_or(ApiError::NotFound)?;
    if !simulated && let Mode::Stripe(stripe) = stripe::mode(&state) {
        call(
            &state,
            stripe,
            reqwest::Method::POST,
            "v1/refunds",
            Some(&acct),
            &[("charge", charge.clone())],
        )
        .await?;
    }
    refund_recorded(&state.db, &charge, i64::from(total)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct Account {
    status: String,
    payments: &'static str,
    /// A switch or disconnection waiting out its 72 hours.
    change: Option<AccountChange>,
}

#[derive(Serialize, sqlx::FromRow)]
struct AccountChange {
    kind: String,
    #[serde(with = "time::serde::rfc3339")]
    requested_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    effective_at: OffsetDateTime,
    /// Switch: whether the new Stripe account has finished Stripe's sign-up. The switch happens
    /// at `effective_at` or once it has, whichever is later.
    new_account_ready: Option<bool>,
}

async fn open_change(db: &PgPool, user_id: Uuid) -> sqlx::Result<Option<AccountChange>> {
    sqlx::query_as(
        "select kind, requested_at, effective_at,
                case when new_account_id is not null then new_account_status = 'active' end as new_account_ready
         from stripe_account_changes
         where user_id = $1 and cancelled_at is null and applied_at is null",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

async fn my_account(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Account>> {
    let status: Option<String> =
        sqlx::query_scalar("select stripe_status from donation_accounts where user_id = $1")
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?;
    Ok(Json(Account {
        status: status.unwrap_or_else(|| "none".into()),
        payments: stripe::mode(&state).name(),
        change: open_change(&state.db, user.id).await?,
    }))
}

/// How long a change to where donations are paid waits (SECURITY §9).
pub const CHANGE_DELAY: Duration = Duration::hours(72);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeBody {
    /// switch or disconnect
    kind: String,
    return_to: Option<String>,
}

/// Switching or disconnecting the Stripe account that receives donations (SECURITY §9,
/// PAYMENTS §9): a fresh sign-in, then 72 hours' wait, announced by email at once so the real
/// owner can cancel it if it wasn't them. A switch sets up the new account through Stripe's
/// sign-up meanwhile.
async fn request_change(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<ChangeBody>,
) -> ApiResult<Json<Onboarding>> {
    if !["switch", "disconnect"].contains(&b.kind.as_str()) {
        return Err(ApiError::invalid("kind", "switch or disconnect"));
    }
    crate::passkeys::require(&state, &user).await?;
    let current: Option<String> =
        sqlx::query_scalar("select stripe_account_id from donation_accounts where user_id = $1")
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?;
    if current.is_none() {
        return Err(ApiError::Conflict(
            "there's no donations account to change: enable donations first".into(),
        ));
    }
    crate::auth::session::require_recent_sign_in(&state.db, &user).await?;
    let return_to = b
        .return_to
        .filter(|p| p.starts_with('/') && !p.starts_with("//") && p.len() <= 200)
        .unwrap_or_else(|| "/account".into());
    let mode = stripe::mode(&state);
    let new_account = match (b.kind.as_str(), &mode) {
        ("disconnect", _) => None,
        (_, Mode::Off) => {
            return Err(ApiError::Unavailable(
                "payments aren't switched on yet on this server",
            ));
        }
        (_, Mode::Simulated) => Some(format!("acct_sim_{}", Uuid::new_v4().simple())),
        (_, Mode::Stripe(stripe)) => {
            let created = call(
                &state,
                stripe,
                reqwest::Method::POST,
                "v1/accounts",
                None,
                &[
                    ("controller[stripe_dashboard][type]", "full".into()),
                    ("controller[fees][payer]", "account".into()),
                    ("controller[losses][payments]", "stripe".into()),
                    ("controller[requirement_collection]", "stripe".into()),
                    ("metadata[slop_user]", user.id.to_string()),
                ],
            )
            .await?;
            Some(
                created["id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Stripe gave no account ID"))?
                    .to_string(),
            )
        }
    };
    let mut tx = state.db.begin().await?;
    let inserted = sqlx::query(
        "insert into stripe_account_changes (id, user_id, kind, new_account_id, new_account_status, effective_at)
         values ($1, $2, $3, $4, $5, now() + $6)",
    )
    .bind(Uuid::now_v7())
    .bind(user.id)
    .bind(&b.kind)
    .bind(&new_account)
    .bind(if matches!(mode, Mode::Simulated) {
        "active"
    } else {
        "pending"
    })
    .bind(CHANGE_DELAY)
    .execute(&mut *tx)
    .await;
    if let Err(err) = inserted {
        if crate::error::is_unique_violation(&err) {
            return Err(ApiError::Conflict(
                "a change is already waiting; cancel it first".into(),
            ));
        }
        return Err(err.into());
    }
    let (title, body) = if b.kind == "switch" {
        (
            "Your donations will be paid to a different Stripe account".to_string(),
            "Someone signed in to your Slop account and asked to pay your donations into a new \
             Stripe account. If it was you, finish the new account's set-up; the switch happens in \
             72 hours. If it wasn't you, cancel it on your account page now and sign out your \
             other sessions."
                .to_string(),
        )
    } else {
        (
            "Donations to your apps will stop in 72 hours".to_string(),
            "Someone signed in to your Slop account and asked to disconnect your Stripe account. \
             In 72 hours donations to your apps turn off and monthly server donations end. If it \
             wasn't you, cancel it on your account page now and sign out your other sessions."
                .to_string(),
        )
    };
    notify::send(
        &mut tx,
        Notice {
            user_id: user.id,
            category: "donations",
            title: title.clone(),
            body: body.clone(),
            link: "/account#donations".into(),
            collapse_key: None,
        },
    )
    .await?;
    notify::email_user(&mut tx, &state.config, user.id, &title, &body).await?;
    admin::audit(
        &mut tx,
        &user.display_name,
        "donations.account_change_requested",
        &user.id.to_string(),
        json!({ "kind": b.kind }),
    )
    .await?;
    tx.commit().await?;

    let url = match (&new_account, &mode) {
        (Some(acct), Mode::Stripe(stripe)) => {
            let base = &state.config.public_url;
            let back = format!("{base}{}", return_to.trim_start_matches('/'));
            let link = call(
                &state,
                stripe,
                reqwest::Method::POST,
                "v1/account_links",
                None,
                &[
                    ("account", acct.clone()),
                    ("type", "account_onboarding".into()),
                    ("refresh_url", back.clone()),
                    ("return_url", back),
                ],
            )
            .await?;
            link["url"].as_str().map(String::from)
        }
        _ => None,
    };
    Ok(Json(Onboarding {
        url,
        status: "waiting".into(),
    }))
}

/// Cancelling is the safe direction, so it needs no fresh sign-in.
async fn cancel_change(State(state): State<AppState>, user: CurrentUser) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let done = sqlx::query(
        "update stripe_account_changes set cancelled_at = now()
         where user_id = $1 and cancelled_at is null and applied_at is null",
    )
    .bind(user.id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if done == 0 {
        return Err(ApiError::NotFound);
    }
    admin::audit(
        &mut tx,
        &user.display_name,
        "donations.account_change_cancelled",
        &user.id.to_string(),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Hourly: changes whose 72 hours are up. A switch waits on until the new account has finished
/// Stripe's sign-up.
pub async fn apply_account_changes(state: &AppState) -> anyhow::Result<usize> {
    if let Mode::Stripe(stripe) = stripe::mode(state) {
        // In case a webhook about the new account went missing.
        let waiting: Vec<String> = sqlx::query_scalar(
            "select new_account_id from stripe_account_changes
             where cancelled_at is null and applied_at is null and effective_at <= now()
               and new_account_id is not null and new_account_status <> 'active'",
        )
        .fetch_all(&state.db)
        .await?;
        for acct in &waiting {
            stripe::sync_account(state, stripe, acct).await?;
        }
    }
    let due: Vec<(Uuid, Uuid, String, Option<String>)> = sqlx::query_as(
        "select c.id, c.user_id, c.kind, c.new_account_id from stripe_account_changes c
         where c.cancelled_at is null and c.applied_at is null and c.effective_at <= now()
           and (c.kind = 'disconnect' or c.new_account_status = 'active')",
    )
    .fetch_all(&state.db)
    .await?;
    for (id, user_id, kind, new_account) in &due {
        if kind == "disconnect" {
            // Stop the subscriptions at Stripe first; they're on the maker's account.
            let subs: Vec<(String, Option<String>, bool)> = sqlx::query_as(
                "select s.stripe_account_id, s.stripe_subscription_id, s.simulated from server_donations s
                 join apps a on a.id = s.app_id
                 where a.maker_id = $1 and s.status in ('pending', 'active')",
            )
            .bind(user_id)
            .fetch_all(&state.db)
            .await?;
            if let Mode::Stripe(stripe) = stripe::mode(state) {
                for (acct, sub, simulated) in &subs {
                    if let (Some(sub), false) = (sub, simulated) {
                        call(
                            state,
                            stripe,
                            reqwest::Method::DELETE,
                            &format!("v1/subscriptions/{sub}"),
                            Some(acct),
                            &[],
                        )
                        .await?;
                    }
                }
            }
        }
        let mut tx = state.db.begin().await?;
        let (title, body) = if kind == "disconnect" {
            let apps: Vec<Uuid> = sqlx::query_scalar(
                "update apps set donations_on = false where maker_id = $1 and donations_on returning id",
            )
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
            let all: Vec<Uuid> = sqlx::query_scalar("select id from apps where maker_id = $1")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
            for app in &all {
                end_server_donations(
                    &mut tx,
                    &state.config,
                    None,
                    Some(*app),
                    "maker_disconnected",
                )
                .await?;
            }
            sqlx::query("delete from donation_accounts where user_id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            (
                "Your Stripe account is disconnected".to_string(),
                format!(
                    "Donations are off on your {} app{} and monthly server donations have ended. \
                     Your donation records stay on your account page.",
                    apps.len(),
                    if apps.len() == 1 { "" } else { "s" }
                ),
            )
        } else {
            sqlx::query(
                "update donation_accounts set stripe_account_id = $2, stripe_status = 'active', updated_at = now()
                 where user_id = $1",
            )
            .bind(user_id)
            .bind(new_account)
            .execute(&mut *tx)
            .await?;
            (
                "Donations now go to your new Stripe account".to_string(),
                "New donations are paid into your new Stripe account. Monthly server donations \
                 that already started stay on the old one until they end."
                    .to_string(),
            )
        };
        sqlx::query("update stripe_account_changes set applied_at = now() where id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        notify::send(
            &mut tx,
            Notice {
                user_id: *user_id,
                category: "donations",
                title: title.clone(),
                body: body.clone(),
                link: "/account#donations".into(),
                collapse_key: None,
            },
        )
        .await?;
        notify::email_user(&mut tx, &state.config, *user_id, &title, &body).await?;
        admin::audit(
            &mut tx,
            "slop",
            "donations.account_change_applied",
            &user_id.to_string(),
            json!({ "kind": kind }),
        )
        .await?;
        tx.commit().await?;
    }
    Ok(due.len())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OnboardBody {
    tax_notice_ack: bool,
    #[serde(default)]
    confirm_adult: bool,
    /// Where to come back to after Stripe (a path in Slop).
    return_to: Option<String>,
}

#[derive(Serialize)]
struct Onboarding {
    url: Option<String>,
    status: String,
}

/// "Enable donations": the tax notice (PAY-9), then Stripe's sign-up on the maker's own account.
async fn onboard(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<OnboardBody>,
) -> ApiResult<Json<Onboarding>> {
    if !b.tax_notice_ack {
        return Err(ApiError::invalid(
            "tax_notice_ack",
            "confirm you've read: donations may be taxable income for you, and Slop doesn't give tax advice",
        ));
    }
    if !user_is_adult(&state, &user, b.confirm_adult).await? {
        return Err(ApiError::invalid(
            "confirm_adult",
            "receiving donations is for adults: confirm you're 18 or older",
        ));
    }
    crate::passkeys::require(&state, &user).await?;
    let return_to = b
        .return_to
        .filter(|p| p.starts_with('/') && !p.starts_with("//") && p.len() <= 200)
        .unwrap_or_else(|| "/publish".into());
    let existing: Option<String> = sqlx::query_scalar(
        "select coalesce((select stripe_account_id from donation_accounts where user_id = $1),
                         (select stripe_account_id from creatives where user_id = $1))",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    // Connecting a Stripe account to receive money is a step-up action (SECURITY §4.3).
    // Finishing the sign-up of one already connected isn't.
    if existing.is_none() {
        crate::auth::session::require_recent_sign_in(&state.db, &user).await?;
    }
    let save = |acct: String, status: &'static str| {
        let db = state.db.clone();
        let user_id = user.id;
        async move {
            sqlx::query(
                "insert into donation_accounts (user_id, stripe_account_id, stripe_status, tax_notice_ack_at)
                 values ($1, $2, $3, now())
                 on conflict (user_id) do update set tax_notice_ack_at = now(), updated_at = now()",
            )
            .bind(user_id)
            .bind(acct)
            .bind(status)
            .execute(&db)
            .await
        }
    };
    match stripe::mode(&state) {
        Mode::Off => Err(ApiError::Unavailable(
            "payments aren't switched on yet on this server",
        )),
        Mode::Simulated => {
            save(
                existing.unwrap_or_else(|| format!("acct_sim_{}", Uuid::new_v4().simple())),
                "active",
            )
            .await?;
            sqlx::query("update donation_accounts set stripe_status = 'active' where user_id = $1")
                .bind(user.id)
                .execute(&state.db)
                .await?;
            Ok(Json(Onboarding {
                url: None,
                status: "active".into(),
            }))
        }
        Mode::Stripe(stripe) => {
            let acct = match existing {
                Some(a) => a,
                None => {
                    let created = call(
                        &state,
                        stripe,
                        reqwest::Method::POST,
                        "v1/accounts",
                        None,
                        &[
                            ("controller[stripe_dashboard][type]", "full".into()),
                            ("controller[fees][payer]", "account".into()),
                            ("controller[losses][payments]", "stripe".into()),
                            ("controller[requirement_collection]", "stripe".into()),
                            ("metadata[slop_user]", user.id.to_string()),
                        ],
                    )
                    .await?;
                    created["id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Stripe gave no account ID"))?
                        .to_string()
                }
            };
            save(acct.clone(), "pending").await?;
            let status = stripe::sync_account(&state, stripe, &acct).await?;
            if status == "active" {
                return Ok(Json(Onboarding { url: None, status }));
            }
            let base = &state.config.public_url;
            let link = call(
                &state,
                stripe,
                reqwest::Method::POST,
                "v1/account_links",
                None,
                &[
                    ("account", acct),
                    ("type", "account_onboarding".into()),
                    (
                        "refresh_url",
                        format!("{base}{}", return_to.trim_start_matches('/')),
                    ),
                    (
                        "return_url",
                        format!("{base}{}", return_to.trim_start_matches('/')),
                    ),
                ],
            )
            .await?;
            Ok(Json(Onboarding {
                url: link["url"].as_str().map(String::from),
                status,
            }))
        }
    }
}

async fn user_is_adult(state: &AppState, user: &CurrentUser, confirm: bool) -> ApiResult<bool> {
    match require_adult(state, user, confirm).await {
        Ok(()) => Ok(true),
        Err(ApiError::Invalid { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

async fn refresh_account(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Onboarding>> {
    let row: Option<(String, String)> = sqlx::query_as(
        "select stripe_account_id, stripe_status from donation_accounts where user_id = $1",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    let (acct, status) = row.ok_or(ApiError::NotFound)?;
    let status = match stripe::mode(&state) {
        Mode::Stripe(stripe) => stripe::sync_account(&state, stripe, &acct).await?,
        _ => status,
    };
    Ok(Json(Onboarding { url: None, status }))
}

#[derive(Serialize, sqlx::FromRow)]
struct Statement {
    month: String,
    currency: String,
    /// donation or people_job
    kind: String,
    payments: i64,
    received_minor: i64,
    slop_fee_minor: i64,
    simulated: bool,
    /// The VAT included in Slop's fee (LEGAL §3.5); zero under the reverse charge.
    #[sqlx(skip)]
    vat_minor: i64,
}

#[derive(Serialize)]
struct Statements {
    statements: Vec<Statement>,
    vat: VatTreatment,
}

/// How VAT applies to Slop's fee for this user (LEGAL §3.5): the 5% always includes it.
#[derive(Serialize)]
struct VatTreatment {
    /// none (no VAT where they are), included (Slop pays it out of its 5%), or reverse_charge
    /// (a VAT-registered business: they account for it).
    kind: &'static str,
    /// The country's standard rate, in basis points.
    rate_bp: Option<i32>,
    country: String,
    vat_number: Option<String>,
    /// Whether the VAT number was confirmed with the EU's VIES service.
    vat_number_checked: bool,
    /// Slop's own VAT registration, once it has one (SLOP_VAT_ID).
    slop_vat_id: Option<String>,
}

/// The VAT inside a VAT-inclusive amount: amount × rate / (1 + rate), to the nearest cent.
pub fn vat_included(amount_minor: i64, rate_bp: i64) -> i64 {
    (amount_minor * rate_bp * 2 + (10_000 + rate_bp)) / (2 * (10_000 + rate_bp))
}

async fn vat_treatment(state: &AppState, user: &CurrentUser) -> sqlx::Result<VatTreatment> {
    let (rate, number, checked): (Option<i32>, Option<String>, bool) = sqlx::query_as(
        "select c.vat_rate_bp, u.vat_number, u.vat_checked_at is not null
         from users u join country_policies c on c.country_code = u.country_code where u.id = $1",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    let key = state.config.field_key.as_ref();
    let number = match number {
        Some(n) => Some(
            crate::fields::open(key, "users.vat_number", &n)
                .map_err(|e| sqlx::Error::Decode(e.into()))?,
        ),
        None => None,
    };
    Ok(VatTreatment {
        kind: match (rate, checked) {
            (None, _) => "none",
            (Some(_), true) => "reverse_charge",
            (Some(_), false) => "included",
        },
        rate_bp: rate,
        country: user.country_code.clone(),
        vat_number: number,
        vat_number_checked: checked,
        slop_vat_id: state.config.payments.vat_id.clone(),
    })
}

/// Monthly fee statements (PAYMENTS §8, TAX-4): what Slop's fee came to on the user's donations
/// and People jobs, and the VAT included in it.
async fn fee_statements(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Statements>> {
    let vat = vat_treatment(&state, &user).await?;
    let mut statements: Vec<Statement> = sqlx::query_as(
            "select to_char(date_trunc('month', p.created_at), 'YYYY-MM') as month, p.currency, p.kind,
                    count(*) as payments, sum(p.amount_minor)::int8 as received_minor,
                    sum(p.slop_fee_minor)::int8 as slop_fee_minor, bool_or(p.simulated) as simulated
             from payments p
             where p.kind in ('donation', 'people_job') and p.stripe_account_id in (
                 select stripe_account_id from donation_accounts where user_id = $1
                 union select stripe_account_id from creatives where user_id = $1 and stripe_account_id is not null)
             group by 1, 2, 3 order by 1 desc, 3",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    if let ("included", Some(rate)) = (vat.kind, vat.rate_bp) {
        for s in &mut statements {
            s.vat_minor = vat_included(s.slop_fee_minor, i64::from(rate));
        }
    }
    Ok(Json(Statements { statements, vat }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VatBody {
    /// With its country prefix (DE…, FR…, GB…); empty or null to remove it.
    vat_number: Option<String>,
}

#[derive(Serialize)]
struct VatSaved {
    vat_number: Option<String>,
    checked: bool,
    note: Option<String>,
}

/// A maker's or creative's VAT number, for the reverse charge on Slop's fee. EU numbers are
/// confirmed with the EU's VIES service; until a number is confirmed, statements show the VAT as
/// included in the fee.
async fn set_vat(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<VatBody>,
) -> ApiResult<Json<VatSaved>> {
    let number = b
        .vat_number
        .as_deref()
        .map(|n| {
            n.chars()
                .filter(|c| !c.is_whitespace() && *c != '.' && *c != '-')
                .collect::<String>()
                .to_ascii_uppercase()
        })
        .filter(|n| !n.is_empty());
    let Some(number) = number else {
        sqlx::query("update users set vat_number = null, vat_checked_at = null where id = $1")
            .bind(user.id)
            .execute(&state.db)
            .await?;
        return Ok(Json(VatSaved {
            vat_number: None,
            checked: false,
            note: None,
        }));
    };
    let valid_shape = number.len() >= 4
        && number.len() <= 15
        && number[..2].chars().all(|c| c.is_ascii_uppercase())
        && number[2..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '*');
    if !valid_shape {
        return Err(ApiError::invalid(
            "vat_number",
            "a VAT number starts with its country code, like DE123456789",
        ));
    }
    // Greece's VAT numbers start with EL.
    let country = match &number[..2] {
        "EL" => "GR",
        c => c,
    };
    if country != user.country_code {
        return Err(ApiError::invalid(
            "vat_number",
            "the VAT number has to be from the country on your account",
        ));
    }
    let checked = match country {
        "GB" => None,
        _ => vies_check(&state, &number).await,
    };
    sqlx::query(
        "update users set vat_number = $2, vat_checked_at = case when $3 then now() end where id = $1",
    )
    .bind(user.id)
    .bind(
        crate::fields::seal(state.config.field_key.as_ref(), "users.vat_number", &number)
            .map_err(ApiError::Internal)?,
    )
    .bind(checked == Some(true))
    .execute(&state.db)
    .await?;
    let note = match checked {
        Some(true) => None,
        Some(false) => Some(
            "The EU's VIES service says this number isn't valid for cross-border trade, so VAT \
             stays included in Slop's fee. Check it, or ask your tax office to enable it for \
             trade within the EU."
                .into(),
        ),
        None if country == "GB" => Some(
            "Saved. Slop can't check UK VAT numbers automatically yet, so VAT stays included in \
             Slop's fee until it's confirmed."
                .into(),
        ),
        None => Some(
            "Saved, but the EU's VIES service didn't answer. Slop will check it again later; \
             until then VAT stays included in Slop's fee."
                .into(),
        ),
    };
    Ok(Json(VatSaved {
        vat_number: Some(number),
        checked: checked == Some(true),
        note,
    }))
}

/// Asks the EU's VIES service whether a VAT number is valid. None when it couldn't say.
async fn vies_check(state: &AppState, number: &str) -> Option<bool> {
    let url = state.config.payments.vies_url.as_ref()?;
    #[derive(Deserialize)]
    struct Answer {
        valid: bool,
    }
    let res = state
        .http
        .post(url.clone())
        .json(&json!({ "countryCode": &number[..2], "vatNumber": &number[2..] }))
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json::<Answer>().await.ok().map(|a| a.valid)
}

// ---------------------------------------------------------------------------------------------
// Hourly and daily

/// Hourly: reminders before the first renewal and yearly (PAY-16), and simulated renewals.
pub async fn hourly(state: &AppState) -> anyhow::Result<()> {
    let mut tx = state.db.begin().await?;
    let due: Vec<(Uuid, Uuid, String, String, i32, OffsetDateTime, bool)> = sqlx::query_as(
        "select s.id, s.donor_id, a.title, s.currency, s.total_minor, s.next_charge_at, s.first_reminder_at is null
         from server_donations s join apps a on a.id = s.app_id
         where s.status = 'active' and s.next_charge_at < now() + interval '7 days'
           and (s.first_reminder_at is null
                or (s.started_at < now() - interval '358 days'
                    and coalesce(s.last_yearly_reminder_at, s.started_at) < now() - interval '358 days'))",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (id, donor, title, currency, total, next, first) in &due {
        let subject = format!("Your server donation to {title} renews on {}", next.date());
        let body = format!(
            "Your monthly server donation of {} to {title} is taken again on {}. Nothing to do if you want it to continue. \
             To stop it, cancel from your account page: it takes two clicks and no further payments are taken.",
            money(i64::from(*total), currency),
            next.date()
        );
        notify::send(
            &mut tx,
            Notice {
                user_id: *donor,
                category: "donations",
                title: subject.clone(),
                body: body.clone(),
                link: "/account#donations".into(),
                collapse_key: None,
            },
        )
        .await?;
        notify::email_user(&mut tx, &state.config, *donor, &subject, &body).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update server_donations set {} = now() where id = $1",
            if *first {
                "first_reminder_at"
            } else {
                "last_yearly_reminder_at"
            }
        )))
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    // Simulated subscriptions renew here; real ones by Stripe's invoices.
    let renew: Vec<(Uuid, OffsetDateTime)> = sqlx::query_as(
        "select s.id, s.next_charge_at from server_donations s
         where s.simulated and s.status = 'active' and s.next_charge_at <= now()
           and not exists (select 1 from donation_holds h where h.app_id = s.app_id and h.lifted_at is null)",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (id, at) in renew {
        monthly_paid(
            &mut tx,
            id,
            &format!("sim_inv_{}", Uuid::new_v4().simple()),
            at,
        )
        .await?;
        sqlx::query(
            "update server_donations set next_charge_at = next_charge_at + $2 where id = $1",
        )
        .bind(id)
        .bind(MONTH)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Per Stripe account: (Slop's fees Stripe says it collected, the fees in Slop's records).
pub fn mismatches(stripe: &[(String, i64)], ours: &[(String, i64)]) -> Vec<(String, i64, i64)> {
    use std::collections::BTreeMap;
    let mut by: BTreeMap<&str, (i64, i64)> = BTreeMap::new();
    for (acct, fee) in stripe {
        by.entry(acct).or_default().0 += fee;
    }
    for (acct, fee) in ours {
        by.entry(acct).or_default().1 += fee;
    }
    by.into_iter()
        .filter(|(_, (a, b))| a != b)
        .map(|(acct, (a, b))| (acct.to_string(), a, b))
        .collect()
}

/// Daily (PAY-2): Slop's Stripe balance holds only its own fees, and every fee matches a payment
/// Slop recorded. Compares the last full day's application fees per account with the records.
pub async fn reconcile(state: &AppState) -> anyhow::Result<usize> {
    let Mode::Stripe(stripe) = stripe::mode(state) else {
        return Ok(0);
    };
    let today = OffsetDateTime::now_utc().replace_time(time::Time::MIDNIGHT);
    let (from, to) = (today - Duration::days(1), today);
    let mut fees = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut form = vec![
            ("created[gte]", from.unix_timestamp().to_string()),
            ("created[lt]", to.unix_timestamp().to_string()),
            ("limit", "100".to_string()),
        ];
        if let Some(a) = &after {
            form.push(("starting_after", a.clone()));
        }
        let page = call(
            state,
            stripe,
            reqwest::Method::GET,
            "v1/application_fees",
            None,
            &form,
        )
        .await?;
        for f in page["data"].as_array().into_iter().flatten() {
            let acct = f["account"].as_str().unwrap_or_default().to_string();
            let net = f["amount"].as_i64().unwrap_or_default()
                - f["amount_refunded"].as_i64().unwrap_or_default();
            fees.push((acct, net));
            after = f["id"].as_str().map(String::from);
        }
        if page["has_more"] != true {
            break;
        }
    }
    let ours: Vec<(String, i64)> = sqlx::query_as(
        "select stripe_account_id, slop_fee_minor::int8 from payments
         where not simulated and kind in ('people_job', 'donation') and created_at >= $1 and created_at < $2
           and stripe_account_id is not null",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    let bad = mismatches(&fees, &ours);
    if !bad.is_empty() {
        tracing::error!(
            ?bad,
            "Stripe application fees don't match Slop's payment records"
        );
        let mut tx = state.db.begin().await?;
        admin::audit(
            &mut tx,
            "reconciliation",
            "payments.mismatch",
            &from.date().to_string(),
            json!({ "accounts": bad }),
        )
        .await?;
        tx.commit().await?;
    }
    Ok(bad.len())
}

/// Runs the hourly jobs and the daily reconciliation.
pub async fn run(state: AppState) {
    let mut last_day = None;
    loop {
        if let Err(err) = hourly(&state).await {
            tracing::error!(error = ?err, "donation reminders and renewals failed");
        }
        if let Err(err) = apply_account_changes(&state).await {
            tracing::error!(error = ?err, "donation account changes failed");
        }
        let day = OffsetDateTime::now_utc().date();
        if last_day != Some(day) {
            match reconcile(&state).await {
                Ok(_) => last_day = Some(day),
                Err(err) => tracing::error!(error = ?err, "payment reconciliation failed"),
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USD: Rate = Rate {
        percent_bp: 290,
        fixed_minor: 30,
    };

    #[test]
    fn the_donor_covers_processing_and_the_maker_gets_the_donation_less_five_percent() {
        let q = quote(300, USD, true);
        assert_eq!(q.slop_fee, 15);
        assert_eq!(q.maker_receives, 285);
        // 340 − (2.9% of 340 = 9.86 → 10) − 30 = 300: the total covers the published rate.
        assert_eq!(q.total, 340);
        assert_eq!(q.processing, 40);
        assert!(q.total - ceil_div(q.total * 290, 10_000) - 30 >= 300);
        for d in [100, 250, 500, 1234, 50_000] {
            let q = quote(d, USD, true);
            assert!(
                q.total - ceil_div(q.total * 290, 10_000) - 30 >= d - 1,
                "{d}: {q:?}"
            );
            assert!(q.total >= d && q.total > q.donation);
        }
    }

    #[test]
    fn without_covering_the_amount_is_the_total() {
        let q = quote(300, USD, false);
        assert_eq!(q.total, 300);
        assert_eq!(q.maker_receives, 300 - 15 - 39);
    }

    #[test]
    fn presets_sit_around_the_suggestion() {
        assert_eq!(presets(500), vec![250, 500, 1000, 2500]);
        assert_eq!(presets(100), vec![100, 200, 500]);
        assert_eq!(presets(50_000), vec![25_000, 50_000]);
    }

    #[test]
    fn reconciliation_compares_per_account() {
        let stripe = vec![
            ("acct_a".to_string(), 15),
            ("acct_a".to_string(), 25),
            ("acct_b".to_string(), 5),
        ];
        let ours = vec![("acct_a".to_string(), 40), ("acct_c".to_string(), 7)];
        assert_eq!(
            mismatches(&stripe, &ours),
            vec![("acct_b".to_string(), 5, 0), ("acct_c".to_string(), 0, 7)]
        );
    }

    /// PAY-5: nothing apps can call or receive (launch and Pulse tokens, update checks, release
    /// manifests, the desktop client) knows about donations.
    #[test]
    fn apps_cannot_learn_who_donated() {
        for (name, source) in [
            ("pulse.rs", include_str!("pulse.rs")),
            ("updates.rs", include_str!("updates.rs")),
            ("desktop_updates.rs", include_str!("desktop_updates.rs")),
            ("releases.rs", include_str!("releases.rs")),
            (
                "slop-manifest",
                include_str!("../../slop-manifest/src/lib.rs"),
            ),
        ] {
            let lower = source.to_lowercase();
            for word in ["donat", "supporter", "donor"] {
                assert!(!lower.contains(word), "{name} mentions \"{word}\"");
            }
        }
    }
}
