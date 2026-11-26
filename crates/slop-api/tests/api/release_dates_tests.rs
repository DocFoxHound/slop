// SPDX-License-Identifier: AGPL-3.0-or-later
//! Release dates for Coming soon and Beta apps, the front page's Releasing soon shelf, and the Hot
//! reel (hot.rs).

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers for this area

/// Today in UTC, and `days` from it, as the API takes a date.
fn day(days: i64) -> String {
    (OffsetDateTime::now_utc().date() + time::Duration::days(days)).to_string()
}

/// `YYYY-MM` of the month `months` after this one.
fn month(months: i32) -> String {
    let today = OffsetDateTime::now_utc().date();
    let m = today.month() as i32 - 1 + months;
    format!(
        "{}-{:02}",
        today.year() + m.div_euclid(12),
        m.rem_euclid(12) + 1
    )
}

async fn patch(app: &TestApp, cookie: &str, slug: &str, body: Value) -> Response {
    app.send(
        req(Method::PATCH, format!("/v1/apps/{slug}"))
            .cookie(cookie)
            .json(body),
    )
    .await
}

/// A refusal's field and message.
async fn refused(res: Response) -> (String, String) {
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_json(res).await;
    (
        body["error"]["field"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    )
}

async fn get_json(app: &TestApp, r: Req) -> Value {
    let res = app.send(r).await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

fn slugs(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|a| a["slug"].as_str().unwrap().to_string())
        .collect()
}

/// Accounts made straight in the database, for library adds and votes.
async fn people(app: &TestApp, n: usize) -> Vec<Uuid> {
    let mut out = Vec::new();
    for i in 0..n {
        let id = Uuid::now_v7();
        sqlx::query(
            "insert into users (id, display_name, country_code, age_16_confirmed_at) values ($1, $2, 'US', now())",
        )
        .bind(id)
        .bind(format!("Person {i}"))
        .execute(&app.db)
        .await
        .unwrap();
        out.push(id);
    }
    out
}

/// `who` added `slug` to their library `days_ago`.
async fn added(app: &TestApp, slug: &str, who: &[Uuid], days_ago: i32) {
    for person in who {
        sqlx::query(
            "insert into library_entries (user_id, app_id, added_at)
             select $1, id, now() - make_interval(days => $3) from apps where slug = $2",
        )
        .bind(person)
        .bind(slug)
        .bind(days_ago)
        .execute(&app.db)
        .await
        .unwrap();
    }
}

/// `who` voted `vote` on `slug` `days_ago` (a vote event; the latest is their vote).
async fn voted(app: &TestApp, slug: &str, who: &[Uuid], vote: i16, days_ago: i32) {
    for person in who {
        sqlx::query(
            "insert into vote_events (id, user_id, app_id, vote, weight, created_at)
             select $1, $2, id, $4, 1, now() - make_interval(days => $5) from apps where slug = $3",
        )
        .bind(Uuid::now_v7())
        .bind(person)
        .bind(slug)
        .bind(vote)
        .bind(days_ago)
        .execute(&app.db)
        .await
        .unwrap();
    }
}

/// The most playing `slug` at once, `days_ago` (Slop Pulse).
async fn played(app: &TestApp, slug: &str, players: i32, days_ago: i32) {
    sqlx::query(
        "insert into pulse_counts (app_id, bucket, players)
         select id, date_trunc('minute', now()) - make_interval(days => $3), $2 from apps where slug = $1",
    )
    .bind(slug)
    .bind(players)
    .bind(days_ago)
    .execute(&app.db)
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------------------------
// Release dates

/// Its maker sets a release date on a Coming soon or Beta app, to the day or the month, from
/// today to three years ahead; it's refused for a released app, cleared with null, and cleared
/// when the app is released.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_give_a_release_date_until_it_is_released(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "coming_soon").await;
    let (_, stranger) = app.user("Stranger", "US").await;

    let res = patch(&app, &maker, "game", json!({ "release_date": day(10) })).await;
    assert_eq!(res.status(), StatusCode::OK);
    let page = body_json(res).await;
    assert_eq!(page["release_date"], day(10));
    assert_eq!(page["release_date_precision"], "day");
    assert_eq!(page["release_date_passed"], false);
    // Today is fine; someone else's app isn't there.
    assert_eq!(
        patch(&app, &maker, "game", json!({ "release_date": day(0) }))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        patch(&app, &stranger, "game", json!({ "release_date": day(5) }))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // A month: kept as its first day. This month is fine, though its 1st may have gone.
    for (months, given) in [
        (2, month(2)),
        (0, month(0)),
        (1, format!("{}-17", month(1))),
    ] {
        let res = patch(
            &app,
            &maker,
            "game",
            json!({ "release_date": given, "release_date_precision": "month" }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK, "{given}");
        let page = body_json(res).await;
        assert_eq!(page["release_date"], format!("{}-01", month(months)));
        assert_eq!(page["release_date_precision"], "month");
    }

    // What's refused, and why.
    let cases = [
        (
            json!({ "release_date": day(-1) }),
            "release_date",
            "has passed",
        ),
        (
            json!({ "release_date": month(-1), "release_date_precision": "month" }),
            "release_date",
            "month is over",
        ),
        (
            json!({ "release_date": day(3 * 365 + 2) }),
            "release_date",
            "three years",
        ),
        (
            json!({ "release_date": "soon" }),
            "release_date",
            "YYYY-MM-DD",
        ),
        (
            json!({ "release_date": "2027-02-30" }),
            "release_date",
            "YYYY-MM-DD",
        ),
        (
            json!({ "release_date": day(5), "release_date_precision": "week" }),
            "release_date_precision",
            "day",
        ),
        (
            json!({ "release_date_precision": "month" }),
            "release_date_precision",
            "with it",
        ),
    ];
    for (body, field, says) in cases {
        let (f, message) = refused(patch(&app, &maker, "game", body.clone()).await).await;
        assert_eq!(f, field, "{body}");
        assert!(message.contains(says), "{body}: {message}");
    }
    assert_eq!(
        get_json(&app, req(Method::GET, "/v1/apps/game").cookie(&maker)).await["release_date"],
        format!("{}-01", month(1)),
        "nothing refused was saved"
    );

    // null clears it.
    let res = patch(&app, &maker, "game", json!({ "release_date": null })).await;
    let page = body_json(res).await;
    assert_eq!(page["release_date"], Value::Null);
    assert_eq!(page["release_date_precision"], Value::Null);
    assert!(page.get("release_date_passed").is_none());

    // Released: none, and moving to Released clears it.
    patch(&app, &maker, "game", json!({ "release_date": day(30) })).await;
    let id = submit_and_fetch(&app, &maker, "0.1.0").await;
    approve_release(&app, lax(), id.parse().unwrap())
        .await
        .unwrap();
    let move_to = |stage: &str| {
        req(Method::POST, "/v1/apps/game/stage")
            .cookie(&maker)
            .json(json!({ "stage": stage }))
    };
    assert_eq!(
        app.send(move_to("beta")).await.status(),
        StatusCode::NO_CONTENT
    );
    let page = get_json(&app, req(Method::GET, "/v1/apps/game").cookie(&maker)).await;
    assert_eq!(page["release_date"], day(30), "Beta keeps it");
    assert_eq!(
        app.send(move_to("released")).await.status(),
        StatusCode::NO_CONTENT
    );
    let page = get_json(&app, req(Method::GET, "/v1/apps/game").cookie(&maker)).await;
    assert_eq!(page["stage"], "released");
    assert_eq!(page["release_date"], Value::Null);
    let (field, message) =
        refused(patch(&app, &maker, "game", json!({ "release_date": day(5) })).await).await;
    assert_eq!(field, "release_date");
    assert!(message.contains("only Coming soon and Beta"), "{message}");
    // Clearing is fine (there's nothing), and so is saving other fields.
    assert_eq!(
        patch(
            &app,
            &maker,
            "game",
            json!({ "release_date": null, "summary": "Out now." })
        )
        .await
        .status(),
        StatusCode::OK
    );
    // The database holds to it too.
    assert!(
        sqlx::query("update apps set release_date = current_date + 3, release_date_precision = 'day' where slug = 'game'")
            .execute(&app.db)
            .await
            .is_err()
    );
}

/// New pages take a release date too, except released ones.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn new_pages_take_a_release_date_unless_released(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = app.user("Maker", "US").await;
    let create = |slug: &str, stage: &str, extra: Value| {
        let mut body = new_app(slug);
        body["stage"] = json!(stage);
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        req(Method::POST, "/v1/apps").cookie(&maker).json(body)
    };
    let res = app
        .send(create(
            "soon-game",
            "coming_soon",
            json!({ "release_date": month(3), "release_date_precision": "month" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let page = body_json(res).await;
    assert_eq!(page["release_date"], format!("{}-01", month(3)));
    assert_eq!(page["release_date_precision"], "month");
    let res = app
        .send(create(
            "beta-game",
            "beta",
            json!({ "release_date": day(7) }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(body_json(res).await["release_date"], day(7));
    let (field, _) = refused(
        app.send(create(
            "out-game",
            "released",
            json!({ "release_date": day(7) }),
        ))
        .await,
    )
    .await;
    assert_eq!(field, "release_date");
    let (field, _) = refused(
        app.send(create(
            "odd-game",
            "beta",
            json!({ "release_date_precision": "day" }),
        ))
        .await,
    )
    .await;
    assert_eq!(field, "release_date_precision");
    let made: i64 = sqlx::query_scalar("select count(*) from apps")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(made, 2);
}

/// Once a date has passed (a day once it's over, a month once it is), players don't see it on
/// the store page or in lists; its maker does, flagged as passed, on their page and their apps.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn passed_release_dates_are_hidden_from_players_but_not_their_maker(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let today = "(now() at time zone 'utc')::date";
    for (slug, set) in [
        (
            "today",
            format!("release_date = {today}, release_date_precision = 'day'"),
        ),
        (
            "yesterday",
            format!("release_date = {today} - 1, release_date_precision = 'day'"),
        ),
        (
            "this-month",
            format!(
                "release_date = date_trunc('month', {today})::date, release_date_precision = 'month'"
            ),
        ),
        (
            "last-month",
            format!(
                "release_date = (date_trunc('month', {today}) - interval '1 month')::date, release_date_precision = 'month'"
            ),
        ),
    ] {
        listed_with(&app, maker_id, slug, "beta", &set).await;
    }
    let shown = |page: &Value| page["release_date"].as_str().map(str::to_string);
    for (slug, seen) in [
        ("today", true),
        ("yesterday", false),
        ("this-month", true),
        ("last-month", false),
    ] {
        let public = get_json(&app, req(Method::GET, format!("/v1/apps/{slug}"))).await;
        assert_eq!(shown(&public).is_some(), seen, "{slug}");
        assert_eq!(public["release_date_precision"].is_string(), seen, "{slug}");
        assert!(public.get("release_date_passed").is_none(), "{slug}");
        let own = get_json(
            &app,
            req(Method::GET, format!("/v1/apps/{slug}")).cookie(&maker),
        )
        .await;
        assert!(shown(&own).is_some(), "{slug}: its maker sees it");
        assert_eq!(own["release_date_passed"], !seen, "{slug}");
    }
    // In lists: shown only while it's to come; the maker's own list flags the passed ones.
    let listed = get_json(&app, req(Method::GET, "/v1/apps?sort=title")).await;
    for a in listed["apps"].as_array().unwrap() {
        let to_come = ["today", "this-month"].contains(&a["slug"].as_str().unwrap());
        assert_eq!(a["release_date"].is_string(), to_come, "{a}");
        assert!(a.get("release_date_passed").is_none());
    }
    let mine = get_json(&app, req(Method::GET, "/v1/me/apps").cookie(&maker)).await;
    let mut passed: Vec<&str> = mine
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["release_date_passed"] == true)
        .map(|a| a["slug"].as_str().unwrap())
        .collect();
    passed.sort();
    assert_eq!(passed, ["last-month", "yesterday"]);
    assert!(
        mine.as_array()
            .unwrap()
            .iter()
            .all(|a| a["release_date"].is_string())
    );
}

/// Releasing soon: listed public Coming soon and Beta apps with a date in the next 14 days (a
/// month that starts by then counts), by the first day each may come (a month before an exact day
/// on its 1st), in the store for the viewer and never dead. The /upcoming sort puts dated apps
/// first.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn releasing_soon_is_the_next_14_days_soonest_first(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    let today = "(now() at time zone 'utc')::date";
    let on = |days: i32| format!("release_date = {today} + {days}, release_date_precision = 'day'");
    let in_month = |of: &str| {
        format!("release_date = date_trunc('month', {of})::date, release_date_precision = 'month'")
    };
    let cases: Vec<(&str, &str, String)> = vec![
        ("in-5-days", "beta", on(5)),
        ("in-14-days", "coming_soon", on(14)),
        ("this-month", "coming_soon", in_month(today)),
        // The month the 14th day is in starts within the window.
        (
            "month-of-day-14",
            "coming_soon",
            in_month(&format!("{today} + 14")),
        ),
        ("today", "coming_soon", on(0)),
        // Left out:
        ("in-15-days", "beta", on(15)),
        (
            "month-after",
            "coming_soon",
            in_month(&format!(
                "(date_trunc('month', {today} + 14) + interval '1 month')"
            )),
        ),
        ("yesterday", "beta", on(-1)),
        ("no-date", "coming_soon", String::new()),
        (
            "draft",
            "coming_soon",
            format!("{}, status = 'draft'", on(3)),
        ),
        (
            "private",
            "coming_soon",
            format!("{}, visibility = 'private'", on(3)),
        ),
        (
            "dead",
            "beta",
            format!("{}, last_commit_at = now() - interval '200 days'", on(3)),
        ),
        (
            "missed-deadline",
            "coming_soon",
            format!("{}, stage_entered_at = now() - interval '400 days'", on(3)),
        ),
        ("adult", "beta", format!("{}, min_age = 18", on(4))),
        ("branch", "beta", on(4)),
    ];
    for (slug, stage, set) in &cases {
        listed_with(&app, maker_id, slug, stage, set).await;
    }
    sqlx::query("update apps set parent_app_id = (select id from apps where slug = 'no-date'), branch_kind = 'fix' where slug = 'branch'")
        .execute(&app.db)
        .await
        .unwrap();
    let soon = |country: &str| {
        list_slugs(
            &app,
            req(Method::GET, "/v1/apps?sort=releasing_soon").header("x-country", country),
        )
    };
    // On the 1st, this-month and today share a first day: the month comes first.
    // In the first half of a month, day 14 is still this month: the two month cases share a first
    // day and go by title. Near month-end, the 1st of next month can fall on or before day 4/5:
    // month precision sorts before day precision on the same date.
    let month_14_offset: i32 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select (date_trunc('month', {today} + 14)::date - {today})::int4"
    )))
    .fetch_one(&app.db)
    .await
    .unwrap();
    let in_order = |slugs: &[&'static str]| -> Vec<&'static str> {
        let mut v: Vec<&'static str> = slugs
            .iter()
            .copied()
            .filter(|s| *s != "month-of-day-14")
            .collect();
        let pos = if month_14_offset <= 0 {
            0
        } else {
            let day_of = |s: &str| match s {
                "this-month" => i32::MIN,
                "today" => 0,
                "adult" => 4,
                "in-5-days" => 5,
                "in-14-days" => 14,
                _ => i32::MAX,
            };
            v.iter()
                .position(|s| day_of(s) >= month_14_offset)
                .unwrap_or(v.len())
        };
        v.insert(pos, "month-of-day-14");
        v
    };
    assert_eq!(
        soon("US").await,
        in_order(&[
            "this-month",
            "today",
            "adult",
            "in-5-days",
            "month-of-day-14",
            "in-14-days"
        ])
    );
    assert_eq!(
        soon("GB").await,
        in_order(&[
            "this-month",
            "today",
            "in-5-days",
            "month-of-day-14",
            "in-14-days"
        ])
    );
    let page = get_json(
        &app,
        req(Method::GET, "/v1/apps?sort=releasing_soon&limit=2"),
    )
    .await;
    assert_eq!(page["apps"][1]["release_date"], day(0));
    assert_eq!(page["next_offset"], 2);

    // Upcoming, by release date: dated ones first, soonest first, then the rest.
    let by_date = list_slugs(
        &app,
        req(Method::GET, "/v1/apps?stage=coming_soon&sort=release_date"),
    )
    .await;
    assert_eq!(
        by_date,
        in_order(&[
            "this-month",
            "today",
            "month-of-day-14",
            "in-14-days",
            "month-after",
            "no-date"
        ])
    );
}

/// Staff can hide the two new automatic shelves like the others.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_hide_hot_and_releasing_soon(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Owner").await;
    let listed = get_json(&app, req(Method::GET, "/v1/admin/shelves").cookie(&owner)).await;
    let keys: Vec<&str> = listed["built_in"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["key"].as_str().unwrap())
        .collect();
    assert!(
        keys.contains(&"hot") && keys.contains(&"releasing_soon"),
        "{keys:?}"
    );
    for key in ["hot", "releasing_soon"] {
        let res = app
            .send(
                req(Method::POST, format!("/v1/admin/front-shelves/{key}"))
                    .cookie(&owner)
                    .json(json!({ "hidden": true })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK, "{key}");
    }
    assert_eq!(
        get_json(&app, req(Method::GET, "/v1/shelves")).await["hidden"],
        json!(["hot", "releasing_soon"])
    );
}

/// A maker's agent sets, reads and clears the release date through the store page tools.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_set_the_release_date(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&maker)
                .json(json!({ "name": "Agent", "scope": "publish" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let token = format!(
        "Bearer {}",
        body_json(res).await["secret"].as_str().unwrap()
    );
    let call = |name: &str, args: Value| {
        req(Method::POST, "/v1/maker/mcp")
            .header("authorization", token.clone())
            .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                          "params": { "name": name, "arguments": args } }))
    };
    let tool = |name: &'static str, args: Value| {
        let r = call(name, args);
        let app = &app;
        async move {
            let reply = body_json(app.send(r).await).await;
            let text = reply["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string();
            (reply["result"]["isError"] == true, text)
        }
    };
    let tools = body_json(
        app.send(
            req(Method::POST, "/v1/maker/mcp")
                .header("authorization", token.clone())
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} })),
        )
        .await,
    )
    .await;
    for name in ["update_store_page", "create_store_page"] {
        let schema = &tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap()["inputSchema"]["properties"];
        assert!(schema["release_date"].is_object(), "{name}");
        assert!(schema["release_date_precision"].is_object(), "{name}");
    }

    let (failed, text) = tool(
        "update_store_page",
        json!({ "slug": "game", "release_date": month(1), "release_date_precision": "month" }),
    )
    .await;
    assert!(!failed, "{text}");
    let (failed, text) = tool("get_store_page", json!({ "slug": "game" })).await;
    assert!(!failed, "{text}");
    let page: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(page["release_date"], format!("{}-01", month(1)));
    assert_eq!(page["release_date_precision"], "month");
    assert_eq!(page["release_date_passed"], false);
    let (failed, text) = tool(
        "update_store_page",
        json!({ "slug": "game", "release_date": day(-3) }),
    )
    .await;
    assert!(failed && text.contains("release_date"), "{text}");
    let (failed, text) = tool(
        "update_store_page",
        json!({ "slug": "game", "release_date": null }),
    )
    .await;
    assert!(!failed, "{text}");
    let page: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(page["release_date"], Value::Null);
}

