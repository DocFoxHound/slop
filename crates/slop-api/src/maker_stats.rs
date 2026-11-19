// SPDX-License-Identifier: AGPL-3.0-or-later
//! The maker's Stats tab (Publish → Stats, DESIGN §8): numbers about every app they make, drafts
//! included, and their account overall, on one page.
//!
//! It only reads what is already kept, the same way the places that show each number read it:
//!
//! - **Library adds and first plays:** the maker's History counts (`app_daily_counts`,
//!   history.rs), so the same rules hold: never for a private app, a dedicated server or the
//!   maker's own account, and nothing before `apps.history_from`. Players per day aren't added
//!   up (an account that plays on three days counts on each); first plays are.
//! - **In libraries now, Interested, votes:** the counts the store shows (catalog.rs), with the
//!   weighted Slaps % only from `votes::MIN_VOTES` votes up.
//! - **Most players at once:** Slop Pulse's 5-minute counts (pulse.rs).
//! - **Donations:** totals per currency, like the app's donation summary (donations.rs), and
//!   real or simulated as the server is, never both (statements.rs). Counts and totals only:
//!   never a donor, and no month breakdowns, which hide groups under `MIN_GROUP` on the app's own
//!   donations panel.
//! - **Awards:** App of the Month (awards.rs). The account's count is the maker's, like their
//!   profile: an award stays theirs if the app changes hands.
//!
//! Whose: apps whose maker is the signed-in user (`apps.maker_id`), as Manage decides
//! (`updates::owned`); a branch or dedicated server is its own maker's. Money is the person's own:
//! AI agent tokens can't reach this route (it isn't in maker_agents.rs).

use std::collections::BTreeMap;

use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    AppState, auth::session::CurrentUser, catalog::AppStage, error::ApiResult, limits, statements,
    votes::MIN_VOTES,
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/me/maker-stats", get(stats))
}

/// The days of the daily series: the last 30 UTC days, today included.
const DAYS: i32 = 30;

/// Donations in one currency.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Money {
    pub currency: String,
    /// Paid donations (refunded ones included).
    pub donations: i64,
    /// What the donors chose for the maker, before SLAPorSLOP's fee.
    pub donated_minor: i64,
    pub refunded_minor: i64,
}

impl Money {
    fn add(&mut self, o: &Money) {
        self.donations += o.donations;
        self.donated_minor += o.donated_minor;
        self.refunded_minor += o.refunded_minor;
    }
}

