// SPDX-License-Identifier: AGPL-3.0-or-later
//! App of the Month (DESIGN §5, owner decisions 2026-11-19). Staff nominate public apps in the
//! store for the current month and vote (three votes each a month, never for an app they make);
//! the owner, signed in themselves, confirms up to three winners for a month. Each winner gets a
//! small cash award that the owner pays **by hand, as a donation to the app** through its Donate
//! button like any donor: SLAPorSLOP never pays makers from its own Stripe balance (PAY-1/PAY-2).
//! The amount is published with the badge on the app's page, the maker's profile and `/awards`.
//!
//! Awards are staff picks made for players: nothing about them can be paid for, and they don't
//! change any ranking. Everything staff do here is in the audit log.

use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, QueryBuilder};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState,
    admin_web::AdminUser,
    auth::session::MaybeUser,
    catalog::{self, AppSummary},
    donations,
    error::{ApiError, ApiResult},
    notify::{self, Notice},
    policy::ViewerPolicy,
    staff::{Perm, Role},
};

/// Winners a month, at most.
pub const MAX_WINNERS: usize = 3;
/// Votes each staff member has a month.
pub const MAX_VOTES: i64 = 3;
/// The award when the owner doesn't give one: $25.
pub const DEFAULT_AMOUNT_MINOR: i64 = 2500;
pub const DEFAULT_CURRENCY: &str = "usd";
const CURRENCIES: [&str; 3] = ["usd", "eur", "gbp"];

/// SQL: app `a` can be nominated or win: public, listed, in Beta or Released (so it has a Donate
/// button for the prize), and an app of its own (not a community build or a dedicated server).
const ELIGIBLE: &str =
    "(a.status = 'listed' and a.visibility = 'public' and a.parent_app_id is null
                         and a.stage in ('beta', 'released'))";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/awards", get(public_awards))
        .route("/admin/awards", get(admin_month))
        .route("/admin/awards/nominations", post(nominate))
        .route("/admin/awards/votes/{slug}", put(vote).delete(unvote))
        .route("/admin/awards/{month}/winners", put(confirm))
        .route("/admin/awards/{month}/winners/{slug}/paid", put(mark_paid))
}

// ---------------------------------------------------------------------------------------------
// Months

/// The first day of this month (UTC): nominations and votes are for it.
fn current_month() -> Date {
    let today = OffsetDateTime::now_utc().date();
    today.replace_day(1).unwrap_or(today)
}

/// "2026-11" → the first of November 2026.
fn parse_month(text: &str) -> ApiResult<Date> {
    let bad = || ApiError::invalid("month", "give the month as YYYY-MM, e.g. 2026-11");
    let (y, m) = text.split_once('-').ok_or_else(bad)?;
    if y.len() != 4 || m.len() != 2 {
        return Err(bad());
    }
    let year: i32 = y.parse().map_err(|_| bad())?;
    let month: u8 = m.parse().map_err(|_| bad())?;
    let month = time::Month::try_from(month).map_err(|_| bad())?;
    Date::from_calendar_date(year, month, 1).map_err(|_| bad())
}

fn month_key(d: Date) -> String {
    format!("{:04}-{:02}", d.year(), u8::from(d.month()))
}

/// "November 2026".
pub fn month_label(d: Date) -> String {
    format!("{} {}", d.month(), d.year())
}

// ---------------------------------------------------------------------------------------------
// Public

/// One of an app's awards, on its store page.
#[derive(Serialize, sqlx::FromRow)]
pub struct AppAward {
    /// YYYY-MM.
    month: String,
    position: i16,
    amount_minor: i32,
    currency: String,
    blurb: String,
}

/// The app's awards, newest first (its store page: `AppDetail.awards`).
pub async fn of_app(db: &PgPool, app_id: Uuid) -> sqlx::Result<Vec<AppAward>> {
    sqlx::query_as(
        "select to_char(month, 'YYYY-MM') as month, position, amount_minor, currency, blurb
         from app_awards where app_id = $1 order by month desc",
    )
    .bind(app_id)
    .fetch_all(db)
    .await
}

