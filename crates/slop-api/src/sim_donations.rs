//! The sandbox's donations (donations.rs): about a third of the makers turn donations on, some
//! apps declare servers, and players who like an app donate now and then over the last 60
//! days. Amounts go through the real `donations::quote`, so the report can check the fee is
//! exactly 5% of every donation and the totals add up.

use serde::Serialize;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{donations, sim::Rng};

#[derive(Debug, Default, Serialize)]
pub struct DonationsGenerated {
    pub apps_taking_donations: usize,
    pub donations: usize,
    pub server_donations: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct DonationsReport {
    pub donations: i64,
    pub donated_minor: i64,
    pub slop_fees_minor: i64,
    /// Should be 0: every donation's fee is exactly 5% of it.
    pub fees_not_5_percent: i64,
    /// Should be 0: every paid donation has its payment record.
    pub paid_without_payment: i64,
    pub active_server_donations: i64,
}

pub async fn write(
    db: &PgPool,
    rng: &mut Rng,
    now: OffsetDateTime,
) -> anyhow::Result<DonationsGenerated> {
    let mut out = DonationsGenerated::default();
    let apps: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "select a.id, a.maker_id, a.kind::text from apps a where a.status = 'listed' and a.stage <> 'coming_soon'
         order by md5(a.id::text)",
    )
    .fetch_all(db)
    .await?;
    let rate = donations::Rate {
        percent_bp: 290,
        fixed_minor: 30,
    };
    for (i, (app, maker, kind)) in apps.iter().enumerate() {
        if i % 3 != 0 {
            continue;
        }
        let acct = format!("acct_sim_{}", maker.simple());
        sqlx::query(
            "insert into donation_accounts (user_id, stripe_account_id, stripe_status, tax_notice_ack_at)
             values ($1, $2, 'active', now()) on conflict do nothing",
        )
        .bind(maker)
        .bind(&acct)
        .execute(db)
        .await?;
        let servers = kind == "game" && rng.chance(0.3);
        let suggested = *rng.pick(&[200i32, 300, 500, 500, 1000]);
        sqlx::query(
            "update apps set donations_on = true, donation_suggested_minor = $2, runs_servers = $3,
                    servers_statement = case when $3 then 'Pays for the multiplayer servers and the leaderboard database.' end
             where id = $1",
        )
        .bind(app)
        .bind(suggested)
        .bind(servers)
        .execute(db)
        .await?;
        out.apps_taking_donations += 1;
        // Players who voted 👍 sometimes donate.
        let fans: Vec<Uuid> = sqlx::query_scalar(
            "select user_id from votes where app_id = $1 and vote = 1 order by md5(user_id::text) limit 60",
        )
        .bind(app)
        .fetch_all(db)
        .await?;
        for fan in fans {
            if !rng.chance(0.25) {
                continue;
            }
            let amount = i64::from(suggested) * *rng.pick(&[1i64, 1, 1, 2, 5])
                / if rng.chance(0.3) { 2 } else { 1 };
            let amount = amount.clamp(donations::MIN_MINOR, donations::MAX_MINOR);
            let q = donations::quote(amount, rate, true);
            if servers && rng.chance(0.3) {
                let started = now - Duration::days(1 + rng.below(80) as i64);
                let sid = Uuid::now_v7();
                let consent =
                    donations::consent_text(&q, "usd", "this app", "its maker", started.date());
                sqlx::query(
                    "insert into server_donations (id, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                                   slop_fee_minor, total_minor, consent_text, status, simulated,
                                                   created_at, started_at, next_charge_at)
                     values ($1, $2, $3, $4, 'usd', $5, $6, $7, $8, 'active', true, $9, $9, $9 + interval '30 days')
                     on conflict do nothing",
                )
                .bind(sid)
                .bind(app)
                .bind(fan)
                .bind(&acct)
                .bind(q.donation as i32)
                .bind(q.slop_fee as i32)
                .bind(q.total as i32)
                .bind(&consent)
                .bind(started)
                .execute(db)
                .await?;
                out.server_donations += 1;
                // One paid month for each 30 days so far.
                let mut at = started;
                while at < now {
                    insert_paid(db, *app, fan, Some(sid), &acct, &q, at, false).await?;
                    out.donations += 1;
                    at += Duration::days(30);
                }
                sqlx::query("update server_donations set next_charge_at = $2 where id = $1")
                    .bind(sid)
                    .bind(at)
                    .execute(db)
                    .await?;
            } else {
                let at = now - Duration::hours(1 + rng.below(24 * 60) as i64);
                insert_paid(db, *app, fan, None, &acct, &q, at, rng.chance(0.2)).await?;
                out.donations += 1;
            }
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
async fn insert_paid(
    db: &PgPool,
    app: Uuid,
    donor: Uuid,
    server: Option<Uuid>,
    acct: &str,
    q: &donations::Quote,
    at: OffsetDateTime,
    share: bool,
) -> anyhow::Result<()> {
    let id = Uuid::now_v7();
    let object = format!("sim_{}", id.simple());
    sqlx::query(
        "insert into donations (id, kind, app_id, donor_id, server_donation_id, stripe_account_id, currency,
                                donation_minor, slop_fee_minor, processing_minor, total_minor, donor_covers_fees,
                                share_name, message, status, stripe_object_id, simulated, created_at, paid_at)
         values ($1, $2, $3, $4, $5, $6, 'usd', $7, $8, $9, $10, true, $11, $12, 'paid', $13, true, $14, $14)",
    )
    .bind(id)
    .bind(if server.is_some() { "monthly" } else { "one_off" })
    .bind(app)
    .bind(donor)
    .bind(server)
    .bind(acct)
    .bind(q.donation as i32)
    .bind(q.slop_fee as i32)
    .bind(q.processing as i32)
    .bind(q.total as i32)
    .bind(share)
    .bind(share.then_some("Thanks for making this. Keep going!"))
    .bind(&object)
    .bind(at)
    .execute(db)
    .await?;
    sqlx::query(
        "insert into payments (id, kind, donation_id, stripe_account_id, stripe_object_id, payer_id, amount_minor,
                               slop_fee_minor, currency, simulated, created_at)
         values ($1, 'donation', $2, $3, $4, $5, $6, $7, 'usd', true, $8)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(acct)
    .bind(&object)
    .bind(donor)
    .bind(q.total as i32)
    .bind(q.slop_fee as i32)
    .bind(at)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn report(db: &PgPool) -> anyhow::Result<DonationsReport> {
    let (donations, donated, fees): (i64, i64, i64) = sqlx::query_as(
        "select count(*), coalesce(sum(donation_minor), 0)::int8, coalesce(sum(slop_fee_minor), 0)::int8
         from donations where status <> 'pending'",
    )
    .fetch_one(db)
    .await?;
    Ok(DonationsReport {
        donations,
        donated_minor: donated,
        slop_fees_minor: fees,
        fees_not_5_percent: sqlx::query_scalar(
            "select count(*) from donations where slop_fee_minor <> (donation_minor * 500 + 5000) / 10000",
        )
        .fetch_one(db)
        .await?,
        paid_without_payment: sqlx::query_scalar(
            "select count(*) from donations d where d.status = 'paid'
               and not exists (select 1 from payments p where p.donation_id = d.id and p.kind = 'donation')",
        )
        .fetch_one(db)
        .await?,
        active_server_donations: sqlx::query_scalar("select count(*) from server_donations where status = 'active'")
            .fetch_one(db)
            .await?,
    })
}