/// Adds `rows` into `into`, per currency, keeping it sorted by currency.
fn add_money(into: &mut Vec<Money>, rows: &[Money]) {
    let mut by: BTreeMap<String, Money> = into.drain(..).map(|m| (m.currency.clone(), m)).collect();
    for r in rows {
        by.entry(r.currency.clone())
            .or_insert_with(|| Money {
                currency: r.currency.clone(),
                ..Money::default()
            })
            .add(r);
    }
    into.extend(by.into_values());
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Award {
    /// YYYY-MM.
    pub month: String,
    pub position: i16,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct AppStats {
    #[serde(skip)]
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    /// game or app.
    pub kind: String,
    pub stage: AppStage,
    /// draft, listed or delisted.
    pub status: String,
    pub private: bool,
    /// The app it's a branch of (a mod, fix, port…), if it is one.
    pub branch_of: Option<String>,
    /// The game it's a dedicated server for, if it is one.
    pub server_for: Option<String>,
    /// Whether library adds and plays are counted for it at all: not for a private app or a
    /// dedicated server. When false, those numbers are null.
    pub counted: bool,
    /// Times added to a library since counting began (history.rs).
    pub library_adds: Option<i64>,
    pub library_adds_7d: Option<i64>,
    pub library_adds_30d: Option<i64>,
    /// Library adds on each of the last 30 UTC days, oldest first; null for a day before
    /// counting began.
    #[sqlx(skip)]
    pub daily_library_adds: Vec<Option<i32>>,
    /// Accounts that started it for the first time in the last 30 days.
    pub first_plays_30d: Option<i64>,
    /// The most playing at once in the last 30 days (Slop Pulse); null without Pulse counts.
    pub peak_players_30d: Option<i32>,
    /// Libraries it's in now, besides its maker's.
    pub in_libraries: i64,
    pub not_slop: i64,
    pub slop: i64,
    /// Weighted % Slaps; null below votes::MIN_VOTES.
    pub not_slop_percent: Option<i32>,
    pub interested: i64,
    pub releases_published: i64,
    /// Bounties open or waiting for a delivery to be confirmed.
    pub open_bounties: i64,
    /// Written reviews shown on its page.
    pub reviews: i64,
    #[sqlx(skip)]
    pub donations: Vec<Money>,
    /// App of the Month, newest first.
    #[sqlx(skip)]
    pub awards: Vec<Award>,
}

#[derive(Serialize, Default)]
pub struct Counts {
    pub draft: i64,
    pub listed: i64,
    pub delisted: i64,
    pub coming_soon: i64,
    pub beta: i64,
    pub released: i64,
    pub private: i64,
}

#[derive(Serialize)]
pub struct Busiest {
    pub slug: String,
    pub title: String,
    pub library_adds_30d: i64,
    pub peak_players_30d: Option<i32>,
}

#[derive(Serialize, Default)]
pub struct Account {
    pub apps: i64,
    pub counts: Counts,
    pub library_adds: i64,
    pub library_adds_30d: i64,
    pub first_plays_30d: i64,
    pub in_libraries: i64,
    pub not_slop: i64,
    pub slop: i64,
    /// Weighted over every vote on the maker's apps; null below votes::MIN_VOTES in all.
    pub not_slop_percent: Option<i32>,
    pub interested: i64,
    pub releases_published: i64,
    /// App of the Month awards won, ever.
    pub awards: i64,
    pub donations: Vec<Money>,
    /// Paid this calendar year (UTC).
    pub donations_this_year: Vec<Money>,
    /// The app with the most library adds in the last 30 days (then players at once).
    pub busiest: Option<Busiest>,
}

#[derive(Serialize)]
pub struct MakerStats {
    /// The donation figures are simulated payments (this server takes no real ones).
    pub simulated: bool,
    /// The 30 UTC days of each app's daily series, oldest first (YYYY-MM-DD).
    pub days: Vec<String>,
    pub account: Account,
    /// Newest changed first, as on Your apps.
    pub apps: Vec<AppStats>,
}

/// Library adds and plays aren't counted for these (pulse.rs, library.rs).
const COUNTED: &str = "(a.visibility <> 'private' and a.branch_kind is distinct from 'server')";
/// The first of the last `$2` UTC days.
const FROM_DAY: &str = "((now() at time zone 'UTC')::date - ($2::int - 1))";

async fn stats(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<MakerStats>> {
    limits::check(&state, user.id, limits::Action::History)?;
    Ok(Json(load(&state, user.id).await?))
}

/// Everything on the Stats tab for `maker`.
pub async fn load(state: &AppState, maker: Uuid) -> sqlx::Result<MakerStats> {
    let db = &state.db;
    let simulated = statements::simulated(state);
    let mut apps: Vec<AppStats> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select a.id, a.slug, a.title, a.kind::text as kind, a.stage, a.status::text as status,
                a.visibility = 'private' as private,
                (select p.slug from apps p where p.id = a.parent_app_id and a.branch_kind <> 'server') as branch_of,
                (select p.slug from apps p where p.id = a.parent_app_id and a.branch_kind = 'server') as server_for,
                {COUNTED} as counted,
                case when {COUNTED} then
                    (select coalesce(sum(c.library_adds), 0)::int8 from app_daily_counts c where c.app_id = a.id) end as library_adds,
                case when {COUNTED} then
                    (select coalesce(sum(c.library_adds), 0)::int8 from app_daily_counts c
                     where c.app_id = a.id and c.day > (now() at time zone 'UTC')::date - 7) end as library_adds_7d,
                case when {COUNTED} then
                    (select coalesce(sum(c.library_adds), 0)::int8 from app_daily_counts c
                     where c.app_id = a.id and c.day >= {FROM_DAY}) end as library_adds_30d,
                case when {COUNTED} then
                    (select coalesce(sum(c.first_plays), 0)::int8 from app_daily_counts c
                     where c.app_id = a.id and c.day >= {FROM_DAY}) end as first_plays_30d,
                (select max(p.players) from pulse_counts p
                 where p.app_id = a.id and p.bucket >= {FROM_DAY}::timestamp at time zone 'UTC') as peak_players_30d,
                (select count(*) from library_entries l where l.app_id = a.id and l.user_id <> a.maker_id) as in_libraries,
                (select count(*) from votes v where v.app_id = a.id and v.vote = 1) as not_slop,
                (select count(*) from votes v where v.app_id = a.id and v.vote = -1) as slop,
                (select case when count(*) >= $3 and sum(v.weight) > 0
                             then round(100 * coalesce(sum(v.weight) filter (where v.vote = 1), 0) / sum(v.weight))::int4 end
                 from votes v where v.app_id = a.id) as not_slop_percent,
                (select count(*) from interests i where i.app_id = a.id) as interested,
                (select count(*) from releases r where r.app_id = a.id and r.status in ('approved', 'revoked')) as releases_published,
                (select count(*) from bounties b where b.app_id = a.id and b.status in ('open', 'claimed')) as open_bounties,
                (select count(*) from app_reviews r where r.app_id = a.id and r.hidden_at is null) as reviews
         from apps a where a.maker_id = $1
         order by a.updated_at desc, a.slug"
    )))
    .bind(maker)
    .bind(DAYS)
    .bind(MIN_VOTES)
    .fetch_all(db)
    .await?;

    let days: Vec<String> = sqlx::query_scalar(
        "select to_char(d, 'YYYY-MM-DD')
         from generate_series((now() at time zone 'UTC')::date - ($1::int - 1), (now() at time zone 'UTC')::date,
                              interval '1 day') d
         order by d",
    )
    .bind(DAYS)
    .fetch_all(db)
    .await?;

    // Each counted app's days: null before counting began, as on its History.
    let series: Vec<(Uuid, Option<i32>)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select a.id, case when d.day >= a.history_from then coalesce(c.library_adds, 0) end
         from apps a
         cross join (select generate_series({FROM_DAY}, (now() at time zone 'UTC')::date, interval '1 day')::date as day) d
         left join app_daily_counts c on c.app_id = a.id and c.day = d.day
         where a.maker_id = $1 and {COUNTED}
         order by a.id, d.day"
    )))
    .bind(maker)
    .bind(DAYS)
    .fetch_all(db)
    .await?;

    // Paid donations per app, currency and whether this year. Pending and expired ones never
    // were (donations.rs `summary`).
    let money: Vec<(Uuid, String, bool, i64, i64, i64)> = sqlx::query_as(
        "select d.app_id, d.currency,
                d.paid_at >= date_trunc('year', now() at time zone 'UTC') at time zone 'UTC' as this_year,
                count(*), sum(d.donation_minor)::int8, sum(d.refunded_minor)::int8
         from donations d join apps a on a.id = d.app_id
         where a.maker_id = $1 and d.status not in ('pending', 'expired') and d.paid_at is not null
           and d.simulated = $2
         group by 1, 2, 3",
    )
    .bind(maker)
    .bind(simulated)
    .fetch_all(db)
    .await?;

    let awards: Vec<(Uuid, String, i16)> = sqlx::query_as(
        "select w.app_id, to_char(w.month, 'YYYY-MM'), w.position
         from app_awards w join apps a on a.id = w.app_id
         where a.maker_id = $1 order by w.month desc, w.position",
    )
    .bind(maker)
    .fetch_all(db)
    .await?;

    let awards_won: i64 = sqlx::query_scalar("select count(*) from app_awards where maker_id = $1")
        .bind(maker)
        .fetch_one(db)
        .await?;
    let not_slop_percent: Option<i32> = sqlx::query_scalar(
        "select case when count(*) >= $2 and sum(v.weight) > 0
                     then round(100 * coalesce(sum(v.weight) filter (where v.vote = 1), 0) / sum(v.weight))::int4 end
         from votes v join apps a on a.id = v.app_id where a.maker_id = $1",
    )
    .bind(maker)
    .bind(MIN_VOTES)
    .fetch_one(db)
    .await?;
    let mut account = Account {
        awards: awards_won,
        not_slop_percent,
        ..Account::default()
    };

    let mut this_year = Vec::new();
    for app in &mut apps {
        app.daily_library_adds = series
            .iter()
            .filter(|(id, _)| *id == app.id)
            .map(|(_, n)| *n)
            .collect();
        let all: Vec<Money> = money
            .iter()
            .filter(|m| m.0 == app.id)
            .map(|(_, currency, _, n, donated, refunded)| Money {
                currency: currency.clone(),
                donations: *n,
                donated_minor: *donated,
                refunded_minor: *refunded,
            })
            .collect();
        add_money(&mut app.donations, &all);
        let year: Vec<Money> = money
            .iter()
            .filter(|m| m.0 == app.id && m.2)
            .map(|(_, currency, _, n, donated, refunded)| Money {
                currency: currency.clone(),
                donations: *n,
                donated_minor: *donated,
                refunded_minor: *refunded,
            })
            .collect();
        add_money(&mut this_year, &year);
        app.awards = awards
            .iter()
            .filter(|a| a.0 == app.id)
            .map(|(_, month, position)| Award {
                month: month.clone(),
                position: *position,
            })
            .collect();

        account.apps += 1;
        let c = &mut account.counts;
        *match app.status.as_str() {
            "listed" => &mut c.listed,
            "delisted" => &mut c.delisted,
            _ => &mut c.draft,
        } += 1;
        *match app.stage {
            AppStage::ComingSoon => &mut c.coming_soon,
            AppStage::Beta => &mut c.beta,
            AppStage::Released => &mut c.released,
        } += 1;
        c.private += i64::from(app.private);
        account.library_adds += app.library_adds.unwrap_or(0);
        account.library_adds_30d += app.library_adds_30d.unwrap_or(0);
        account.first_plays_30d += app.first_plays_30d.unwrap_or(0);
        account.in_libraries += app.in_libraries;
        account.not_slop += app.not_slop;
        account.slop += app.slop;
        account.interested += app.interested;
        account.releases_published += app.releases_published;
        add_money(&mut account.donations, &app.donations);
    }
    account.donations_this_year = this_year;
    account.busiest = busiest(&apps);
    Ok(MakerStats {
        simulated,
        days,
        account,
        apps,
    })
}