/// One award on a maker's profile.
#[derive(Serialize, sqlx::FromRow)]
pub struct MakerAward {
    slug: String,
    title: String,
    /// YYYY-MM.
    month: String,
    amount_minor: i32,
    currency: String,
}

/// The awards a maker won with apps that are in the store for this viewer, newest first.
pub async fn of_maker(
    state: &AppState,
    maker: Uuid,
    policy: ViewerPolicy,
) -> sqlx::Result<Vec<MakerAward>> {
    let mut qb = QueryBuilder::new(
        "select a.slug, a.title, to_char(w.month, 'YYYY-MM') as month, w.amount_minor, w.currency
         from app_awards w join apps a on a.id = w.app_id where w.maker_id = ",
    );
    qb.push_bind(maker).push(" and ");
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(" order by w.month desc, w.position");
    qb.build_query_as().fetch_all(&state.db).await
}

#[derive(Serialize, sqlx::FromRow)]
struct Winner {
    #[sqlx(flatten)]
    #[serde(flatten)]
    app: AppSummary,
    #[serde(skip)]
    month: String,
    position: i16,
    amount_minor: i32,
    currency: String,
    blurb: String,
    /// The maker it was awarded to, while their account is open.
    maker_id: Option<Uuid>,
    maker_name: Option<String>,
}

#[derive(Serialize)]
struct AwardMonth {
    /// YYYY-MM.
    month: String,
    winners: Vec<Winner>,
}

/// Every month's winners that are in the store for the viewer, newest month first.
async fn public_awards(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<AwardMonth>>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns_with(
        &mut qb,
        &state.config,
        ", to_char(w.month, 'YYYY-MM') as month, w.position, w.amount_minor, w.currency, w.blurb,
           mk.id as maker_id, mk.display_name as maker_name",
    );
    qb.push(
        " join app_awards w on w.app_id = a.id
          left join users mk on mk.id = w.maker_id and mk.closed_at is null and mk.suspended_at is null
          where ",
    );
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(" order by w.month desc, w.position");
    let rows: Vec<Winner> = qb.build_query_as().fetch_all(&state.db).await?;
    let mut months: Vec<AwardMonth> = Vec::new();
    for w in rows {
        match months.last_mut() {
            Some(m) if m.month == w.month => m.winners.push(w),
            _ => months.push(AwardMonth {
                month: w.month.clone(),
                winners: vec![w],
            }),
        }
    }
    Ok(Json(months))
}

// ---------------------------------------------------------------------------------------------
// Staff

