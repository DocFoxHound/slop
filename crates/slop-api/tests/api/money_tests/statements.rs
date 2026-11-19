// SPDX-License-Identifier: AGPL-3.0-or-later
//! Yearly money statements (statements.rs): totals of what reached the user's own Stripe
//! accounts, with refunds, disputes and SLAPorSLOP's fee, and the CSV file.

use super::*;

/// A paid one-off donation to `slug` and its payment record, as `donation_paid` records them.
#[allow(clippy::too_many_arguments)]
async fn paid_donation(
    app: &TestApp,
    slug: &str,
    donor: Uuid,
    account: &str,
    charge: &str,
    (currency, donation, total): (&str, i32, i32),
    covers: bool,
    simulated: bool,
    at: OffsetDateTime,
) {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into donations (id, kind, app_id, donor_id, stripe_account_id, currency, donation_minor, slop_fee_minor,
                                processing_minor, total_minor, donor_covers_fees, status, stripe_object_id, simulated,
                                created_at, paid_at)
         select $1, 'one_off', a.id, $2, $3, $4, $5, $5 / 20, case when $7 then $6 - $5 else 45 end, $6, $7, 'paid', $8,
                $9, $10, $10
         from apps a where a.slug = $11",
    )
    .bind(id)
    .bind(donor)
    .bind(account)
    .bind(currency)
    .bind(donation)
    .bind(total)
    .bind(covers)
    .bind(charge)
    .bind(simulated)
    .bind(at)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into payments (id, kind, donation_id, stripe_account_id, stripe_object_id, payer_id, amount_minor,
                               slop_fee_minor, currency, simulated, created_at)
         values ($1, 'donation', $2, $3, $4, $5, $6, $7 / 20, $8, $9, $10)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(account)
    .bind(charge)
    .bind(donor)
    .bind(total)
    .bind(donation)
    .bind(currency)
    .bind(simulated)
    .bind(at)
    .execute(&app.db)
    .await
    .unwrap();
}

/// A paid People job for `creative` (whose page names `account`), with its payment record.
async fn paid_job(
    app: &TestApp,
    creative: Uuid,
    client: Uuid,
    account: &str,
    charge: &str,
    amount: i32,
) {
    sqlx::query(
        "insert into creatives (user_id, handle, name, ai_use, trader, stripe_account_id, stripe_status)
         values ($1, 'mia-makes', 'Mia', 'none', false, $2, 'active') on conflict (user_id) do nothing",
    )
    .bind(creative)
    .bind(account)
    .execute(&app.db)
    .await
    .unwrap();
    let (job, part) = (Uuid::now_v7(), Uuid::now_v7());
    sqlx::query(
        "insert into jobs (id, creative_id, client_id, app_id, skill, status, request_md)
         select $1, $2, $3, id, 'art', 'completed', 'Draw a cover.' from apps limit 1",
    )
    .bind(job)
    .bind(creative)
    .bind(client)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into job_payments (id, job_id, part, due_on, amount_minor, currency, status, paid_at)
         values ($1, $2, 'full', 'accept', $3, 'usd', 'paid', now())",
    )
    .bind(part)
    .bind(job)
    .bind(amount)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into payments (id, kind, job_payment_id, stripe_account_id, stripe_object_id, payer_id, amount_minor,
                               slop_fee_minor, currency)
         values ($1, 'people_job', $2, $3, $4, $5, $6, $6 / 20, 'usd')",
    )
    .bind(Uuid::now_v7())
    .bind(part)
    .bind(account)
    .bind(charge)
    .bind(client)
    .bind(amount)
    .execute(&app.db)
    .await
    .unwrap();
}

/// The row for this currency and kind.
fn row<'a>(rows: &'a Value, currency: &str, kind: &str) -> &'a Value {
    rows.as_array()
        .unwrap()
        .iter()
        .find(|r| r["currency"] == currency && r["kind"] == kind)
        .unwrap_or_else(|| panic!("no {currency} {kind} in {rows}"))
}

