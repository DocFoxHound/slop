// SPDX-License-Identifier: AGPL-3.0-or-later
//! Yearly money statements for makers and creatives (PAYMENTS §8, decided 2026-11-19): what was
//! paid into their own Stripe accounts in a year, by month, currency and kind (donations, bounty
//! payouts, People jobs), with refunds, disputes and SLAPorSLOP's fee, on Account → Money and as
//! a CSV file.
//!
//! - **A summary for their records, not a tax form.** Every payment was a direct charge on their
//!   own Stripe account, so their Stripe dashboard has each one, and Stripe sends any US 1099-K.
//!   SLAPorSLOP doesn't give tax advice.
//! - **Totals only**, never a row per payment or per payer: a maker's donation summary hides
//!   groups smaller than 10 so no donor stands out (donations.rs `MIN_GROUP`), and a statement
//!   mustn't undo that.
//! - **Whose money:** paid into a Stripe account that was theirs (`stripe_account_owners`,
//!   migration 0106), so it stays on their statements after they disconnect or switch accounts.
//!   Refunds and disputes count in the month they were recorded; SLAPorSLOP's fee isn't returned
//!   with a refund, so it stays counted. Months are in UTC.
//! - **Real or simulated, never both:** a server taking real payments counts only real ones; one
//!   that simulates them (the sandbox, development) only simulated ones, and says so.
//! - The person's own: AI agent tokens can't reach these routes (none is in maker_agents.rs).

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    donations::{VatTreatment, vat_included, vat_treatment},
    error::{ApiError, ApiResult},
    people::stripe::{self, Mode},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/statements", get(years))
        .route("/me/statements/{year}", get(year))
}

/// The totals for one period (a month or a year), currency and kind.
#[derive(Serialize, sqlx::FromRow, Clone, Debug, PartialEq)]
pub struct Totals {
    /// YYYY-MM for a month, YYYY for the year.
    pub period: String,
    pub currency: String,
    /// donation, bounty or people_job
    pub kind: String,
    /// Payments received (refunds and disputes aren't counted).
    pub payments: i64,
    /// What the payers were charged in all.
    pub charged_minor: i64,
    /// The card processing donors added on top of their donation (PAYMENTS §5.1), which went
    /// toward Stripe's fee rather than to the maker.
    pub processing_covered_minor: i64,
    /// What the payers paid toward the maker or creative: charged, less the processing added.
    #[sqlx(skip)]
    pub gross_minor: i64,
    pub refunded_minor: i64,
    /// Card disputes as Stripe reported them when they were opened.
    pub disputed_minor: i64,
    pub slop_fee_minor: i64,
    /// The VAT included in SLAPorSLOP's fee (LEGAL §3.5); zero where there's none or it's
    /// reverse-charged.
    #[sqlx(skip)]
    pub vat_minor: i64,
    /// Gross, less refunds, disputes and SLAPorSLOP's fee. Stripe's own processing fees aren't
    /// taken off: they're in the Stripe dashboard.
    #[sqlx(skip)]
    pub net_minor: i64,
}

impl Totals {
    fn finish(&mut self, vat_rate_bp: Option<i64>) {
        self.gross_minor = self.charged_minor - self.processing_covered_minor;
        self.net_minor =
            self.gross_minor - self.refunded_minor - self.disputed_minor - self.slop_fee_minor;
        self.vat_minor = vat_rate_bp.map_or(0, |rate| vat_included(self.slop_fee_minor, rate));
    }

    fn add(&mut self, o: &Totals) {
        self.payments += o.payments;
        self.charged_minor += o.charged_minor;
        self.processing_covered_minor += o.processing_covered_minor;
        self.gross_minor += o.gross_minor;
        self.refunded_minor += o.refunded_minor;
        self.disputed_minor += o.disputed_minor;
        self.slop_fee_minor += o.slop_fee_minor;
        self.vat_minor += o.vat_minor;
        self.net_minor += o.net_minor;
    }
}

/// Whether this server's statements are of simulated payments (it takes no real ones).
pub fn simulated(state: &AppState) -> bool {
    matches!(stripe::mode(state), Mode::Simulated)
}

/// The rate at which VAT is included in SLAPorSLOP's fee for this user, if it is.
pub fn vat_rate(vat: &VatTreatment) -> Option<i64> {
    match (vat.kind, vat.rate_bp) {
        ("included", Some(rate)) => Some(i64::from(rate)),
        _ => None,
    }
}