/// Confirming winners and marking prizes paid: the owner, signed in themselves (never a token,
/// like approving a staff action).
fn owner_in_person(admin: &AdminUser) -> ApiResult<()> {
    if admin.token.is_some() || !admin.roles.contains(&Role::Owner) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

#[derive(Serialize, sqlx::FromRow)]
struct Nomination {
    slug: String,
    title: String,
    /// Still eligible (public, listed, Beta or Released).
    eligible: bool,
    /// The viewing staff member makes it: they can't vote for it.
    mine: bool,
    nominated_by: Option<String>,
    note: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    votes: i64,
    /// Who voted, by name.
    voters: Vec<String>,
    my_vote: bool,
}

#[derive(Serialize, sqlx::FromRow)]
struct StaffWinner {
    #[serde(skip)]
    app_id: Uuid,
    slug: String,
    title: String,
    position: i16,
    amount_minor: i32,
    currency: String,
    blurb: String,
    confirmed_by: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    confirmed_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    prize_paid_at: Option<OffsetDateTime>,
    /// It takes donations right now, so the prize can be paid.
    #[sqlx(skip)]
    accepts_donations: bool,
    /// The currency its Donate button takes.
    #[sqlx(skip)]
    donation_currency: Option<String>,
}

#[derive(Serialize)]
struct StaffMonth {
    /// YYYY-MM.
    month: String,
    /// It's this month: nominating and voting are open.
    current: bool,
    /// The viewer may confirm winners and mark prizes paid (the owner, signed in).
    can_confirm: bool,
    max_votes: i64,
    max_winners: usize,
    my_votes: i64,
    nominations: Vec<Nomination>,
    winners: Vec<StaffWinner>,
    /// Months with nominations or winners, newest first (this month always).
    months: Vec<String>,
}

#[derive(Deserialize)]
struct MonthQuery {
    month: Option<String>,
}

async fn admin_month(
    State(state): State<AppState>,
    admin: AdminUser,
    Query(q): Query<MonthQuery>,
) -> ApiResult<Json<StaffMonth>> {
    admin.need(Perm::Review)?;
    let month = match q.month.as_deref() {
        Some(m) if !m.is_empty() => parse_month(m)?,
        _ => current_month(),
    };
    month_view(&state, &admin, month).await.map(Json)
}

async fn month_view(state: &AppState, admin: &AdminUser, month: Date) -> ApiResult<StaffMonth> {
    let nominations: Vec<Nomination> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select a.slug, a.title, {ELIGIBLE} as eligible, a.maker_id = $2 as mine,
                (select u.display_name from users u where u.id = n.nominated_by) as nominated_by,
                n.note, n.created_at,
                (select count(*) from award_votes v where v.month = n.month and v.app_id = n.app_id) as votes,
                coalesce(array(select u.display_name from award_votes v join users u on u.id = v.voter_id
                               where v.month = n.month and v.app_id = n.app_id order by v.created_at), '{{}}') as voters,
                exists (select 1 from award_votes v
                        where v.month = n.month and v.app_id = n.app_id and v.voter_id = $2) as my_vote
         from award_nominations n join apps a on a.id = n.app_id
         where n.month = $1 order by votes desc, n.created_at"
    )))
    .bind(month)
    .bind(admin.user_id)
    .fetch_all(&state.db)
    .await?;
    let mut winners: Vec<StaffWinner> = sqlx::query_as(
        "select w.app_id, a.slug, a.title, w.position, w.amount_minor, w.currency, w.blurb,
                (select u.display_name from users u where u.id = w.confirmed_by) as confirmed_by,
                w.confirmed_at, w.prize_paid_at
         from app_awards w join apps a on a.id = w.app_id where w.month = $1 order by w.position",
    )
    .bind(month)
    .fetch_all(&state.db)
    .await?;
    for w in &mut winners {
        if let Some((taking, currency)) = donations::donation_state(state, w.app_id).await? {
            w.accepts_donations = taking;
            w.donation_currency = Some(currency);
        }
    }
    let my_votes: i64 =
        sqlx::query_scalar("select count(*) from award_votes where month = $1 and voter_id = $2")
            .bind(month)
            .bind(admin.user_id)
            .fetch_one(&state.db)
            .await?;
    let mut months: Vec<String> = sqlx::query_scalar(
        "select to_char(m, 'YYYY-MM') from (select month as m from award_nominations
                                            union select month from app_awards) x
         order by m desc",
    )
    .fetch_all(&state.db)
    .await?;
    let this = month_key(current_month());
    if !months.contains(&this) {
        months.insert(0, this);
    }
    Ok(StaffMonth {
        month: month_key(month),
        current: month == current_month(),
        can_confirm: owner_in_person(admin).is_ok(),
        max_votes: MAX_VOTES,
        max_winners: MAX_WINNERS,
        my_votes,
        nominations,
        winners,
        months,
    })
}

#[derive(sqlx::FromRow)]
struct Candidate {
    id: Uuid,
    title: String,
    maker_id: Uuid,
    eligible: bool,
}

async fn candidate<'c>(db: impl sqlx::PgExecutor<'c>, slug: &str) -> ApiResult<Candidate> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select a.id, a.title, a.maker_id, {ELIGIBLE} as eligible from apps a where a.slug = $1"
    )))
    .bind(slug)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)
}

const OWN_APP: &str = "you make this app, so you can't nominate, vote for or confirm it";
const NOT_ELIGIBLE: &str =
    "only a public app in the store, in Beta or Released, can be App of the Month";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NominateBody {
    slug: String,
    #[serde(default)]
    note: String,
}