fn amounts(r: &Value) -> [i64; 8] {
    [
        "payments",
        "charged_minor",
        "processing_covered_minor",
        "gross_minor",
        "refunded_minor",
        "disputed_minor",
        "slop_fee_minor",
        "net_minor",
    ]
    .map(|k| r[k].as_i64().unwrap_or_else(|| panic!("{k} in {r}")))
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn yearly_statements_total_each_currency_and_kind_with_refunds_and_disputes(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (mia_id, mia) = app.user("Mia Maker", "US").await;
    let (otto_id, otto) = app.user("Otto Other", "US").await;
    let (donor, _) = app.user("Dee Donor", "US").await;
    listed_with(&app, mia_id, "arena", "released", "").await;
    listed_with(&app, otto_id, "otto-game", "released", "").await;
    for (user, acct) in [(mia_id, "acct_mia"), (otto_id, "acct_otto")] {
        sqlx::query(
            "insert into donation_accounts (user_id, stripe_account_id, stripe_status, tax_notice_ack_at)
             values ($1, $2, 'active', now())",
        )
        .bind(user)
        .bind(acct)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let now = OffsetDateTime::now_utc();
    let this_year = now.year();
    let last_year = time::Date::from_calendar_date(this_year - 1, time::Month::June, 1)
        .unwrap()
        .midnight()
        .assume_utc();
    let keys = |k: &str| vec![k.to_string()];

    // Dollars: two donations whose donors covered the processing, one refunded in two parts.
    paid_donation(
        &app,
        "arena",
        donor,
        "acct_mia",
        "ch_1",
        ("usd", 1000, 1059),
        true,
        false,
        now,
    )
    .await;
    paid_donation(
        &app,
        "arena",
        donor,
        "acct_mia",
        "ch_2",
        ("usd", 2000, 2088),
        true,
        false,
        now,
    )
    .await;
    slop_api::donations::refund_recorded(&app.db, "ch_2", &keys("ch_2"), 500)
        .await
        .unwrap();
    slop_api::donations::refund_recorded(&app.db, "ch_2", &keys("ch_2"), 800)
        .await
        .unwrap();
    // Euros: where the donor doesn't cover the processing, then a dispute.
    paid_donation(
        &app,
        "arena",
        donor,
        "acct_mia",
        "ch_3",
        ("eur", 500, 500),
        false,
        false,
        now,
    )
    .await;
    slop_api::donations::dispute_recorded(&app.db, &keys("ch_3"), "dp_1", 500)
        .await
        .unwrap();
    // A simulated donation never counts on a server taking real payments.
    paid_donation(
        &app,
        "arena",
        donor,
        "acct_mia",
        "sim_1",
        ("usd", 9000, 9300),
        true,
        true,
        now,
    )
    .await;
    // Pounds: a bounty payout, refunded in full.
    sqlx::query(
        "insert into payments (id, kind, stripe_account_id, stripe_object_id, amount_minor, slop_fee_minor, currency)
         values ($1, 'bounty', 'acct_mia', 'pi_b1', 2000, 100, 'gbp')",
    )
    .bind(Uuid::now_v7())
    .execute(&app.db)
    .await
    .unwrap();
    slop_api::bounties::refund_recorded(&app.db, "ch_b1", &keys("pi_b1"), 2000)
        .await
        .unwrap();
    // A People job: a refund, and a dispute (recorded without the account, through the payment).
    paid_job(&app, mia_id, donor, "acct_mia", "ch_job", 12000).await;
    slop_api::people::jobs::refund_recorded(&app.db, "ch_job", 2000)
        .await
        .unwrap();
    slop_api::people::jobs::dispute_recorded(&app.db, "ch_job", "dp_job", 3000)
        .await
        .unwrap();
    // Last year, and someone else's money.
    paid_donation(
        &app,
        "arena",
        donor,
        "acct_mia",
        "ch_old",
        ("usd", 300, 340),
        true,
        false,
        last_year,
    )
    .await;
    paid_donation(
        &app,
        "otto-game",
        donor,
        "acct_otto",
        "ch_otto",
        ("usd", 7000, 7233),
        true,
        false,
        now,
    )
    .await;

    let years = get_json(&app, &mia, "/v1/me/statements").await;
    assert_eq!(years["years"], json!([this_year, this_year - 1]), "{years}");
    assert_eq!(years["simulated"], false);

    let s = get_json(&app, &mia, &format!("/v1/me/statements/{this_year}")).await;
    assert_eq!(s["year"], this_year);
    let t = &s["totals"];
    assert_eq!(t.as_array().unwrap().len(), 4, "{t}");
    // payments, charged, processing covered, gross, refunded, disputed, fee, net
    assert_eq!(
        amounts(row(t, "usd", "donation")),
        [2, 3147, 147, 3000, 800, 0, 150, 3000 - 800 - 150]
    );
    assert_eq!(
        amounts(row(t, "eur", "donation")),
        [1, 500, 0, 500, 0, 500, 25, -25]
    );
    assert_eq!(
        amounts(row(t, "gbp", "bounty")),
        [1, 2000, 0, 2000, 2000, 0, 100, -100]
    );
    assert_eq!(
        amounts(row(t, "usd", "people_job")),
        [1, 12000, 0, 12000, 2000, 3000, 600, 6400]
    );
    assert_eq!(row(t, "usd", "donation")["period"], this_year.to_string());
    // All of it in this month.
    let month = format!("{this_year}-{:02}", u8::from(now.month()));
    let months = s["months"].as_array().unwrap();
    assert_eq!(months.len(), 4);
    assert!(months.iter().all(|m| m["period"] == month.as_str()), "{s}");
    assert_eq!(s["vat"]["kind"], "none");

    let old = get_json(&app, &mia, &format!("/v1/me/statements/{}", this_year - 1)).await;
    assert_eq!(
        amounts(row(&old["totals"], "usd", "donation")),
        [1, 340, 40, 300, 0, 0, 15, 285]
    );

    // Otto sees only his own; a year without money is empty, not an error.
    let o = get_json(&app, &otto, &format!("/v1/me/statements/{this_year}")).await;
    assert_eq!(o["totals"].as_array().unwrap().len(), 1, "{o}");
    assert_eq!(amounts(row(&o["totals"], "usd", "donation"))[3], 7000);
    let o = get_json(&app, &otto, &format!("/v1/me/statements/{}", this_year - 1)).await;
    assert_eq!(o["totals"], json!([]));
    assert_eq!(o["months"], json!([]));

    // The monthly fee statements use the same totals, bounties and refunds included.
    let fees = get_json(&app, &mia, "/v1/me/fee-statements").await;
    let current: Vec<&Value> = fees["statements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["month"] == month.as_str())
        .collect();
    assert_eq!(current.len(), 4, "{fees}");
    let usd = current
        .iter()
        .find(|r| r["kind"] == "donation" && r["currency"] == "usd")
        .unwrap();
    assert_eq!(usd["received_minor"], 3000);
    assert_eq!(usd["refunded_minor"], 800);
    assert_eq!(
        fees["statements"][0]["month"],
        month.as_str(),
        "newest first"
    );

    // Not years that can't be, and never without signing in.
    for bad in ["26", "abcd", "1999", "20260", "2026.pdf"] {
        let res = app
            .send(req(Method::GET, format!("/v1/me/statements/{bad}")).cookie(&mia))
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{bad}");
    }
    for path in [
        "/v1/me/statements".to_string(),
        format!("/v1/me/statements/{this_year}.csv"),
    ] {
        assert_eq!(
            app.send(req(Method::GET, path)).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_csv_has_one_header_and_the_same_totals(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = app.user("Mia Maker", "DE").await;
    let (_, donor) = adult(&app, "Dee Donor", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    donations_on(&app, &maker, "arena", false).await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/arena/donate")
                .cookie(&donor)
                .json(json!({ "amount_minor": 2000, "confirm_adult": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let year = OffsetDateTime::now_utc().year();
    let res = app
        .send(req(Method::GET, format!("/v1/me/statements/{year}.csv")).cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()[header::CONTENT_TYPE],
        "text/csv; charset=utf-8"
    );
    // A simulating server's statement says so in its name.
    assert_eq!(
        res.headers()[header::CONTENT_DISPOSITION],
        format!("attachment; filename=\"slaporslop-statement-{year}-simulated.csv\"").as_str()
    );
    let body =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    let lines: Vec<&str> = body.split("\r\n").filter(|l| !l.is_empty()).collect();
    assert_eq!(lines[0], slop_api::statements::CSV_HEADER);
    assert_eq!(lines.len(), 3, "a month and the year: {body}");
    let json = get_json(&app, &maker, &format!("/v1/me/statements/{year}")).await;
    assert_eq!(json["simulated"], true);
    let t = row(&json["totals"], "usd", "donation");
    let total = t["charged_minor"].as_i64().unwrap();
    let covered = total - 2000;
    // Germany: 19% VAT inside the 1.00 fee is 0.16.
    assert_eq!(t["vat_minor"], 16);
    let tail = format!(
        "USD,Donations,1,{}.{:02},{}.{:02},20.00,0.00,0.00,1.00,0.16,19.00",
        total / 100,
        total % 100,
        covered / 100,
        covered % 100
    );
    assert_eq!(lines[1].split_once(',').unwrap().1, tail, "{body}");
    assert_eq!(lines[2], format!("{year},{tail}"));
    for line in &lines {
        assert_eq!(line.split(',').count(), 12, "{line}");
    }
    // Nobody else's donation shows in the file: a donor's own statement is empty.
    let res = app
        .send(req(Method::GET, format!("/v1/me/statements/{year}.csv")).cookie(&donor))
        .await;
    let body =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert_eq!(body, format!("{}\r\n", slop_api::statements::CSV_HEADER));
}

/// A maker's history stays after they disconnect Stripe, and after they come back with a new
/// account and then switch to another.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn statements_keep_money_from_accounts_disconnected_or_switched_away_from(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, donor) = adult(&app, "Dee Donor", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let (app, maker, donor) = (&app, &maker, &donor);
    let donate = move || async move {
        let res = app
            .send(
                req(Method::POST, "/v1/apps/arena/donate")
                    .cookie(donor)
                    .json(json!({ "amount_minor": 1000, "confirm_adult": true })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    };
    let change = move |kind: &'static str| async move {
        let res = app
            .send(
                req(Method::POST, "/v1/me/donation-account/change")
                    .cookie(maker)
                    .json(json!({ "kind": kind })),
            )
            .await;
        assert!(res.status().is_success(), "{kind}");
        sqlx::query(
            "update stripe_account_changes set effective_at = now() where applied_at is null",
        )
        .execute(&app.db)
        .await
        .unwrap();
        slop_api::donations::apply_account_changes(&app.state)
            .await
            .unwrap();
    };
    let year = OffsetDateTime::now_utc().year();
    let paid = move || async move {
        let s = get_json(app, maker, &format!("/v1/me/statements/{year}")).await;
        let t = row(&s["totals"], "usd", "donation");
        (
            t["payments"].as_i64().unwrap(),
            t["gross_minor"].as_i64().unwrap(),
        )
    };

    donations_on(app, maker, "arena", false).await;
    donate().await;
    change("disconnect").await;
    let left: i64 = sqlx::query_scalar("select count(*) from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(left, 0, "disconnected");
    assert_eq!(paid().await, (1, 1000));

    // Back with a new account, then a switch to a third.
    donations_on(app, maker, "arena", false).await;
    donate().await;
    change("switch").await;
    donate().await;
    let accounts: i64 = sqlx::query_scalar(
        "select count(distinct stripe_account_id) from payments where kind = 'donation'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(accounts, 3);
    assert_eq!(paid().await, (3, 3000));
    let years = get_json(app, maker, "/v1/me/statements").await;
    assert_eq!(years["years"], json!([year]));
    assert_eq!(years["simulated"], true);

    // The download of their data has the same totals and the accounts.
    let export = get_json(app, maker, "/v1/me/export").await;
    assert_eq!(
        export["stripe_accounts"].as_array().unwrap().len(),
        3,
        "{}",
        export["stripe_accounts"]
    );
    assert_eq!(
        row(&export["money_received"], "usd", "donation")["payments"],
        3
    );
}

/// An account is the first owner's: another user's row naming it later doesn't bring its money
/// to their statement.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn another_users_money_never_shows(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (mia_id, mia) = app.user("Mia Maker", "US").await;
    let (eve_id, eve) = app.user("Eve Else", "US").await;
    let (donor, _) = app.user("Dee Donor", "US").await;
    listed_with(&app, mia_id, "arena", "released", "").await;
    sqlx::query(
        "insert into donation_accounts (user_id, stripe_account_id, stripe_status, tax_notice_ack_at)
         values ($1, 'acct_mia', 'active', now())",
    )
    .bind(mia_id)
    .execute(&app.db)
    .await
    .unwrap();
    let now = OffsetDateTime::now_utc();
    paid_donation(
        &app,
        "arena",
        donor,
        "acct_mia",
        "ch_1",
        ("usd", 1000, 1059),
        true,
        false,
        now,
    )
    .await;
    sqlx::query(
        "insert into creatives (user_id, handle, name, ai_use, trader, stripe_account_id)
         values ($1, 'eve-else', 'Eve', 'none', false, 'acct_mia')",
    )
    .bind(eve_id)
    .execute(&app.db)
    .await
    .unwrap();
    let year = now.year();
    let s = get_json(&app, &eve, &format!("/v1/me/statements/{year}")).await;
    assert_eq!(s["totals"], json!([]), "{s}");
    assert_eq!(
        get_json(&app, &eve, "/v1/me/statements").await["years"],
        json!([])
    );
    let fees = get_json(&app, &eve, "/v1/me/fee-statements").await;
    assert_eq!(fees["statements"], json!([]));
    let export = get_json(&app, &eve, "/v1/me/export").await;
    assert_eq!(export["money_received"], json!([]));
    let s = get_json(&app, &mia, &format!("/v1/me/statements/{year}")).await;
    assert_eq!(s["totals"].as_array().unwrap().len(), 1);
}

/// Money is the person's: an AI agent's token can't read statements or download them.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agent_tokens_cant_read_statements(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, maker) = maker_taking_donations(&app, "arena", false).await;
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&maker)
                .json(json!({ "name": "Claude", "scope": "publish" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let token = format!(
        "Bearer {}",
        body_json(res).await["secret"].as_str().unwrap()
    );
    let year = OffsetDateTime::now_utc().year();
    for path in [
        "/v1/me/statements".to_string(),
        format!("/v1/me/statements/{year}"),
        format!("/v1/me/statements/{year}.csv"),
        "/v1/me/fee-statements".to_string(),
    ] {
        let res = app
            .send(req(Method::GET, path.clone()).header("authorization", token.clone()))
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{path}");
    }
}