// ---------------------------------------------------------------------------------------------
// Hot

/// Hot: this week's library adds, Slaps (each account's latest vote) and players, growth over the
/// week before counted twice, at least 5 activity, Beta and Released only, in the store for the
/// viewer, never branches or dead projects; at most 10, highest score first.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn hot_is_whats_taking_off_this_week(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Maker", "US").await;
    let p = people(&app, 12).await;
    for (slug, stage, set) in [
        ("steady", "released", ""),
        ("rising", "released", ""),
        ("slapped", "beta", ""),
        ("played", "released", ""),
        ("one-add", "released", ""),
        ("changed-mind", "released", ""),
        ("coming", "coming_soon", ""),
        ("branch", "released", ""),
        (
            "dead",
            "beta",
            "last_commit_at = now() - interval '200 days'",
        ),
        ("adult", "released", "min_age = 18"),
        ("hidden", "released", "visibility = 'private'"),
    ] {
        listed_with(&app, maker_id, slug, stage, set).await;
    }
    sqlx::query("update apps set parent_app_id = (select id from apps where slug = 'steady'), branch_kind = 'fix' where slug = 'branch'")
        .execute(&app.db)
        .await
        .unwrap();
    // steady: 6 adds this week and 6 the week before: 6.
    added(&app, "steady", &p[..6], 2).await;
    added(&app, "steady", &p[6..12], 10).await;
    // rising: 5 adds this week, none before: 5 + 5 = 10.
    added(&app, "rising", &p[..5], 1).await;
    // slapped: 1 add and 3 Slaps (one voted Slop first): 7 + 7 = 14.
    added(&app, "slapped", &p[..1], 1).await;
    voted(&app, "slapped", &p[..1], -1, 3).await;
    voted(&app, "slapped", &p[..3], 1, 2).await;
    // played: up to 8 at once this week, 10 the week before: 8.
    played(&app, "played", 8, 1).await;
    played(&app, "played", 10, 9).await;
    // one-add: 1, under the floor.
    added(&app, "one-add", &p[..1], 1).await;
    // changed-mind: 1 add and two Slaps changed to Slop: 1, under the floor.
    added(&app, "changed-mind", &p[..1], 1).await;
    voted(&app, "changed-mind", &p[..2], 1, 3).await;
    voted(&app, "changed-mind", &p[..2], -1, 2).await;
    // Busy, but left out: Coming soon, a branch, dead, private; adult only in the 18+ view.
    for slug in ["coming", "branch", "dead", "adult", "hidden"] {
        added(&app, slug, &p[..10], 1).await;
    }
    // Long ago doesn't count.
    added(&app, "one-add", &p[1..10], 20).await;

    let hot = |country: &str| {
        let app = &app;
        let country = country.to_string();
        async move {
            get_json(
                app,
                req(Method::GET, "/v1/shelves/hot").header("x-country", country),
            )
            .await
        }
    };
    let us = hot("US").await;
    assert_eq!(
        slugs(&us),
        ["adult", "slapped", "rising", "played", "steady"]
    );
    let slapped = &us[1];
    assert_eq!(
        (
            slapped["adds_week"].as_i64(),
            slapped["slaps_week"].as_i64(),
            slapped["players_week"].as_i64()
        ),
        (Some(1), Some(3), Some(0))
    );
    assert_eq!(us[3]["players_week"], 8);
    assert_eq!(us[1]["stage"], "beta");
    // Each with its gallery for the reel to go through (none approved here), and no inner id.
    assert!(
        us.as_array()
            .unwrap()
            .iter()
            .all(|a| a["gallery"] == json!([]) && a.get("hot_id").is_none())
    );
    assert_eq!(
        slugs(&hot("GB").await),
        ["slapped", "rising", "played", "steady"]
    );

    // At most 10.
    for k in 0..8 {
        let slug = format!("more-{k}");
        listed_with(&app, maker_id, &slug, "released", "").await;
        added(&app, &slug, &p[..5], 1).await;
    }
    assert_eq!(slugs(&hot("US").await).len(), 10);
}