/// Puts an app forward for this month.
async fn nominate(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<NominateBody>,
) -> ApiResult<(StatusCode, Json<StaffMonth>)> {
    admin.need(Perm::Review)?;
    let note = body.note.trim();
    if note.chars().count() > 500 {
        return Err(ApiError::invalid("note", "keep it to 500 characters"));
    }
    let slug = body.slug.trim();
    let app = candidate(&state.db, slug).await?;
    if app.maker_id == admin.user_id {
        return Err(ApiError::Refused(OWN_APP));
    }
    if !app.eligible {
        return Err(ApiError::Conflict(NOT_ELIGIBLE.into()));
    }
    let month = current_month();
    let mut tx = state.db.begin().await?;
    let added = sqlx::query(
        "insert into award_nominations (month, app_id, nominated_by, note) values ($1, $2, $3, $4)
         on conflict do nothing",
    )
    .bind(month)
    .bind(app.id)
    .bind(admin.user_id)
    .bind(note)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added == 0 {
        return Err(ApiError::Conflict(format!(
            "{} is already nominated for {}",
            app.title,
            month_label(month)
        )));
    }
    crate::admin::audit(
        &mut tx,
        &admin.actor,
        "awards.nominate",
        slug,
        json!({ "month": month_key(month), "note": note }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(month_view(&state, &admin, month).await?),
    ))
}

/// A nominated app this month, by its address, for a vote.
/// Taking a vote back works even once the app has left the store; voting doesn't.
async fn nominated(
    state: &AppState,
    admin: &AdminUser,
    slug: &str,
    voting: bool,
) -> ApiResult<(Uuid, Date)> {
    let month = current_month();
    let row: Option<(Uuid, Uuid, bool)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select a.id, a.maker_id, {ELIGIBLE} from award_nominations n join apps a on a.id = n.app_id
         where n.month = $1 and a.slug = $2"
    )))
    .bind(month)
    .bind(slug)
    .fetch_optional(&state.db)
    .await?;
    let (app_id, maker_id, eligible) = row.ok_or(ApiError::NotFound)?;
    if maker_id == admin.user_id {
        return Err(ApiError::Refused(OWN_APP));
    }
    if voting && !eligible {
        return Err(ApiError::Conflict(NOT_ELIGIBLE.into()));
    }
    Ok((app_id, month))
}