/// Monthly totals of the money paid into `user`'s Stripe accounts, oldest first: in `year` (UTC)
/// or every year, real payments or simulated ones. Refunds and disputes count under the kind of
/// payment they were against; a People job's name no account, so they're found through it.
pub async fn monthly(
    db: &PgPool,
    user: Uuid,
    year: Option<i32>,
    simulated: bool,
    vat_rate_bp: Option<i64>,
) -> sqlx::Result<Vec<Totals>> {
    let mut rows: Vec<Totals> = sqlx::query_as(
        "with mine as (
             select p.id from payments p
             join stripe_account_owners w on w.stripe_account_id = p.stripe_account_id
             where w.user_id = $1
         ), entries as (
             select p.created_at at time zone 'UTC' as at, p.currency, p.kind as entry,
                    p.amount_minor, p.slop_fee_minor,
                    coalesce(o.kind, case when p.donation_id is not null then 'donation'
                                          when p.bounty_pledge_id is not null then 'bounty'
                                          when p.kind in ('refund', 'dispute') then 'people_job'
                                          else p.kind end) as kind,
                    case when p.kind = 'donation' and d.donor_covers_fees
                         then greatest(d.total_minor - d.donation_minor, 0) else 0 end as covered,
                    p.simulated or coalesce(o.simulated, false) or coalesce(d.simulated, false) as simulated
             from payments p
             left join payments o on o.id = p.refund_of
             left join donations d on d.id = coalesce(p.donation_id, o.donation_id)
             where p.id in (select id from mine union select r.id from payments r where r.refund_of in (select id from mine))
         )
         select to_char(at, 'YYYY-MM') as period, currency, kind,
                count(*) filter (where entry not in ('refund', 'dispute')) as payments,
                coalesce(sum(amount_minor) filter (where entry not in ('refund', 'dispute')), 0)::int8 as charged_minor,
                coalesce(sum(covered), 0)::int8 as processing_covered_minor,
                coalesce(-sum(amount_minor) filter (where entry = 'refund'), 0)::int8 as refunded_minor,
                coalesce(-sum(amount_minor) filter (where entry = 'dispute'), 0)::int8 as disputed_minor,
                coalesce(sum(slop_fee_minor), 0)::int8 as slop_fee_minor
         from entries
         where simulated = $2 and ($3::int4 is null or extract(year from at) = $3)
         group by 1, 2, 3 order by 1, 2, 3",
    )
    .bind(user)
    .bind(simulated)
    .bind(year)
    .fetch_all(db)
    .await?;
    for r in &mut rows {
        r.finish(vat_rate_bp);
    }
    Ok(rows)
}

/// A year's totals per currency and kind, from its months (the VAT is the months' VAT added up,
/// as each month's fee statement showed it).
fn year_totals(year: i32, months: &[Totals]) -> Vec<Totals> {
    let mut by: BTreeMap<(String, String), Totals> = BTreeMap::new();
    for m in months {
        by.entry((m.currency.clone(), m.kind.clone()))
            .or_insert_with(|| Totals {
                period: year.to_string(),
                currency: m.currency.clone(),
                kind: m.kind.clone(),
                payments: 0,
                charged_minor: 0,
                processing_covered_minor: 0,
                gross_minor: 0,
                refunded_minor: 0,
                disputed_minor: 0,
                slop_fee_minor: 0,
                vat_minor: 0,
                net_minor: 0,
            })
            .add(m);
    }
    by.into_values().collect()
}

#[derive(Serialize)]
struct Years {
    /// Years with money in them, newest first.
    years: Vec<i32>,
    simulated: bool,
}

async fn years(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Years>> {
    let simulated = simulated(&state);
    let months = monthly(&state.db, user.id, None, simulated, None).await?;
    let mut years: Vec<i32> = months
        .iter()
        .filter_map(|m| m.period.get(..4)?.parse().ok())
        .collect();
    years.dedup();
    years.reverse();
    Ok(Json(Years { years, simulated }))
}

#[derive(Serialize)]
struct YearStatement {
    year: i32,
    simulated: bool,
    /// The year per currency and kind.
    totals: Vec<Totals>,
    /// Each month with money in it, per currency and kind, oldest first.
    months: Vec<Totals>,
    vat: VatTreatment,
}

/// `/me/statements/2026` as JSON, or `/me/statements/2026.csv` as a file.
async fn year(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(param): Path<String>,
) -> ApiResult<Response> {
    let (digits, csv) = match param.strip_suffix(".csv") {
        Some(d) => (d, true),
        None => (param.as_str(), false),
    };
    let year: i32 = digits
        .parse()
        .ok()
        .filter(|y| digits.len() == 4 && (2000..=9999).contains(y))
        .ok_or(ApiError::NotFound)?;
    let simulated = simulated(&state);
    let vat = vat_treatment(&state, &user).await?;
    let months = monthly(&state.db, user.id, Some(year), simulated, vat_rate(&vat)).await?;
    let totals = year_totals(year, &months);
    if !csv {
        return Ok(Json(YearStatement {
            year,
            simulated,
            totals,
            months,
            vat,
        })
        .into_response());
    }
    let body = to_csv(&months, &totals, vat.kind == "included");
    let name = if simulated {
        format!("slaporslop-statement-{year}-simulated.csv")
    } else {
        format!("slaporslop-statement-{year}.csv")
    };
    let mut res = body.into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    res.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
            .map_err(anyhow::Error::from)?,
    );
    Ok(res)
}

