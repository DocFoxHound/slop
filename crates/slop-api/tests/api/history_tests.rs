// SPDX-License-Identifier: AGPL-3.0-or-later
//! History for makers (history.rs, migration 0095, DESIGN §11): what a launch and a library add
//! count, what is never counted (a private app, a dedicated server, the maker's own account, a
//! launch with "Share my playtime" off), the maker's read of it, and the back-fill.

use crate::*;

use slop_api::limits::Action;

// ---------------------------------------------------------------------------------------------
// Helpers for this area

/// The daily totals of `slug`, oldest first: (day, players, first plays, library adds).
async fn counts(app: &TestApp, slug: &str) -> Vec<(String, i32, i32, i32)> {
    sqlx::query_as(
        "select to_char(c.day, 'YYYY-MM-DD'), c.players, c.first_plays, c.library_adds
         from app_daily_counts c join apps a on a.id = c.app_id
         where a.slug = $1 order by c.day",
    )
    .bind(slug)
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// The maker's history of `slug` (`query` is "" or "?days=…").
async fn history(app: &TestApp, cookie: &str, slug: &str, query: &str) -> Value {
    let res = app
        .send(req(Method::GET, format!("/v1/apps/{slug}/history{query}")).cookie(cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

/// The UTC day `back` days ago, as the API writes it.
async fn day(app: &TestApp, back: i32) -> String {
    sqlx::query_scalar("select to_char((now() at time zone 'UTC')::date - $1, 'YYYY-MM-DD')")
        .bind(back)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn launch(app: &TestApp, cookie: &str, slug: &str, query: &str) -> Response {
    app.send(req(Method::POST, format!("/v1/apps/{slug}/launch{query}")).cookie(cookie))
        .await
}

async fn add(app: &TestApp, cookie: &str, slug: &str) -> StatusCode {
    app.send(req(Method::PUT, format!("/v1/me/library/{slug}")).cookie(cookie))
        .await
        .status()
}

async fn remove(app: &TestApp, cookie: &str, slug: &str) -> StatusCode {
    app.send(req(Method::DELETE, format!("/v1/me/library/{slug}")).cookie(cookie))
        .await
        .status()
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn exec(app: &TestApp, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(&app.db)
        .await
        .unwrap();
}

/// (days launched, last platform) of `user`'s use of `slug`, once they've launched it.
async fn usage(app: &TestApp, user: Uuid, slug: &str) -> Option<(i32, Option<String>)> {
    sqlx::query_as(
        "select u.days_launched, u.last_platform from user_app_usage u join apps a on a.id = u.app_id
         where u.user_id = $1 and a.slug = $2",
    )
    .bind(user)
    .bind(slug)
    .fetch_optional(&app.db)
    .await
    .unwrap()
}

/// (code, message) of a refusal.
async fn refusal(res: Response) -> (String, String) {
    let body = body_json(res).await;
    let text = |k: &str| body["error"][k].as_str().unwrap_or_default().to_string();
    (text("code"), text("message"))
}

/// A usage or library row of `user` for `slug`, dated noon (UTC) `back` days ago.
async fn old_row(app: &TestApp, table: &str, user: Uuid, slug: &str, back: i32) {
    row_at(app, table, user, slug, back, 12).await;
}

/// A usage or library row of `user` for `slug`, dated `hour`:00 (UTC) `back` days ago.
async fn row_at(app: &TestApp, table: &str, user: Uuid, slug: &str, back: i32, hour: i32) {
    let column = if table == "library_entries" {
        "added_at"
    } else {
        "first_launched_at"
    };
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into {table} (user_id, app_id, {column})
         select $1, id, (((now() at time zone 'UTC')::date - $3) + make_time($4, 0, 0)) at time zone 'UTC'
         from apps where slug = $2"
    )))
    .bind(user)
    .bind(slug)
    .bind(back)
    .bind(hour)
    .execute(&app.db)
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------------------------
// What a launch counts

/// A launch counts one player per account per UTC day, and one first play the first time ever.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_launch_counts_a_player_once_a_day_and_a_first_play_once(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (_, sam) = app.user("Sam", "US").await;
    let today = day(&app, 0).await;
    let yesterday = day(&app, 1).await;

    assert!(counts(&app, "game").await.is_empty());
    for _ in 0..2 {
        assert_eq!(
            launch(&app, &pat, "game", "").await.status(),
            StatusCode::OK
        );
    }
    assert_eq!(counts(&app, "game").await, [(today.clone(), 1, 1, 0)]);
    assert_eq!(
        launch(&app, &sam, "game", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(counts(&app, "game").await, [(today.clone(), 2, 2, 0)]);

    // The next day: Pat is a player again, but not a first play.
    sqlx::query(
        "update user_app_usage set last_launch_day = (now() at time zone 'UTC')::date - 1 where user_id = $1",
    )
    .bind(pat_id)
    .execute(&app.db)
    .await
    .unwrap();
    exec(&app, "update app_daily_counts set day = day - 1").await;
    assert_eq!(
        launch(&app, &pat, "game", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        counts(&app, "game").await,
        [(yesterday, 2, 2, 0), (today, 1, 0, 0)]
    );
    assert_eq!(usage(&app, pat_id, "game").await, Some((2, None)));
}

/// Two launches by one account at the same moment count one player: only the launch that wrote
/// the day is counted.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn two_launches_at_once_count_one_player(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    let (pat_id, pat) = app.user("Pat", "US").await;

    let (a, b) = tokio::join!(
        launch(&app, &pat, "game", ""),
        launch(&app, &pat, "game", "")
    );
    assert_eq!(a.status(), StatusCode::OK);
    assert_eq!(b.status(), StatusCode::OK);
    assert_eq!(counts(&app, "game").await, [(day(&app, 0).await, 1, 1, 0)]);
    assert_eq!(usage(&app, pat_id, "game").await, Some((1, None)));
}

/// A later launch the same day counts nothing more, but still says which platform was last.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_second_launch_the_same_day_counts_once(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    let (pat_id, pat) = app.user("Pat", "US").await;

    let res = launch(&app, &pat, "game", "?platform=windows").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        usage(&app, pat_id, "game").await,
        Some((1, Some("windows".into())))
    );
    let res = launch(&app, &pat, "game", "?platform=linux").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(counts(&app, "game").await, [(day(&app, 0).await, 1, 1, 0)]);
    assert_eq!(
        usage(&app, pat_id, "game").await,
        Some((1, Some("linux".into())))
    );
    // With no platform named, the last one stays.
    let res = launch(&app, &pat, "game", "").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        usage(&app, pat_id, "game").await,
        Some((1, Some("linux".into())))
    );
}

/// With "Share my playtime" off a launch is still recorded for the player (it's what lets them
/// vote), but the maker's totals don't count it. A library add isn't playtime: it counts.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_launch_with_share_my_playtime_off_counts_nobody(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let today = day(&app, 0).await;
    let share = |on: bool| {
        req(Method::PATCH, "/v1/me")
            .cookie(&pat)
            .json(json!({ "share_playtime": on }))
    };

    assert_eq!(
        app.send(share(false)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        launch(&app, &pat, "game", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(usage(&app, pat_id, "game").await, Some((1, None)));
    assert!(counts(&app, "game").await.is_empty());
    assert_eq!(add(&app, &pat, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(counts(&app, "game").await, [(today.clone(), 0, 0, 1)]);

    // Switched on later the same day: that day's launch is already written, so not counted.
    assert_eq!(app.send(share(true)).await.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        launch(&app, &pat, "game", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(counts(&app, "game").await, [(today.clone(), 0, 0, 1)]);

    // The next day Pat is a player, and never a first play: the first launch was with it off.
    sqlx::query(
        "update user_app_usage set last_launch_day = (now() at time zone 'UTC')::date - 1 where user_id = $1",
    )
    .bind(pat_id)
    .execute(&app.db)
    .await
    .unwrap();
    exec(&app, "update app_daily_counts set day = day - 1").await;
    assert_eq!(
        launch(&app, &pat, "game", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        counts(&app, "game").await,
        [(day(&app, 1).await, 0, 0, 1), (today, 1, 0, 0)]
    );
}

/// A launch the route refuses records nothing, for the player or for the maker.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_refused_launch_counts_nobody(db: PgPool) {
    let app = test_app(db).await;
    // A browser game in the store that is still Coming soon: only its maker plays it yet.
    let (maker_id, maker) = maker_with_browser_app(&app, "coming_soon").await;
    exec(
        &app,
        "update apps set status = 'listed' where slug = 'game'",
    )
    .await;
    approved_play_address(&app, "game", "1.0.0", "https://games.example.com/play/").await;
    // And a draft nobody else can see.
    listed_with(&app, maker_id, "draft", "released", "status = 'draft'").await;
    let (_, pat) = app.user("Pat", "US").await;

    let res = launch(&app, &pat, "game", "").await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        refusal(res).await,
        ("conflict".to_string(), "nothing to play yet".to_string())
    );
    let res = launch(&app, &pat, "draft", "").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(refusal(res).await.0, "not_found");
    assert_eq!(count(&app, "select count(*) from user_app_usage").await, 0);
    assert_eq!(
        count(&app, "select count(*) from app_daily_counts").await,
        0
    );

    // The maker may play it: recorded for them, and not counted (their own launch).
    assert_eq!(
        launch(&app, &maker, "game", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(count(&app, "select count(*) from user_app_usage").await, 1);
    assert_eq!(
        count(&app, "select count(*) from app_daily_counts").await,
        0
    );
}

/// The maker's own launch and library add are recorded for them as for anyone, and not counted.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_makers_own_launch_and_add_are_not_counted(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    // Someone else's public app is in the store too: the check is about the app launched.
    let (other_id, _) = app.user("Other Maker", "US").await;
    listed_with(&app, other_id, "theirs", "released", "").await;

    assert_eq!(
        launch(&app, &maker, "game", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(add(&app, &maker, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(usage(&app, maker_id, "game").await, Some((1, None)));
    assert_eq!(count(&app, "select count(*) from library_entries").await, 1);
    assert!(counts(&app, "game").await.is_empty());
    assert!(counts(&app, "theirs").await.is_empty());
    // Taking it out again subtracts nothing either.
    assert_eq!(remove(&app, &maker, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(count(&app, "select count(*) from library_entries").await, 0);
    assert!(counts(&app, "game").await.is_empty());

    // Someone else's is counted, and the maker's still changes nothing.
    let (_, pat) = app.user("Pat", "US").await;
    assert_eq!(
        launch(&app, &pat, "game", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(add(&app, &pat, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(add(&app, &maker, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(remove(&app, &maker, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(counts(&app, "game").await, [(day(&app, 0).await, 1, 1, 1)]);
}

// ---------------------------------------------------------------------------------------------
// What a library add counts

/// Only a real insert counts, and a remove the same UTC day takes it back, from that app only. A
/// remove on a later day changes nothing.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_library_add_counts_only_a_real_insert(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    listed_with(&app, maker_id, "other", "released", "").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (_, sam) = app.user("Sam", "US").await;
    let (_, kim) = app.user("Kim", "US").await;
    let today = day(&app, 0).await;
    // Another app counted today: Pat's removes of the game never touch it.
    assert_eq!(add(&app, &kim, "other").await, StatusCode::NO_CONTENT);
    assert_eq!(counts(&app, "other").await, [(today.clone(), 0, 0, 1)]);

    for _ in 0..2 {
        assert_eq!(add(&app, &pat, "game").await, StatusCode::NO_CONTENT);
    }
    assert_eq!(counts(&app, "game").await, [(today.clone(), 0, 0, 1)]);
    assert_eq!(remove(&app, &pat, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(counts(&app, "game").await, [(today.clone(), 0, 0, 0)]);
    assert_eq!(counts(&app, "other").await, [(today.clone(), 0, 0, 1)]);
    // Removing what isn't there takes nothing more off.
    assert_eq!(remove(&app, &pat, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(counts(&app, "game").await, [(today.clone(), 0, 0, 0)]);
    assert_eq!(add(&app, &pat, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(counts(&app, "game").await, [(today.clone(), 0, 0, 1)]);
    assert_eq!(add(&app, &sam, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(counts(&app, "game").await, [(today.clone(), 0, 0, 2)]);

    // A day later: Pat's add was yesterday's, and stays counted there.
    sqlx::query(
        "update library_entries set added_at = added_at - interval '1 day' where user_id = $1",
    )
    .bind(pat_id)
    .execute(&app.db)
    .await
    .unwrap();
    exec(&app, "update app_daily_counts set day = day - 1").await;
    // Kim adds it today, so today has a row that Pat's remove must leave alone.
    assert_eq!(add(&app, &kim, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(remove(&app, &pat, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(
        counts(&app, "game").await,
        [(day(&app, 1).await, 0, 0, 2), (today.clone(), 0, 0, 1)]
    );
    assert_eq!(count(&app, "select count(*) from library_entries").await, 3);
    // Added again on the later day, the account counts again.
    assert_eq!(add(&app, &pat, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(
        counts(&app, "game").await,
        [(day(&app, 1).await, 0, 0, 2), (today, 0, 0, 2)]
    );
}

/// Library adds and history requests are limited per account, and a refused add counts nothing.
/// The history limit is charged before the app is looked at, so it says nothing about any app.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn adds_and_history_requests_are_limited_per_account(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (_, sam) = app.user("Sam", "US").await;
    let now = std::time::Instant::now();
    let drain = |user: Uuid, action: Action| {
        while app.state.account_limiter.take(user, action, now).is_ok() {}
    };

    drain(pat_id, Action::LibraryAdd);
    let res = app
        .send(req(Method::PUT, "/v1/me/library/game").cookie(&pat))
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        refusal(res).await,
        (
            "rate_limited".to_string(),
            "too many library adds for now: try again in a while".to_string()
        )
    );
    assert_eq!(count(&app, "select count(*) from library_entries").await, 0);
    assert!(counts(&app, "game").await.is_empty());
    // Someone else's adds are their own allowance, and removing has none.
    assert_eq!(add(&app, &sam, "game").await, StatusCode::NO_CONTENT);
    assert_eq!(remove(&app, &pat, "game").await, StatusCode::NO_CONTENT);

    // Reading history: Pat (who would get 404) and the maker are told the same thing.
    drain(pat_id, Action::History);
    drain(maker_id, Action::History);
    for (cookie, slug) in [(&pat, "game"), (&pat, "no-such-app"), (&maker, "game")] {
        let res = app
            .send(req(Method::GET, format!("/v1/apps/{slug}/history")).cookie(cookie))
            .await;
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(res.headers().contains_key(header::RETRY_AFTER));
        assert_eq!(
            refusal(res).await,
            (
                "rate_limited".to_string(),
                "too many history requests for now: try again in a while".to_string()
            )
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Private apps

/// Nothing is counted while an app is private, and its maker is told it has no history. The day
/// it goes public is the first day of its history: a part day.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn nothing_is_counted_for_a_private_app_until_it_is_public(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    let (_, sam) = app.user("Sam", "US").await;
    listed_with(
        &app,
        maker_id,
        "secret",
        "released",
        "status = 'draft', visibility = 'private', repo_url = 'https://github.com/mia/secret', platforms = '{windows}'",
    )
    .await;
    // Someone else's public app is in the store too: the check is about the app launched.
    let (other_id, _) = app.user("Other Maker", "US").await;
    listed_with(&app, other_id, "theirs", "released", "").await;
    approved_release(&app, "secret", maker_id).await;
    sqlx::query(
        "insert into app_shares (app_id, user_id) select id, $1 from apps where slug = 'secret'",
    )
    .bind(pat_id)
    .execute(&app.db)
    .await
    .unwrap();

    assert_eq!(
        launch(&app, &pat, "secret", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(add(&app, &pat, "secret").await, StatusCode::NO_CONTENT);
    assert_eq!(usage(&app, pat_id, "secret").await, Some((1, None)));
    assert_eq!(count(&app, "select count(*) from library_entries").await, 1);
    assert_eq!(
        count(&app, "select count(*) from app_daily_counts").await,
        0
    );
    // Taking a private app out of the library changes nothing either.
    assert_eq!(remove(&app, &pat, "secret").await, StatusCode::NO_CONTENT);
    assert_eq!(add(&app, &pat, "secret").await, StatusCode::NO_CONTENT);
    assert_eq!(
        count(&app, "select count(*) from app_daily_counts").await,
        0
    );
    let res = app
        .send(req(Method::GET, "/v1/apps/secret/history").cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        refusal(res).await,
        (
            "conflict".to_string(),
            "Private apps have no history".to_string()
        )
    );
    // Someone it is shared with is told no more than a stranger.
    for cookie in [&pat, &sam] {
        let res = app
            .send(req(Method::GET, "/v1/apps/secret/history").cookie(cookie))
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    // Going public sets both dates to that day, whatever they were.
    exec(
        &app,
        "update apps set history_from = history_from - 5, players_from = players_from - 5,
                public_requested_at = now()
         where slug = 'secret'",
    )
    .await;
    admin::make_public(&app.db, "secret", "rev").await.unwrap();
    let today = day(&app, 0).await;
    let dates: (String, String) = sqlx::query_as(
        "select to_char(history_from, 'YYYY-MM-DD'), to_char(players_from, 'YYYY-MM-DD')
         from apps where slug = 'secret'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(dates, (today.clone(), today.clone()));

    assert_eq!(
        launch(&app, &sam, "secret", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(counts(&app, "secret").await, [(today.clone(), 1, 1, 0)]);
    // Pat's launch of the day happened while it was private, and it is not a first play.
    assert_eq!(
        launch(&app, &pat, "secret", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(counts(&app, "secret").await, [(today.clone(), 1, 1, 0)]);
    let read = history(&app, &maker, "secret", "").await;
    assert_eq!(read["history_from"], today);
    assert_eq!(read["players_from"], today);
    assert_eq!(
        read["days"][29],
        json!({ "day": today, "players": 1, "first_plays": 1, "library_adds": 0, "peak": 0 })
    );
    assert_eq!(
        read["days"][28],
        json!({ "day": day(&app, 1).await, "players": null, "first_plays": null,
                "library_adds": null, "peak": null })
    );
}

// ---------------------------------------------------------------------------------------------
// Reading it

/// Every day of the range is there, oldest first. A day before counting began is null, not
/// zero; a counted day with nothing in it is zero.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn history_gives_every_day_and_says_what_was_not_counted(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    exec(
        &app,
        "update apps set history_from = (now() at time zone 'UTC')::date - 10,
                         players_from = (now() at time zone 'UTC')::date - 3
         where slug = 'game'",
    )
    .await;
    for (back, players, first_plays, library_adds) in [(20, 5, 5, 5), (5, 4, 2, 3), (1, 7, 1, 2)] {
        sqlx::query(
            "insert into app_daily_counts (app_id, day, players, first_plays, library_adds)
             select id, (now() at time zone 'UTC')::date - $1, $2, $3, $4 from apps where slug = 'game'",
        )
        .bind(back)
        .bind(players)
        .bind(first_plays)
        .bind(library_adds)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Pulse buckets: two on one day (the larger is that day's busiest moment), the first and
    // the last hours of a UTC day, and one on a day before the history begins.
    for (back, hour, players) in [(20, 12, 9), (5, 0, 3), (5, 21, 8), (0, 0, 2)] {
        sqlx::query(
            "insert into pulse_counts (app_id, bucket, players)
             select id, (((now() at time zone 'UTC')::date - $1) + make_time($2, 0, 0)) at time zone 'UTC', $3
             from apps where slug = 'game'",
        )
        .bind(back)
        .bind(hour)
        .bind(players)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Another app's numbers never show up in this one's.
    listed_with(&app, maker_id, "other", "released", "").await;
    exec(
        &app,
        "insert into app_daily_counts (app_id, day, players, first_plays, library_adds)
         select id, (now() at time zone 'UTC')::date, 50, 50, 50 from apps where slug = 'other'",
    )
    .await;
    exec(
        &app,
        "insert into pulse_counts (app_id, bucket, players)
         select id, now(), 50 from apps where slug = 'other'",
    )
    .await;

    let read = history(&app, &maker, "game", "?days=30").await;
    assert_eq!(read["history_from"], day(&app, 10).await);
    assert_eq!(read["players_from"], day(&app, 3).await);
    let days = read["days"].as_array().unwrap();
    assert_eq!(days.len(), 30);
    for (i, entry) in days.iter().enumerate() {
        let back = 29 - i as i32;
        assert_eq!(entry["day"], day(&app, back).await, "oldest first");
        let expected = match back {
            11.. => json!([null, null, null, null]),
            5 => json!([null, 2, 3, 8]),
            4..=10 => json!([null, 0, 0, 0]),
            1 => json!([7, 1, 2, 0]),
            0 => json!([0, 0, 0, 2]),
            _ => json!([0, 0, 0, 0]),
        };
        assert_eq!(
            json!([
                entry["players"],
                entry["first_plays"],
                entry["library_adds"],
                entry["peak"]
            ]),
            expected,
            "{back} days ago"
        );
    }

    assert_eq!(history(&app, &maker, "game", "").await, read);
    for n in [90, 365] {
        let read = history(&app, &maker, "game", &format!("?days={n}")).await;
        let days = read["days"].as_array().unwrap();
        assert_eq!(days.len(), n);
        assert_eq!(days[n - 1]["day"], day(&app, 0).await);
        assert_eq!(days[0]["day"], day(&app, n as i32 - 1).await);
        // The row from 20 days ago is in range now, and still before the history began.
        assert_eq!(days[n - 21]["day"], day(&app, 20).await);
        assert!(days[n - 21]["library_adds"].is_null());
        assert!(days[n - 21]["peak"].is_null());
    }
    for bad in ["?days=7", "?days=abc", "?days=", "?days=30.0", "?days=-30"] {
        let res = app
            .send(req(Method::GET, format!("/v1/apps/game/history{bad}")).cookie(&maker))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(
            body_json(res).await["error"],
            json!({ "code": "invalid", "field": "days", "message": "30, 90 or 365" }),
            "{bad}"
        );
    }
}

/// Only the maker reads an app's history: anyone else is told there is no such app. A dedicated
/// server has none, and nothing is counted for one.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_the_maker_reads_history(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    let (other_id, other) = app.user("Other Maker", "US").await;
    listed_with(&app, other_id, "theirs", "released", "").await;
    let (_, pat) = app.user("Pat", "US").await;
    assert_eq!(add(&app, &pat, "game").await, StatusCode::NO_CONTENT);

    let read = |cookie: Option<&str>, slug: &str| {
        let r = req(Method::GET, format!("/v1/apps/{slug}/history"));
        match cookie {
            Some(c) => r.cookie(c),
            None => r,
        }
    };
    assert_eq!(
        app.send(read(None, "game")).await.status(),
        StatusCode::UNAUTHORIZED
    );
    for cookie in [&other, &pat] {
        let res = app.send(read(Some(cookie), "game")).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_json(res).await["error"]["code"], "not_found");
    }
    let res = app.send(read(Some(&maker), "no-such-app")).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        app.send(read(Some(&maker), "theirs")).await.status(),
        StatusCode::NOT_FOUND
    );
    // A wrong `days` tells a stranger nothing either: the app is looked up first.
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/game/history?days=7").cookie(&pat))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(read(Some(&maker), "game")).await.status(),
        StatusCode::OK
    );
    // A delisted app's maker still reads what was counted.
    exec(
        &app,
        "update apps set status = 'delisted' where slug = 'game'",
    )
    .await;
    let read_it = history(&app, &maker, "game", "").await;
    assert_eq!(read_it["days"][29]["library_adds"], 1);
    exec(
        &app,
        "update apps set status = 'listed' where slug = 'game'",
    )
    .await;

    // The maker's own dedicated server.
    listed_with(
        &app,
        maker_id,
        "srv",
        "released",
        "parent_app_id = (select id from apps where slug = 'game'), branch_kind = 'server', platforms = '{linux}'",
    )
    .await;
    let res = app.send(read(Some(&maker), "srv")).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        refusal(res).await,
        (
            "conflict".to_string(),
            "Dedicated servers have no history".to_string()
        )
    );
    assert_eq!(
        app.send(read(Some(&pat), "srv")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(add(&app, &pat, "srv").await, StatusCode::NO_CONTENT);
    assert_eq!(
        launch(&app, &pat, "srv", "?platform=linux").await.status(),
        StatusCode::OK
    );
    assert_eq!(count(&app, "select count(*) from library_entries").await, 2);
    assert!(counts(&app, "srv").await.is_empty());
    // Taking the server out again leaves the game's own count alone.
    assert_eq!(remove(&app, &pat, "srv").await, StatusCode::NO_CONTENT);
    assert_eq!(counts(&app, "game").await, [(day(&app, 0).await, 0, 0, 1)]);
}

// ---------------------------------------------------------------------------------------------
// The back-fill (migration 0095)

/// The migration's back-fill, run again on rows made for it: first plays and library adds are
/// counted from the rows that still exist, on the day they happened, from the day the app's
/// history begins. Never the maker's own, a private app's (not even earlier on the day it went
/// public), or the first play of an account that doesn't share its playtime; players per day are
/// not rebuilt.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_back_fill_counts_only_what_still_exists(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    let (pat_id, _) = app.user("Pat", "US").await;
    let (sam_id, _) = app.user("Sam", "US").await;
    let (olly_id, _) = app.user("Olly Off", "US").await;
    let (kim_id, _) = app.user("Kim", "US").await;
    sqlx::query("update users set share_playtime = false where id = $1")
        .bind(olly_id)
        .execute(&app.db)
        .await
        .unwrap();
    let made =
        "created_at = (((now() at time zone 'UTC')::date - {}) + time '12:00') at time zone 'UTC'";
    // Made 10 days ago; made yesterday (older rows fall before its history); private; and one
    // made 10 days ago that went public 2 days ago.
    listed_with(&app, maker_id, "old", "released", &made.replace("{}", "10")).await;
    listed_with(
        &app,
        maker_id,
        "young",
        "released",
        &made.replace("{}", "1"),
    )
    .await;
    listed_with(
        &app,
        maker_id,
        "hidden",
        "released",
        &format!(
            "status = 'draft', visibility = 'private', {}",
            made.replace("{}", "10")
        ),
    )
    .await;
    listed_with(
        &app,
        maker_id,
        "opened",
        "released",
        &made.replace("{}", "10"),
    )
    .await;
    // Made public twice: the later day is the one that counts.
    for back in [6, 2] {
        sqlx::query(
            "insert into audit_log (actor, action, subject, at)
             values ('rev', 'app.make_public', 'opened',
                     (((now() at time zone 'UTC')::date - $1) + time '12:00') at time zone 'UTC')",
        )
        .bind(back)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // A dedicated server of the old app.
    listed_with(
        &app,
        maker_id,
        "srv",
        "released",
        &format!(
            "parent_app_id = (select id from apps where slug = 'old'), branch_kind = 'server', platforms = '{{linux}}', {}",
            made.replace("{}", "10")
        ),
    )
    .await;

    for slug in ["old", "young", "hidden"] {
        old_row(&app, "user_app_usage", pat_id, slug, 3).await;
        old_row(&app, "user_app_usage", sam_id, slug, 2).await;
        old_row(&app, "user_app_usage", olly_id, slug, 2).await;
        old_row(&app, "user_app_usage", maker_id, slug, 2).await;
        for user in [pat_id, sam_id, olly_id, maker_id] {
            old_row(&app, "library_entries", user, slug, 2).await;
        }
    }
    // The app that went public (at noon, 2 days ago): a first play the day before, and adds
    // before and on the day. Kim's first play and add that morning were while it was private.
    old_row(&app, "user_app_usage", pat_id, "opened", 3).await;
    old_row(&app, "library_entries", pat_id, "opened", 3).await;
    old_row(&app, "library_entries", sam_id, "opened", 2).await;
    old_row(&app, "user_app_usage", sam_id, "opened", 0).await;
    row_at(&app, "user_app_usage", kim_id, "opened", 2, 6).await;
    row_at(&app, "library_entries", kim_id, "opened", 2, 6).await;
    old_row(&app, "library_entries", pat_id, "srv", 2).await;

    // As the migration finds the database: nothing counted, and every app's dates the ship day.
    assert_eq!(
        count(&app, "select count(*) from app_daily_counts").await,
        0
    );
    let sql = include_str!("../../migrations/0095_app_history.sql");
    let block = sql
        .split_once("-- backfill:start\n")
        .unwrap()
        .1
        .split_once("-- backfill:end")
        .unwrap()
        .0;
    let statements: Vec<&str> = block
        .split(";\n")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(statements.len(), 2, "{statements:?}");
    for statement in statements {
        sqlx::query(sqlx::AssertSqlSafe(statement.to_string()))
            .execute(&app.db)
            .await
            .unwrap();
    }

    let dates: Vec<(String, String, String)> = sqlx::query_as(
        "select slug, to_char(history_from, 'YYYY-MM-DD'), to_char(players_from, 'YYYY-MM-DD')
         from apps order by slug",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let today = day(&app, 0).await;
    let from = |slug: &str, history_from: String| (slug.to_string(), history_from, today.clone());
    assert_eq!(
        dates,
        [
            // Private: untouched, and reset the day it goes public.
            from("hidden", today.clone()),
            from("old", day(&app, 10).await),
            from("opened", day(&app, 2).await),
            from("srv", day(&app, 10).await),
            from("young", day(&app, 1).await),
        ]
    );
    // Pat's first play; then Sam's (not Olly's, who doesn't share playtime, nor the maker's) and
    // three adds (Olly's counts: an add isn't playtime).
    assert_eq!(
        counts(&app, "old").await,
        [(day(&app, 3).await, 0, 1, 0), (day(&app, 2).await, 0, 1, 3)]
    );
    assert!(counts(&app, "young").await.is_empty());
    assert!(counts(&app, "hidden").await.is_empty());
    assert!(counts(&app, "srv").await.is_empty());
    // Only what happened from the moment it went public: not Kim's, earlier that day.
    assert_eq!(
        counts(&app, "opened").await,
        [(day(&app, 2).await, 0, 0, 1), (today, 0, 1, 0)]
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from app_daily_counts where players <> 0"
        )
        .await,
        0
    );
}