/// One vote for a nomination this month (three a month; voting again for the same app is a no-op).
async fn vote(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    admin.need(Perm::Review)?;
    let (app_id, month) = nominated(&state, &admin, &slug, true).await?;
    let mut tx = state.db.begin().await?;
    // One staff member's votes for a month are counted and added one request at a time.
    sqlx::query(
        "select pg_advisory_xact_lock(hashtextextended('award_votes:' || $1::text || $2::text, 0))",
    )
    .bind(admin.user_id)
    .bind(month)
    .execute(&mut *tx)
    .await?;
    let (used, already): (i64, bool) = sqlx::query_as(
        "select count(*), coalesce(bool_or(app_id = $3), false) from award_votes
         where month = $1 and voter_id = $2",
    )
    .bind(month)
    .bind(admin.user_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;
    if already {
        return Ok(StatusCode::NO_CONTENT);
    }
    if used >= MAX_VOTES {
        return Err(ApiError::Conflict(format!(
            "you've used your {MAX_VOTES} votes for {}: take one back first",
            month_label(month)
        )));
    }
    sqlx::query("insert into award_votes (month, app_id, voter_id) values ($1, $2, $3)")
        .bind(month)
        .bind(app_id)
        .bind(admin.user_id)
        .execute(&mut *tx)
        .await?;
    crate::admin::audit(
        &mut tx,
        &admin.actor,
        "awards.vote",
        &slug,
        json!({ "month": month_key(month) }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Takes back a vote for this month.
async fn unvote(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    admin.need(Perm::Review)?;
    let (app_id, month) = nominated(&state, &admin, &slug, false).await?;
    let mut tx = state.db.begin().await?;
    let removed =
        sqlx::query("delete from award_votes where month = $1 and app_id = $2 and voter_id = $3")
            .bind(month)
            .bind(app_id)
            .bind(admin.user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if removed > 0 {
        crate::admin::audit(
            &mut tx,
            &admin.actor,
            "awards.unvote",
            &slug,
            json!({ "month": month_key(month) }),
        )
        .await
        .map_err(ApiError::Internal)?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WinnerIn {
    slug: String,
    /// Default $25.
    amount_minor: Option<i64>,
    /// usd, eur or gbp; default usd.
    currency: Option<String>,
    #[serde(default)]
    blurb: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmBody {
    winners: Vec<WinnerIn>,
}

/// The owner confirms a month's winners (this month or an earlier one), in order: they replace
/// the month's winners, until any of its prizes is marked paid. Each new winner's maker is told
/// the award, and that it comes as a donation from SLAPorSLOP to the app.
async fn confirm(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(month): Path<String>,
    Json(body): Json<ConfirmBody>,
) -> ApiResult<Json<StaffMonth>> {
    owner_in_person(&admin)?;
    let month = parse_month(&month)?;
    if month > current_month() {
        return Err(ApiError::invalid(
            "month",
            "pick winners for this month or an earlier one",
        ));
    }
    if body.winners.len() > MAX_WINNERS {
        return Err(ApiError::invalid(
            "winners",
            format!("pick at most {MAX_WINNERS} winners a month"),
        ));
    }
    let mut seen = HashSet::new();
    if !body.winners.iter().all(|w| seen.insert(w.slug.trim())) {
        return Err(ApiError::invalid("winners", "each app wins once a month"));
    }
    let label = month_label(month);
    let mut tx = state.db.begin().await?;
    sqlx::query("select pg_advisory_xact_lock(hashtextextended('app_awards:' || $1::text, 0))")
        .bind(month)
        .execute(&mut *tx)
        .await?;
    let paid: bool = sqlx::query_scalar(
        "select exists (select 1 from app_awards where month = $1 and prize_paid_at is not null)",
    )
    .bind(month)
    .fetch_one(&mut *tx)
    .await?;
    if paid {
        return Err(ApiError::Conflict(format!(
            "a prize for {label} is already marked paid, so its winners are fixed"
        )));
    }
    let before: Vec<(Uuid, Uuid, String, i32, String)> = sqlx::query_as(
        "select w.app_id, w.maker_id, a.title, w.amount_minor, w.currency
         from app_awards w join apps a on a.id = w.app_id where w.month = $1",
    )
    .bind(month)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("delete from app_awards where month = $1")
        .bind(month)
        .execute(&mut *tx)
        .await?;
    let mut logged = Vec::new();
    let mut won = Vec::new();
    for (i, w) in body.winners.iter().enumerate() {
        let slug = w.slug.trim();
        let amount = w.amount_minor.unwrap_or(DEFAULT_AMOUNT_MINOR);
        if !(donations::MIN_MINOR..=donations::MAX_MINOR).contains(&amount) {
            return Err(ApiError::invalid(
                "amount_minor",
                format!(
                    "an award is {} to {}, like a donation",
                    donations::money(donations::MIN_MINOR, "usd"),
                    donations::money(donations::MAX_MINOR, "usd")
                ),
            ));
        }
        let currency = w
            .currency
            .as_deref()
            .unwrap_or(DEFAULT_CURRENCY)
            .to_lowercase();
        if !CURRENCIES.contains(&currency.as_str()) {
            return Err(ApiError::invalid("currency", "usd, eur or gbp"));
        }
        let blurb = w.blurb.trim();
        if blurb.chars().count() > 160 {
            return Err(ApiError::invalid("blurb", "keep it to 160 characters"));
        }
        let app = candidate(&mut *tx, slug).await.map_err(|e| match e {
            ApiError::NotFound => ApiError::invalid("winners", format!("no app at {slug}")),
            e => e,
        })?;
        let was_nominated: bool = sqlx::query_scalar(
            "select exists (select 1 from award_nominations where month = $1 and app_id = $2)",
        )
        .bind(month)
        .bind(app.id)
        .fetch_one(&mut *tx)
        .await?;
        if !was_nominated {
            return Err(ApiError::invalid(
                "winners",
                format!("{} wasn't nominated for {label}", app.title),
            ));
        }
        if app.maker_id == admin.user_id {
            return Err(ApiError::Refused(OWN_APP));
        }
        if !app.eligible {
            return Err(ApiError::Conflict(format!("{}: {NOT_ELIGIBLE}", app.title)));
        }
        sqlx::query(
            "insert into app_awards (month, app_id, maker_id, position, amount_minor, currency, blurb, confirmed_by)
             values ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(month)
        .bind(app.id)
        .bind(app.maker_id)
        .bind((i + 1) as i16)
        .bind(amount as i32)
        .bind(&currency)
        .bind(blurb)
        .bind(admin.user_id)
        .execute(&mut *tx)
        .await?;
        logged.push(json!({ "slug": slug, "amount_minor": amount, "currency": currency }));
        won.push((app, slug.to_string(), amount, currency));
    }
    crate::admin::audit(
        &mut tx,
        &admin.actor,
        "awards.confirm",
        &month_key(month),
        json!({ "winners": logged }),
    )
    .await
    .map_err(ApiError::Internal)?;
    // Makers hear once: when their app wins, or its award changes; and if it's taken off.
    for (app, slug, amount, currency) in &won {
        let unchanged = before.iter().any(|(id, maker, _, a, c)| {
            id == &app.id && maker == &app.maker_id && i64::from(*a) == *amount && c == currency
        });
        if unchanged {
            continue;
        }
        let money = donations::money(*amount, currency);
        notify::send(
            &mut tx,
            Notice {
                user_id: app.maker_id,
                category: "apps",
                title: format!("{} is App of the Month · {label}", app.title),
                body: format!(
                    "SLAPorSLOP staff picked {} as an App of the Month for {label}, with a {money} award. \
                     The award comes as a donation from SLAPorSLOP to {}, through its Donate button: \
                     keep donations on so it can be paid. The badge shows on its page and your profile.",
                    app.title, app.title
                ),
                link: format!("/app/{slug}"),
                collapse_key: Some(format!("award:{}:{}", month_key(month), app.id)),
            },
        )
        .await?;
    }
    for (id, maker, title, _, _) in &before {
        if won.iter().any(|(app, ..)| &app.id == id) {
            continue;
        }
        notify::send(
            &mut tx,
            Notice {
                user_id: *maker,
                category: "apps",
                title: format!("App of the Month for {label} was corrected"),
                body: format!(
                    "SLAPorSLOP corrected its App of the Month list for {label}: {title} isn't on it any more. Sorry for the mix-up."
                ),
                link: "/awards".into(),
                collapse_key: Some(format!("award:{}:{}", month_key(month), id)),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(month_view(&state, &admin, month).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaidBody {
    paid: bool,
}

/// The owner marks a winner's prize donation made (or not made after all).
async fn mark_paid(
    State(state): State<AppState>,
    admin: AdminUser,
    Path((month, slug)): Path<(String, String)>,
    Json(body): Json<PaidBody>,
) -> ApiResult<Json<StaffMonth>> {
    owner_in_person(&admin)?;
    let month = parse_month(&month)?;
    let mut tx = state.db.begin().await?;
    let changed = sqlx::query(
        "update app_awards set prize_paid_at = case when $3 then coalesce(prize_paid_at, now()) end
         where month = $1 and app_id = (select id from apps where slug = $2)",
    )
    .bind(month)
    .bind(&slug)
    .bind(body.paid)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    crate::admin::audit(
        &mut tx,
        &admin.actor,
        if body.paid {
            "awards.prize_paid"
        } else {
            "awards.prize_unpaid"
        },
        &slug,
        json!({ "month": month_key(month) }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(month_view(&state, &admin, month).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_are_year_dash_month() {
        let d = parse_month("2026-11").unwrap();
        assert_eq!(month_key(d), "2026-11");
        assert_eq!(month_label(d), "November 2026");
        for bad in [
            "2026-13",
            "2026-1",
            "26-11",
            "2026/11",
            "2026-11-01",
            "",
            "abcd-ef",
        ] {
            assert!(parse_month(bad).is_err(), "{bad}");
        }
    }
}