pub const CSV_HEADER: &str = "Period,Currency,Kind,Payments,Charged,Processing added by donors,\
                              Received,Refunds,Disputes,SLAPorSLOP fee,VAT in the fee,Fee without VAT,Net";

/// One header row, then one row per month, currency and kind, then the year's per currency and
/// kind. Plain numbers in the currency's units (12.34), no notes, so it opens as it is in a
/// spreadsheet. The two VAT columns are empty where SLAPorSLOP's fee includes none.
pub fn to_csv(months: &[Totals], totals: &[Totals], vat_included: bool) -> String {
    let amount = |minor: i64| {
        let sign = if minor < 0 { "-" } else { "" };
        format!("{sign}{}.{:02}", minor.abs() / 100, minor.abs() % 100)
    };
    let mut out = String::from(CSV_HEADER);
    out.push_str("\r\n");
    for t in months.iter().chain(totals) {
        let kind = match t.kind.as_str() {
            "donation" => "Donations",
            "bounty" => "Bounties",
            _ => "People jobs",
        };
        let (vat, ex_vat) = if vat_included {
            (amount(t.vat_minor), amount(t.slop_fee_minor - t.vat_minor))
        } else {
            (String::new(), String::new())
        };
        out.push_str(&format!(
            "{},{},{kind},{},{},{},{},{},{},{},{vat},{ex_vat},{}\r\n",
            t.period,
            t.currency.to_ascii_uppercase(),
            t.payments,
            amount(t.charged_minor),
            amount(t.processing_covered_minor),
            amount(t.gross_minor),
            amount(t.refunded_minor),
            amount(t.disputed_minor),
            amount(t.slop_fee_minor),
            amount(t.net_minor),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn month(period: &str, kind: &str, charged: i64, covered: i64, refunded: i64) -> Totals {
        let mut t = Totals {
            period: period.into(),
            currency: "usd".into(),
            kind: kind.into(),
            payments: 1,
            charged_minor: charged,
            processing_covered_minor: covered,
            gross_minor: 0,
            refunded_minor: refunded,
            disputed_minor: 0,
            slop_fee_minor: (charged - covered) / 20,
            vat_minor: 0,
            net_minor: 0,
        };
        t.finish(Some(1900));
        t
    }

    #[test]
    fn a_year_adds_up_its_months_per_currency_and_kind() {
        let months = [
            month("2026-01", "donation", 1040, 40, 0),
            month("2026-02", "donation", 2070, 70, 1040),
            month("2026-02", "people_job", 5000, 0, 0),
        ];
        let year = year_totals(2026, &months);
        assert_eq!(year.len(), 2);
        let d = &year[0];
        assert_eq!((d.period.as_str(), d.kind.as_str()), ("2026", "donation"));
        assert_eq!(d.payments, 2);
        assert_eq!(d.gross_minor, 3000);
        assert_eq!(d.slop_fee_minor, 150);
        assert_eq!(d.net_minor, 3000 - 1040 - 150);
        // The months' VAT added up: 50 → 8, 100 → 16.
        assert_eq!(d.vat_minor, 24);
    }

    #[test]
    fn the_csv_is_one_header_and_plain_rows() {
        let months = [month("2026-03", "bounty", 2000, 0, 2500)];
        let totals = year_totals(2026, &months);
        let csv = to_csv(&months, &totals, false);
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(lines[0], CSV_HEADER);
        assert_eq!(
            lines[1],
            "2026-03,USD,Bounties,1,20.00,0.00,20.00,25.00,0.00,1.00,,,-6.00"
        );
        assert_eq!(
            lines[2],
            "2026,USD,Bounties,1,20.00,0.00,20.00,25.00,0.00,1.00,,,-6.00"
        );
        assert_eq!(lines[3], "");
        assert!(to_csv(&months, &totals, true).contains(",1.00,0.16,0.84,-6.00"));
    }
}
