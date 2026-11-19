// SPDX-License-Identifier: AGPL-3.0-or-later
//! The maker's Stats tab (maker_stats.rs, DESIGN §8): every app they make and their account
//! overall, from the numbers already kept. Only their own apps, only their own token-free
//! session, and real or simulated donations as the server is.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers for this area

async fn exec(app: &TestApp, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(&app.db)
        .await
        .unwrap();
}

async fn stats(app: &TestApp, cookie: &str) -> Value {
    let res = app
        .send(req(Method::GET, "/v1/me/maker-stats").cookie(cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

fn app_row<'a>(stats: &'a Value, slug: &str) -> &'a Value {
    stats["apps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["slug"] == slug)
        .unwrap_or_else(|| panic!("no {slug} in {stats}"))
}

/// Library adds and first plays on `slug`, `back` UTC days ago.
async fn daily(app: &TestApp, slug: &str, back: i32, adds: i32, first_plays: i32) {
    sqlx::query(
        "insert into app_daily_counts (app_id, day, first_plays, library_adds)
         select id, (now() at time zone 'UTC')::date - $2, $3, $4 from apps where slug = $1",
    )
    .bind(slug)
    .bind(back)
    .bind(first_plays)
    .bind(adds)
    .execute(&app.db)
    .await
    .unwrap();
}

/// A vote on `slug` from an account that isn't otherwise in the test.
async fn vote(app: &TestApp, slug: &str, vote: i16) {
    sqlx::query(
        "insert into votes (user_id, app_id, vote, weight) select $1, id, $2, 1 from apps where slug = $3",
    )
    .bind(Uuid::now_v7())
    .bind(vote)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
}

/// A donation to `slug` with this status, paid `paid` (if it was).
async fn donation(
    app: &TestApp,
    slug: &str,
    donor: Uuid,
    (currency, minor): (&str, i32),
    status: &str,
    simulated: bool,
    paid: Option<OffsetDateTime>,
) {
    sqlx::query(
        "insert into donations (id, kind, app_id, donor_id, stripe_account_id, currency, donation_minor, slop_fee_minor,
                                processing_minor, total_minor, donor_covers_fees, status, simulated, paid_at)
         select $1, 'one_off', a.id, $2, 'acct_x', $3, $4, $4 / 20, 45, $4 + 45, true, $5, $6, $7
         from apps a where a.slug = $8",
    )
    .bind(Uuid::now_v7())
    .bind(donor)
    .bind(currency)
    .bind(minor)
    .bind(status)
    .bind(simulated)
    .bind(paid)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
}

/// Mia's apps, and Otto's one game with bigger numbers than all of hers.
async fn two_makers(app: &TestApp) -> (Uuid, String, Uuid, String) {
    let (mia_id, mia) = app.user("Mia Maker", "US").await;
    let (otto_id, otto) = app.user("Otto Other", "US").await;
    let since = "history_from = current_date - 60, players_from = current_date - 60";
    listed_with(app, mia_id, "arena", "released", since).await;
    listed_with(app, mia_id, "tower", "beta", since).await;
    listed_with(app, mia_id, "sketch", "coming_soon", "status = 'draft'").await;
    listed_with(app, mia_id, "secret", "released", "visibility = 'private'").await;
    listed_with(
        app,
        mia_id,
        "arena-server",
        "released",
        "platforms = '{linux}', branch_kind = 'server',
         parent_app_id = (select id from apps where slug = 'arena')",
    )
    .await;
    listed_with(app, otto_id, "otto-game", "released", since).await;
    (mia_id, mia, otto_id, otto)
}

// ---------------------------------------------------------------------------------------------
// Tests

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stats_add_up_per_app_and_for_the_account(db: PgPool) {
    let app = test_app(db).await;
    let (mia_id, mia, otto_id, _) = two_makers(&app).await;
    let (p1, _) = app.user("Pat Player", "US").await;
    let (p2, _) = app.user("Pia Player", "US").await;

    // Library adds: today, 3, 10 and 40 days ago on arena; yesterday on tower.
    daily(&app, "arena", 0, 2, 1).await;
    daily(&app, "arena", 3, 3, 2).await;
    daily(&app, "arena", 10, 4, 0).await;
    daily(&app, "arena", 40, 5, 9).await;
    daily(&app, "tower", 1, 1, 1).await;
    // Never counted for a private app (it can't be: this is only to show it isn't read).
    daily(&app, "secret", 0, 50, 50).await;
    daily(&app, "otto-game", 0, 100, 100).await;
    // Slop Pulse: within the 30 days and before them.
    exec(
        &app,
        "insert into pulse_counts (app_id, bucket, players)
         select id, now() - interval '1 hour', 7 from apps where slug = 'arena'
         union all select id, now() - interval '40 days', 99 from apps where slug = 'arena'
         union all select id, now() - interval '1 hour', 500 from apps where slug = 'otto-game'",
    )
    .await;
    // Votes: arena 4 Slaps and 1 Slop (80%); tower one Slaps, too few for a %.
    for v in [1, 1, 1, 1, -1] {
        vote(&app, "arena", v).await;
    }
    vote(&app, "tower", 1).await;
    for _ in 0..6 {
        vote(&app, "otto-game", -1).await;
    }
    // In libraries: two players and Mia herself, who isn't counted.
    for user in [p1, p2, mia_id] {
        sqlx::query("insert into library_entries (user_id, app_id) select $1, id from apps where slug = 'arena'")
            .bind(user)
            .execute(&app.db)
            .await
            .unwrap();
    }
    for user in [p1, p2] {
        sqlx::query(
            "insert into interests (user_id, app_id) select $1, id from apps where slug = 'sketch'",
        )
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Releases: two published (one revoked since), one still being checked.
    exec(
        &app,
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by)
         select gen_random_uuid(), a.id, v.version, repeat('a', 40), v.status::release_status, a.maker_id
         from apps a, (values ('1.0.0', 'revoked'), ('1.1.0', 'approved'), ('1.2.0', 'fetching')) v (version, status)
         where a.slug = 'arena'",
    )
    .await;
    exec(
        &app,
        "insert into bounties (id, app_id, created_by, title, body, currency, status)
         select gen_random_uuid(), id, maker_id, 'Fix it', 'Please', 'usd', s
         from apps, unnest(array['open', 'paid']) s where slug = 'arena'",
    )
    .await;
    exec(
        &app,
        "insert into app_reviews (id, user_id, app_id, body, hidden_at)
         select gen_random_uuid(), u.id, a.id, 'Good', case when u.display_name = 'Pia Player' then now() end
         from apps a, users u where a.slug = 'arena' and u.display_name in ('Pat Player', 'Pia Player')",
    )
    .await;
    // App of the Month: Mia once, with arena; Otto once.
    for (slug, maker, position) in [("arena", mia_id, 1), ("otto-game", otto_id, 2)] {
        sqlx::query(
            "insert into app_awards (month, app_id, maker_id, position, amount_minor, currency)
             select date_trunc('month', now())::date, id, $2, $3, 2500, 'usd' from apps where slug = $1",
        )
        .bind(slug)
        .bind(maker)
        .bind(position)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Donations: this year and last year, a refunded one, one never paid; a euro one to tower.
    let now = OffsetDateTime::now_utc();
    let last_year = time::Date::from_calendar_date(now.year() - 1, time::Month::June, 1)
        .unwrap()
        .midnight()
        .assume_utc();
    donation(&app, "arena", p1, ("usd", 1000), "paid", false, Some(now)).await;
    donation(
        &app,
        "arena",
        p2,
        ("usd", 500),
        "paid",
        false,
        Some(last_year),
    )
    .await;
    donation(
        &app,
        "arena",
        p2,
        ("usd", 300),
        "refunded",
        false,
        Some(now),
    )
    .await;
    exec(
        &app,
        "update donations set refunded_minor = 300 where status = 'refunded'",
    )
    .await;
    donation(&app, "arena", p1, ("usd", 4000), "pending", false, None).await;
    donation(&app, "tower", p1, ("eur", 700), "paid", false, Some(now)).await;
    donation(
        &app,
        "otto-game",
        p1,
        ("usd", 9999),
        "paid",
        false,
        Some(now),
    )
    .await;

    let s = stats(&app, &mia).await;
    assert_eq!(s["simulated"], false);
    assert_eq!(s["days"].as_array().unwrap().len(), 30);

    let arena = app_row(&s, "arena");
    assert_eq!(arena["counted"], true);
    assert_eq!(
        [
            &arena["library_adds"],
            &arena["library_adds_7d"],
            &arena["library_adds_30d"],
            &arena["first_plays_30d"],
        ],
        [&json!(14), &json!(5), &json!(9), &json!(3)]
    );
    let series = arena["daily_library_adds"].as_array().unwrap();
    assert_eq!(series.len(), 30);
    assert_eq!(series[29], 2);
    assert_eq!(series[26], 3);
    assert_eq!(series[19], 4);
    assert_eq!(series.iter().map(|v| v.as_i64().unwrap()).sum::<i64>(), 9);
    assert_eq!(arena["peak_players_30d"], 7);
    assert_eq!(arena["in_libraries"], 2);
    assert_eq!((&arena["not_slop"], &arena["slop"]), (&json!(4), &json!(1)));
    assert_eq!(arena["not_slop_percent"], 80);
    assert_eq!(arena["releases_published"], 2);
    assert_eq!(arena["open_bounties"], 1);
    assert_eq!(arena["reviews"], 1);
    assert_eq!(
        arena["donations"],
        json!([{ "currency": "usd", "donations": 3, "donated_minor": 1800, "refunded_minor": 300 }])
    );
    assert_eq!(arena["awards"].as_array().unwrap().len(), 1);
    assert_eq!(arena["awards"][0]["position"], 1);

    let tower = app_row(&s, "tower");
    assert_eq!(tower["not_slop_percent"], Value::Null, "below MIN_VOTES");
    assert_eq!(tower["library_adds_30d"], 1);
    assert_eq!(tower["peak_players_30d"], Value::Null, "no Pulse counts");
    assert_eq!(app_row(&s, "sketch")["interested"], 2);
    assert_eq!(app_row(&s, "sketch")["status"], "draft");
    // Nothing is counted for a private app or a dedicated server.
    for slug in ["secret", "arena-server"] {
        let a = app_row(&s, slug);
        assert_eq!(a["counted"], false, "{slug}");
        assert_eq!(a["library_adds"], Value::Null, "{slug}");
        assert_eq!(a["daily_library_adds"], json!([]), "{slug}");
    }
    assert_eq!(app_row(&s, "arena-server")["server_for"], "arena");

    let acct = &s["account"];
    assert_eq!(acct["apps"], 5);
    assert_eq!(
        acct["counts"],
        json!({ "draft": 1, "listed": 4, "delisted": 0, "coming_soon": 1, "beta": 1, "released": 3, "private": 1 })
    );
    assert_eq!(
        (&acct["library_adds"], &acct["library_adds_30d"]),
        (&json!(15), &json!(10))
    );
    assert_eq!(acct["first_plays_30d"], 4);
    assert_eq!((&acct["not_slop"], &acct["slop"]), (&json!(5), &json!(1)));
    // Weighted over all six votes: 5 / 6.
    assert_eq!(acct["not_slop_percent"], 83);
    assert_eq!(acct["interested"], 2);
    assert_eq!(acct["awards"], 1);
    assert_eq!(acct["releases_published"], 2);
    assert_eq!(
        acct["donations"],
        json!([
            { "currency": "eur", "donations": 1, "donated_minor": 700, "refunded_minor": 0 },
            { "currency": "usd", "donations": 3, "donated_minor": 1800, "refunded_minor": 300 },
        ])
    );
    assert_eq!(
        acct["donations_this_year"],
        json!([
            { "currency": "eur", "donations": 1, "donated_minor": 700, "refunded_minor": 0 },
            { "currency": "usd", "donations": 2, "donated_minor": 1300, "refunded_minor": 300 },
        ])
    );
    assert_eq!(acct["busiest"]["slug"], "arena");
    assert_eq!(acct["busiest"]["library_adds_30d"], 9);

    // Never a donor or a player: counts and totals only.
    let text = s.to_string();
    for name in ["Pat Player", "Pia Player", &p1.to_string(), "otto-game"] {
        assert!(!text.contains(name), "{name} in {text}");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn another_makers_apps_are_never_in_them(db: PgPool) {
    let app = test_app(db).await;
    let (_, mia, _, otto) = two_makers(&app).await;
    daily(&app, "arena", 0, 3, 1).await;
    daily(&app, "otto-game", 0, 40, 4).await;
    let s = stats(&app, &otto).await;
    let slugs: Vec<&str> = s["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, ["otto-game"]);
    assert_eq!(s["account"]["library_adds"], 40);
    assert_eq!(s["account"]["busiest"]["slug"], "otto-game");
    let s = stats(&app, &mia).await;
    assert_eq!(s["account"]["library_adds"], 3);
    assert!(!s.to_string().contains("otto-game"));
}

/// A branch of someone else's game is its own maker's, as on Manage.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_branch_counts_for_its_own_maker(db: PgPool) {
    let app = test_app(db).await;
    let (_, mia, otto_id, otto) = two_makers(&app).await;
    listed_with(
        &app,
        otto_id,
        "arena-mod",
        "released",
        "branch_kind = 'mod', parent_app_id = (select id from apps where slug = 'arena')",
    )
    .await;
    let s = stats(&app, &otto).await;
    assert_eq!(app_row(&s, "arena-mod")["branch_of"], "arena");
    let s = stats(&app, &mia).await;
    assert!(!s.to_string().contains("arena-mod"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_account_without_apps_gets_empty_stats(db: PgPool) {
    let app = test_app(db).await;
    let (_, eve) = app.user("Eve Empty", "US").await;
    let s = stats(&app, &eve).await;
    assert_eq!(s["apps"], json!([]));
    let acct = &s["account"];
    assert_eq!(acct["apps"], 0);
    assert_eq!(acct["library_adds"], 0);
    assert_eq!(acct["not_slop_percent"], Value::Null);
    assert_eq!(acct["donations"], json!([]));
    assert_eq!(acct["busiest"], Value::Null);
    // Signed out: nothing.
    let res = app.send(req(Method::GET, "/v1/me/maker-stats")).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

/// Real or simulated, never both, as on the yearly statements.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn donations_are_real_or_simulated_as_the_server_is(db: PgPool) {
    let real = test_app(db.clone()).await;
    let (_, mia, _, _) = two_makers(&real).await;
    let (donor, _) = real.user("Dee Donor", "US").await;
    let now = Some(OffsetDateTime::now_utc());
    donation(&real, "arena", donor, ("usd", 1000), "paid", false, now).await;
    donation(&real, "arena", donor, ("usd", 9000), "paid", true, now).await;
    let s = stats(&real, &mia).await;
    assert_eq!(s["simulated"], false);
    assert_eq!(s["account"]["donations"][0]["donated_minor"], 1000);
    assert_eq!(app_row(&s, "arena")["donations"][0]["donations"], 1);

    let sim = test_app_with(db, |c| c.payments.simulated = true).await;
    let s = stats(&sim, &mia).await;
    assert_eq!(s["simulated"], true);
    assert_eq!(s["account"]["donations"][0]["donated_minor"], 9000);
    assert_eq!(
        s["account"]["donations_this_year"][0]["donated_minor"],
        9000
    );
}

/// Money is the person's: an AI agent's token can't read the Stats tab.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agent_tokens_cant_read_maker_stats(db: PgPool) {
    let app = test_app(db).await;
    let (_, mia, _, _) = two_makers(&app).await;
    for scope in ["read", "publish"] {
        let res = app
            .send(
                req(Method::POST, "/v1/me/maker-tokens")
                    .cookie(&mia)
                    .json(json!({ "name": "Claude", "scope": scope })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        let token = format!(
            "Bearer {}",
            body_json(res).await["secret"].as_str().unwrap()
        );
        let res = app
            .send(req(Method::GET, "/v1/me/maker-stats").header("authorization", token))
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{scope}");
    }
}

/// "stats" (like "new" and "guide") is the website's page under /publish/, so no app can have it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stats_is_not_an_app_address(db: PgPool) {
    let app = test_app(db).await;
    let (_, mia) = app.user("Mia Maker", "US").await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&mia)
                .json(new_app("stats")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "slug");
    let res = app
        .send(req(Method::GET, "/v1/apps/stats/available").cookie(&mia))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let check = body_json(res).await;
    assert_eq!(check["available"], false);
    assert_eq!(check["suggestion"], "stats-2");
}