/// The app with the most library adds in the last 30 days, then the most players at once, then
/// first plays; none when nothing happened anywhere.
fn busiest(apps: &[AppStats]) -> Option<Busiest> {
    apps.iter()
        .map(|a| {
            let adds = a.library_adds_30d.unwrap_or(0);
            let peak = a.peak_players_30d.unwrap_or(0);
            let first = a.first_plays_30d.unwrap_or(0);
            ((adds, i64::from(peak), first), a)
        })
        .filter(|(key, _)| *key != (0, 0, 0))
        // The first of equals: max_by_key would take the last.
        .fold(
            None::<((i64, i64, i64), &AppStats)>,
            |best, (key, a)| match best {
                Some((k, _)) if k >= key => best,
                _ => Some((key, a)),
            },
        )
        .map(|(_, a)| Busiest {
            slug: a.slug.clone(),
            title: a.title.clone(),
            library_adds_30d: a.library_adds_30d.unwrap_or(0),
            peak_players_30d: a.peak_players_30d,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn money(currency: &str, n: i64, donated: i64) -> Money {
        Money {
            currency: currency.into(),
            donations: n,
            donated_minor: donated,
            refunded_minor: 0,
        }
    }

    #[test]
    fn money_adds_up_per_currency_in_order() {
        let mut into = vec![money("usd", 1, 500)];
        add_money(&mut into, &[money("eur", 2, 300), money("usd", 3, 700)]);
        assert_eq!(into, vec![money("eur", 2, 300), money("usd", 4, 1200)]);
    }
}
