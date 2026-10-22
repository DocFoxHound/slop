// SPDX-License-Identifier: AGPL-3.0-or-later
//! Part 1 of the admin tests: reports, decisions and appeals, abuse signals, freezing, the admin
//! panel's review actions, staff and approvals, legal documents, ops alerts, limits, cleanup and
//! the simulation's sign-in.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers

/// A staff member with one role (staff.rs), signed in.
async fn staff(app: &TestApp, name: &str, role: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("insert into staff_roles (user_id, role, granted_by) values ($1, $2, 'test')")
        .bind(id)
        .bind(role)
        .execute(&app.db)
        .await
        .unwrap();
    (id, cookie)
}

/// An admin API token (`read`, `review` or `moderate`) for a signed-in owner, as the value of an
/// Authorization header.
async fn token(app: &TestApp, cookie: &str, scope: &str) -> String {
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(cookie)
                .json(json!({ "name": format!("{scope} agent"), "scope": scope, "days": 7 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    format!("Bearer {}", body_json(res).await["token"].as_str().unwrap())
}

/// Email set up, so signed-out reporters' emails land in the outbox.
fn with_email(c: &mut Config) {
    c.email = Some(slop_api::config::EmailConfig {
        smtp_url: "smtp://localhost:2525".into(),
        from: "Slop <notifications@slop.example>".into(),
    });
}

/// Emails waiting to go out: (to, subject, body).
async fn outbox(app: &TestApp) -> Vec<(String, String, String)> {
    sqlx::query_as("select to_address, subject, body from email_outbox order by created_at")
        .fetch_all(&app.db)
        .await
        .unwrap()
}

/// The titles of a user's notifications, oldest first.
async fn notice_titles(app: &TestApp, user: Uuid) -> Vec<String> {
    sqlx::query_scalar("select title from notifications where user_id = $1 order by created_at")
        .bind(user)
        .fetch_all(&app.db)
        .await
        .unwrap()
}

/// Every audit log action, oldest first.
async fn audit_actions(app: &TestApp) -> Vec<String> {
    sqlx::query_scalar("select action from audit_log order by id")
        .fetch_all(&app.db)
        .await
        .unwrap()
}

/// A report body with the details and the good-faith statement.
fn report_of(kind: &str, subject: &str, reason: &str) -> Value {
    json!({ "kind": kind, "subject": subject, "reason": reason, "details": "Please look at this.", "good_faith": true })
}

async fn send_report(app: &TestApp, cookie: Option<&str>, body: Value) -> Response {
    let mut r = req(Method::POST, "/v1/reports").json(body);
    if let Some(c) = cookie {
        r = r.cookie(c);
    }
    app.send(r).await
}

/// The newest open report's ID.
async fn newest_report(app: &TestApp) -> Uuid {
    sqlx::query_scalar(
        "select id from reports where status = 'open' order by created_at desc limit 1",
    )
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// A creative with a live People page (an approved portfolio).
async fn live_creative_row(app: &TestApp, user: Uuid, handle: &str) {
    sqlx::query(
        "insert into creatives (user_id, handle, name, ai_use, trader) values ($1, $2, $2, 'none', false)",
    )
    .bind(user)
    .bind(handle)
    .execute(&app.db)
    .await
    .unwrap();
    let set = Uuid::now_v7();
    sqlx::query(
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, status, source)
         values ($1, $2, 'https://github.com/octo/game', repeat('a', 40), '[]', 'f', 'approved', 'creative')",
    )
    .bind(set)
    .bind(user)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update creatives set portfolio_set_id = $1 where user_id = $2")
        .bind(set)
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
}

/// `client` hired `creative` (who has a live People page as `handle`) for a job on their app
/// `slug`; the job is done and the client reviewed it. Returns the job's ID.
async fn reviewed_job(
    app: &TestApp,
    creative: Uuid,
    handle: &str,
    client: Uuid,
    slug: &str,
) -> Uuid {
    live_creative_row(app, creative, handle).await;
    listed_with(app, client, slug, "released", "").await;
    let job = Uuid::now_v7();
    sqlx::query(
        "insert into jobs (id, creative_id, client_id, app_id, skill, status, request_md)
         select $1, $2, $3, id, 'music', 'completed', 'A theme song, please.' from apps where slug = $4",
    )
    .bind(job)
    .bind(creative)
    .bind(client)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into creative_reviews (job_id, creative_id, client_id, vote, body)
         values ($1, $2, $3, -1, 'Late, and rude about it.')",
    )
    .bind(job)
    .bind(creative)
    .bind(client)
    .execute(&app.db)
    .await
    .unwrap();
    job
}

/// A player team made by `owner`.
async fn team_of(app: &TestApp, owner: Uuid, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into teams (id, slug, name, kind, created_by) values ($1, $2, 'The ' || $2, 'player', $3)",
    )
    .bind(id)
    .bind(slug)
    .bind(owner)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("insert into team_members (team_id, user_id, role) values ($1, $2, 'owner')")
        .bind(id)
        .bind(owner)
        .execute(&app.db)
        .await
        .unwrap();
    id
}

/// A review of `slug` by `author`, with the maker's reply under it when `reply` is set.
async fn app_review(app: &TestApp, author: Uuid, slug: &str, reply: Option<&str>) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into app_reviews (id, user_id, app_id, body, reply, reply_at)
         select $1, $2, id, 'Crashes on start.', $4::text, case when $4::text is null then null else now() end
         from apps where slug = $3",
    )
    .bind(id)
    .bind(author)
    .bind(slug)
    .bind(reply)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

async fn reasons_for(app: &TestApp, query: &str) -> (StatusCode, Value) {
    let res = app
        .send(req(Method::GET, format!("/v1/reports/reasons{query}")))
        .await;
    let status = res.status();
    (status, body_json(res).await)
}

fn codes(list: &Value) -> Vec<&str> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|r| r["code"].as_str().unwrap())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Reporting

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn report_reasons_depend_on_what_is_reported(db: PgPool) {
    let app = test_app(db).await;
    let (_, app_reasons) = reasons_for(&app, "?kind=app").await;
    assert_eq!(
        codes(&app_reasons),
        [
            "child_abuse",
            "intimate_image",
            "malware",
            "source_mismatch",
            "outside_code",
            "scam",
            "illegal",
            "sexual",
            "copyright",
            "license",
            "gambling",
            "misleading",
            "broken",
            "other"
        ]
    );
    // Urgent and serious reasons (priority 0 and 1) are flagged; the rest aren't.
    for r in app_reasons.as_array().unwrap() {
        let urgent = !matches!(
            r["code"].as_str().unwrap(),
            "license" | "gambling" | "misleading" | "broken" | "other"
        );
        assert_eq!(r["urgent"], urgent, "{r}");
        assert!(!r["label"].as_str().unwrap().is_empty());
    }
    let people = [
        "child_abuse",
        "threat",
        "intimate_image",
        "scam",
        "illegal",
        "sexual",
        "copyright",
        "harassment",
        "private_info",
        "misleading",
        "spam",
        "other",
    ];
    for kind in ["profile", "team"] {
        let (_, list) = reasons_for(&app, &format!("?kind={kind}")).await;
        assert_eq!(codes(&list), people, "{kind}");
    }
    let conversations = [
        "child_abuse",
        "threat",
        "intimate_image",
        "scam",
        "illegal",
        "sexual",
        "harassment",
        "private_info",
        "spam",
        "other",
    ];
    for kind in ["message", "chat_message", "bounty"] {
        let (_, list) = reasons_for(&app, &format!("?kind={kind}")).await;
        assert_eq!(codes(&list), conversations, "{kind}");
    }
    let reviews = [
        "child_abuse",
        "threat",
        "intimate_image",
        "scam",
        "illegal",
        "sexual",
        "copyright",
        "harassment",
        "private_info",
        "fake_review",
        "spam",
        "other",
    ];
    for kind in ["review", "app_review"] {
        let (_, list) = reasons_for(&app, &format!("?kind={kind}")).await;
        assert_eq!(codes(&list), reviews, "{kind}");
        let find = |code: &str| {
            list.as_array()
                .unwrap()
                .iter()
                .find(|r| r["code"] == code)
                .unwrap()
                .clone()
        };
        assert_eq!(find("threat")["urgent"], true);
        assert_eq!(find("spam")["urgent"], false);
    }
    // Something that can't be reported has no reasons; asking without a kind is a bad request.
    assert_eq!(
        reasons_for(&app, "?kind=weather").await,
        (StatusCode::OK, json!([]))
    );
    assert_eq!(reasons_for(&app, "").await.0, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn profiles_creative_reviews_and_teams_can_be_reported_until_hidden(db: PgPool) {
    let app = test_app(db).await;
    let (creative, _) = app.user("Cass Creative", "US").await;
    let (client, client_cookie) = app.user("Cliff Client", "US").await;
    let (_, reporter) = app.user("Rita Reporter", "US").await;
    let job = reviewed_job(&app, creative, "cass", client, "song-game").await;
    team_of(&app, client, "cliffs-crew").await;

    // A People page, by handle (any case).
    let res = send_report(&app, Some(&reporter), report_of("profile", "CASS", "scam")).await;
    assert_eq!(res.status(), StatusCode::OK);
    // A creative's review, by its job ID: about the client who wrote it.
    let res = send_report(
        &app,
        Some(&reporter),
        report_of("review", &job.to_string(), "harassment"),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    // A team, by its address: about the team's creator.
    let res = send_report(
        &app,
        Some(&reporter),
        report_of("team", "cliffs-crew", "misleading"),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let rows: Vec<(String, Uuid, String, i16)> = sqlx::query_as(
        "select subject_kind, subject_user_id, subject_label, priority from reports order by created_at",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        rows,
        [
            (
                "profile".to_string(),
                creative,
                "People profile: cass (@cass)".to_string(),
                1
            ),
            (
                "review".to_string(),
                client,
                "Review of cass by Cliff Client".to_string(),
                2
            ),
            (
                "team".to_string(),
                client,
                "Team: The cliffs-crew".to_string(),
                2
            ),
        ]
    );
    // The client can't report the review they wrote.
    let res = send_report(
        &app,
        Some(&client_cookie),
        report_of("review", &job.to_string(), "spam"),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "subject");

    // Hidden things, and things that aren't there, can't be reported.
    let (other, _) = app.user("Other", "US").await;
    sqlx::query("update creatives set hidden_reason = 'Spam' where user_id = $1")
        .bind(creative)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update creative_reviews set hidden_at = now() where job_id = $1")
        .bind(job)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update teams set hidden_reason = 'Spam' where slug = 'cliffs-crew'")
        .execute(&app.db)
        .await
        .unwrap();
    // A People page without an approved portfolio isn't live either.
    sqlx::query(
        "insert into creatives (user_id, handle, name, ai_use, trader) values ($1, 'draft-page', 'Draft', 'none', false)",
    )
    .bind(other)
    .execute(&app.db)
    .await
    .unwrap();
    let (_, second) = app.user("Second Reporter", "US").await;
    for (kind, subject) in [
        ("profile", "cass".to_string()),
        ("profile", "draft-page".to_string()),
        ("profile", "nobody".to_string()),
        ("review", job.to_string()),
        ("review", "not-a-uuid".to_string()),
        ("review", Uuid::now_v7().to_string()),
        ("team", "cliffs-crew".to_string()),
        ("team", "no-team".to_string()),
        ("bounty", Uuid::now_v7().to_string()),
        ("app_review", "x".to_string()),
    ] {
        let res = send_report(&app, Some(&second), report_of(kind, &subject, "spam")).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{kind} {subject}");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reports_are_checked_before_they_are_taken(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (_, reporter) = app.user("Rita Reporter", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;

    // Unknown kinds, and reasons that don't fit the kind, are refused.
    for body in [
        report_of("weather", "castle", "other"),
        report_of("review", "castle", "malware"),
        report_of("app", "castle", "fake_review"),
        report_of("app", "castle", "made_up"),
    ] {
        let res = send_report(&app, Some(&reporter), body.clone()).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body_json(res).await["error"]["field"], "reason", "{body}");
    }
    // Details: something, and at most 4000 characters.
    for details in [String::new(), "   ".into(), "x".repeat(4001)] {
        let mut body = report_of("app", "castle", "broken");
        body["details"] = json!(details);
        let res = send_report(&app, Some(&reporter), body).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "details");
    }

    // A copyright report carries what a DMCA notice needs, each part within its length. (Another
    // reporter: every try counts towards the hourly limit.)
    let (reporter_id, reporter) = app.user("Cora Copyright", "US").await;
    let notice = json!({
        "work": "My song \"Night Drive\" (2024)", "owner": "", "address": "1 Main St, Springfield",
        "phone": "+1 555 0100", "good_faith": true, "accurate_under_penalty": true, "signature": "Rita Reporter"
    });
    for (field, change) in [
        ("copyright.statements", json!({ "good_faith": false })),
        (
            "copyright.statements",
            json!({ "accurate_under_penalty": false }),
        ),
        ("copyright.work", json!({ "work": " " })),
        ("copyright.phone", json!({ "phone": "1".repeat(41) })),
        ("copyright.address", json!({ "address": "a".repeat(301) })),
        ("copyright.signature", json!({ "signature": "" })),
        ("copyright.owner", json!({ "owner": "o".repeat(201) })),
    ] {
        let mut c = notice.clone();
        for (k, v) in change.as_object().unwrap() {
            c[k] = v.clone();
        }
        let mut body = report_of("app", "castle", "copyright");
        body["copyright"] = c;
        let res = send_report(&app, Some(&reporter), body).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        assert_eq!(body_json(res).await["error"]["field"], field);
    }
    let mut body = report_of("app", "castle", "copyright");
    body["copyright"] = notice.clone();
    let res = send_report(&app, Some(&reporter), body).await;
    assert_eq!(res.status(), StatusCode::OK);
    let stored: Value = sqlx::query_scalar("select copyright from reports where reporter_id = $1")
        .bind(reporter_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stored, notice, "the notice is kept as sent");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn accounts_send_at_most_ten_reports_an_hour_and_twenty_a_day(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;

    // The hourly limit counts every try, even ones that are refused.
    let (_, busy) = app.user("Busy Reporter", "US").await;
    for _ in 0..10 {
        let res = send_report(&app, Some(&busy), report_of("app", "castle", "made_up")).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let res = send_report(&app, Some(&busy), report_of("app", "castle", "broken")).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers().get("retry-after").is_some());
    assert_eq!(body_json(res).await["error"]["code"], "rate_limited");
    // Signed out, there's no account to limit.
    let mut signed_out = report_of("app", "castle", "broken");
    signed_out["name"] = json!("Sam");
    signed_out["email"] = json!("sam@example.com");
    assert_eq!(
        send_report(&app, None, signed_out).await.status(),
        StatusCode::OK
    );

    // Twenty in a day is the most, whatever they were about.
    let (daily_id, daily) = app.user("Daily Reporter", "US").await;
    for i in 0..20 {
        sqlx::query(
            "insert into reports (id, reference, subject_kind, subject_id, subject_label, reason, details,
                                  reporter_id, priority, due_at)
             values ($1, $2, 'app', $3, 'App: other', 'broken', 'x', $4, 2, now() + interval '7 days')",
        )
        .bind(Uuid::now_v7())
        .bind(format!("R-TEST-{i:04}"))
        .bind(Uuid::now_v7())
        .bind(daily_id)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let res = send_report(&app, Some(&daily), report_of("app", "castle", "broken")).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("a lot of reports today")
    );
    sqlx::query(
        "update reports set created_at = now() - interval '25 hours' where reporter_id = $1",
    )
    .bind(daily_id)
    .execute(&app.db)
    .await
    .unwrap();
    let res = send_report(&app, Some(&daily), report_of("app", "castle", "broken")).await;
    assert_eq!(res.status(), StatusCode::OK, "yesterday's don't count");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn signed_out_reports_about_one_thing_join_the_one_waiting(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let from = |email: &str, slug: &str| {
        let mut body = report_of("app", slug, "broken");
        body["name"] = json!("Sam");
        body["email"] = json!(email);
        body
    };
    let send = |email: &str, slug: &str| {
        let body = from(email, slug);
        let app = &app;
        async move {
            let res = send_report(app, None, body).await;
            assert_eq!(res.status(), StatusCode::OK);
            body_json(res).await["reference"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let count = || async {
        sqlx::query_scalar::<_, i64>("select count(*) from reports")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };

    // The same address again (in any case) joins its own report.
    let first = send("sam@example.com", "castle").await;
    assert_eq!(send("SAM@Example.com", "castle").await, first);
    assert_eq!(count().await, 1);
    // A different thing, or a different person, is a new report.
    assert_ne!(send("sam@example.com", "tower").await, first);
    let other = send("ann@example.com", "castle").await;
    assert_ne!(other, first);
    assert_eq!(count().await, 3);
    // Once it's answered, the same address can report it again.
    sqlx::query("update reports set status = 'dismissed' where reference = $1")
        .bind(&first)
        .execute(&app.db)
        .await
        .unwrap();
    assert_ne!(send("sam@example.com", "castle").await, first);
    assert_eq!(count().await, 4);

    // A flood: after ten signed-out reports about one thing in a day, more join the oldest open
    // one.
    for i in 0..7 {
        send(&format!("p{i}@example.com"), "castle").await;
    }
    assert_eq!(
        count().await,
        11,
        "ten signed-out reports about the castle, one about the tower"
    );
    assert_eq!(
        send("late@example.com", "castle").await,
        other,
        "the oldest open one"
    );
    assert_eq!(count().await, 11);
    // Signed in, the account is the limit instead: a new report.
    let (_, rita) = app.user("Rita", "US").await;
    assert_eq!(
        send_report(&app, Some(&rita), report_of("app", "castle", "broken"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(count().await, 12);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_what_the_reporter_can_see_can_be_reported(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (_, us) = app.user("Uma", "US").await;
    let (_, gb) = app.user("Gil", "GB").await;
    listed_with(&app, maker_id, "gore-fest", "released", "min_age = 18").await;
    listed_with(
        &app,
        maker_id,
        "secret",
        "released",
        "visibility = 'private'",
    )
    .await;

    // An 18+ app is hidden in the UK's under-18 view, so it can't be reported from there.
    assert_eq!(
        send_report(&app, Some(&gb), report_of("app", "gore-fest", "sexual"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send_report(&app, Some(&us), report_of("app", "gore-fest", "sexual"))
            .await
            .status(),
        StatusCode::OK
    );
    // Signed out with no country: the under-18 view too.
    let mut signed_out = report_of("app", "gore-fest", "sexual");
    signed_out["name"] = json!("Sam");
    signed_out["email"] = json!("sam@example.com");
    assert_eq!(
        send_report(&app, None, signed_out.clone()).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(
            req(Method::POST, "/v1/reports")
                .header("x-country", "US")
                .json(signed_out)
        )
        .await
        .status(),
        StatusCode::OK
    );
    // Private Slop nobody shared with them.
    assert_eq!(
        send_report(&app, Some(&us), report_of("app", "secret", "scam"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Job messages: only the two sides, never their own, and only while the text is kept.
    let (creative, creative_cookie) = app.user("Cass Creative", "US").await;
    let (client, client_cookie) = app.user("Cliff Client", "US").await;
    let job = reviewed_job(&app, creative, "cass", client, "song-game").await;
    let message = Uuid::now_v7();
    sqlx::query(
        "insert into job_messages (id, job_id, author_id, body, sha256) values ($1, $2, $3, 'Pay me outside Slop.', 'h')",
    )
    .bind(message)
    .bind(job)
    .bind(creative)
    .execute(&app.db)
    .await
    .unwrap();
    let report_message = report_of("message", &message.to_string(), "scam");
    let mut anonymous = report_message.clone();
    anonymous["name"] = json!("Sam");
    anonymous["email"] = json!("sam@example.com");
    assert_eq!(
        send_report(&app, None, anonymous).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send_report(&app, Some(&us), report_message.clone())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = send_report(&app, Some(&creative_cookie), report_message.clone()).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "subject");
    let res = send_report(&app, Some(&client_cookie), report_message.clone()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let quoted: Option<String> =
        sqlx::query_scalar("select quoted from reports where subject_kind = 'message'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        quoted.as_deref(),
        Some("Pay me outside Slop."),
        "the message is kept as it was"
    );
    let second = Uuid::now_v7();
    sqlx::query(
        "insert into job_messages (id, job_id, author_id, body, sha256) values ($1, $2, $3, null, 'h')",
    )
    .bind(second)
    .bind(job)
    .bind(creative)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        send_report(
            &app,
            Some(&client_cookie),
            report_of("message", &second.to_string(), "scam")
        )
        .await
        .status(),
        StatusCode::CONFLICT,
        "its text was deleted"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_reviewer_can_report_the_makers_reply_under_their_own_review(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    let (quinn_id, quinn) = app.user("Quinn Player", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    let replied = app_review(&app, pat_id, "castle", Some("Learn to play.")).await;
    let plain = app_review(&app, quinn_id, "castle", None).await;

    // Pat reports the reply under their own review: allowed.
    let res = send_report(
        &app,
        Some(&pat),
        report_of("app_review", &replied.to_string(), "harassment"),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    // Quinn's review has no reply, so reporting it is reporting their own.
    let res = send_report(
        &app,
        Some(&quinn),
        report_of("app_review", &plain.to_string(), "harassment"),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // Once the reply is removed, Pat's review is only their own again.
    sqlx::query("update app_reviews set reply_hidden_at = now() where id = $1")
        .bind(replied)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update reports set status = 'dismissed'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = send_report(
        &app,
        Some(&pat),
        report_of("app_review", &replied.to_string(), "spam"),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // The maker can report a review (about its author).
    let res = send_report(
        &app,
        Some(&maker),
        report_of("app_review", &plain.to_string(), "fake_review"),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let about: Uuid = sqlx::query_scalar(
        "select subject_user_id from reports where status = 'open' and reason = 'fake_review'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(about, quinn_id);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn malware_and_source_reports_ask_for_a_rescan(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (_, rita) = app.user("Rita", "US").await;
    let (_, sam) = app.user("Sam", "US").await;
    for slug in ["miner", "fake-build", "scammy"] {
        listed_with(&app, maker_id, slug, "released", "").await;
    }
    for (cookie, slug, reason) in [
        (&rita, "miner", "malware"),
        (&sam, "miner", "malware"),
        (&rita, "fake-build", "source_mismatch"),
        (&rita, "scammy", "scam"),
    ] {
        assert_eq!(
            send_report(&app, Some(cookie), report_of("app", slug, reason))
                .await
                .status(),
            StatusCode::OK
        );
    }
    let mut asked: Vec<(String, String)> = sqlx::query_as(
        "select a.slug, r.reason from rescan_requests r join apps a on a.id = r.app_id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    asked.sort();
    assert_eq!(
        asked,
        [
            ("fake-build".to_string(), "source_mismatch".to_string()),
            ("miner".to_string(), "malware".to_string())
        ],
        "once per app, and not for other reasons"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reporters_see_only_their_own_reports_newest_first(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (_, rita) = app.user("Rita", "US").await;
    let (_, sam) = app.user("Sam", "US").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/reports")).await.status(),
        StatusCode::UNAUTHORIZED
    );
    send_report(&app, Some(&rita), report_of("app", "castle", "malware")).await;
    send_report(&app, Some(&sam), report_of("app", "castle", "scam")).await;
    send_report(&app, Some(&rita), report_of("app", "tower", "broken")).await;
    let tower_report: Uuid = sqlx::query_scalar("select id from reports where reason = 'broken'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/reports/{tower_report}/dismiss"),
            )
            .cookie(&admin)
            .json(json!({ "outcome": "It starts fine on our machines." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/reports").cookie(&rita))
            .await,
    )
    .await;
    let mine = mine.as_array().unwrap();
    assert_eq!(mine.len(), 2, "not Sam's");
    assert_eq!(mine[0]["subject_label"], "App: tower");
    assert_eq!(mine[0]["reason"], "broken");
    assert_eq!(mine[0]["reason_label"], "The build is broken");
    assert_eq!(mine[0]["status"], "dismissed");
    assert_eq!(mine[0]["outcome"], "It starts fine on our machines.");
    assert!(mine[0]["decided_at"].is_string());
    assert_eq!(
        mine[1]["reason_label"],
        "Malware, hidden mining or data theft"
    );
    assert_eq!(mine[1]["status"], "open");
    assert!(mine[1]["outcome"].is_null());
    assert!(mine[1]["decided_at"].is_null());
    let sams = body_json(
        app.send(req(Method::GET, "/v1/me/reports").cookie(&sam))
            .await,
    )
    .await;
    assert_eq!(sams.as_array().unwrap().len(), 1);
    assert_eq!(sams[0]["reason"], "scam");
}

// ---------------------------------------------------------------------------------------------
// Appeals

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn appeals_are_checked_and_copyright_appeals_are_counter_notices(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    listed_with(&app, maker_id, "copycat", "released", "").await;
    listed_with(&app, maker_id, "spammy", "released", "").await;
    for (slug, rule) in [("copycat", "copyright"), ("spammy", "spam")] {
        let res = app
            .send(
                req(Method::POST, format!("/v1/admin/apps/{slug}/delist"))
                    .cookie(&admin)
                    .json(json!({ "rule": rule, "reason": "Checked against the original." })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
    }
    let decision = |rule: &str| {
        let db = app.db.clone();
        let rule = rule.to_string();
        async move {
            sqlx::query_scalar::<_, Uuid>("select id from moderation_decisions where rule = $1")
                .bind(rule)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    let copyright = decision("copyright").await;
    let spam = decision("spam").await;
    let appeal = |id: Uuid, body: Value| {
        req(Method::POST, format!("/v1/me/decisions/{id}/appeal"))
            .cookie(&maker)
            .json(body)
    };

    // The appeal itself: something, at most 4000 characters.
    for text in [String::new(), "  ".into(), "x".repeat(4001)] {
        let res = app.send(appeal(spam, json!({ "appeal": text }))).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "appeal");
    }
    assert_eq!(
        app.send(appeal(Uuid::now_v7(), json!({ "appeal": "Mistake." })))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // A copyright decision is appealed with a counter-notice, complete and within its lengths.
    let res = app
        .send(appeal(copyright, json!({ "appeal": "It's my own work." })))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "counter_notice");
    let counter = json!({
        "name": "Mia Maker", "address": "2 Side St, Springfield", "phone": "+1 555 0101",
        "mistake_under_penalty": true, "consent_to_jurisdiction": true, "signature": "Mia Maker"
    });
    for (field, change) in [
        (
            "counter_notice.statements",
            json!({ "mistake_under_penalty": false }),
        ),
        (
            "counter_notice.statements",
            json!({ "consent_to_jurisdiction": false }),
        ),
        ("counter_notice.name", json!({ "name": "n".repeat(101) })),
        ("counter_notice.address", json!({ "address": "" })),
        ("counter_notice.phone", json!({ "phone": "1".repeat(41) })),
        ("counter_notice.signature", json!({ "signature": " " })),
    ] {
        let mut c = counter.clone();
        for (k, v) in change.as_object().unwrap() {
            c[k] = v.clone();
        }
        let res = app
            .send(appeal(
                copyright,
                json!({ "appeal": "It's my own work.", "counter_notice": c }),
            ))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        assert_eq!(body_json(res).await["error"]["field"], field);
    }
    let res = app
        .send(appeal(
            copyright,
            json!({ "appeal": "It's my own work.", "counter_notice": counter }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["can_appeal"], false);
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin))
            .await,
    )
    .await;
    assert_eq!(queue["appeals"][0]["counter_notice"], counter);

    // Six months on, it's too late.
    sqlx::query(
        "update moderation_decisions set created_at = now() - interval '184 days' where id = $1",
    )
    .bind(spam)
    .execute(&app.db)
    .await
    .unwrap();
    let decisions = body_json(
        app.send(req(Method::GET, "/v1/me/decisions").cookie(&maker))
            .await,
    )
    .await;
    let old = decisions
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == spam.to_string())
        .unwrap()
        .clone();
    assert_eq!(old["can_appeal"], false);
    let res = app
        .send(appeal(spam, json!({ "appeal": "I missed the email." })))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

// ---------------------------------------------------------------------------------------------
// Acting on reports, and the decisions it makes

/// A bounty on `slug` by `author`, with one saved pledge from `backer`.
async fn bounty_on(app: &TestApp, slug: &str, author: Uuid, backer: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into bounties (id, app_id, created_by, title, body, currency)
         select $1, id, $2, 'Fix the save bug', 'Saves vanish after level 3.', 'usd' from apps where slug = $3",
    )
    .bind(id)
    .bind(author)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into bounty_pledges (id, bounty_id, user_id, amount_minor, status) values ($1, $2, $3, 500, 'active')",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(backer)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

/// Acts on the newest open report as `cookie`; returns the response.
async fn act_on_newest(app: &TestApp, cookie: &str, body: Value) -> Response {
    let id = newest_report(app).await;
    app.send(
        req(Method::POST, format!("/v1/admin/reports/{id}/act"))
            .cookie(cookie)
            .json(body),
    )
    .await
}

fn decision_body(action: &str, rule: &str) -> Value {
    json!({ "action": action, "rule": rule, "facts": "Checked it ourselves.", "outcome": "We acted on it." })
}

/// The decision's user appeals, and an admin reverses it.
async fn appeal_and_reverse(app: &TestApp, decision: &str, user: &str, admin: &str) {
    let res = app
        .send(
            req(Method::POST, format!("/v1/me/decisions/{decision}/appeal"))
                .cookie(user)
                .json(json!({ "appeal": "That was a mistake." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{decision}");
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/decisions/{decision}/appeal"),
            )
            .cookie(admin)
            .json(json!({ "reverse": true, "reason": "We looked again: it's fine." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT, "{decision}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn each_kind_of_decision_is_carried_out_and_undone_on_appeal(db: PgPool) {
    let app = test_app_with(db, |c| c.rate_limits.write_per_minute = 10_000).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (creative, creative_cookie) = app.user("Cass Creative", "US").await;
    let (client, client_cookie) = app.user("Cliff Client", "US").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    let job = reviewed_job(&app, creative, "cass", client, "song-game").await;
    sqlx::query(
        "insert into review_disputes (job_id, reason, statement, status, decision_reason)
         values ($1, 'rules', 'It insults me.', 'removed', 'It broke the rules.')",
    )
    .bind(job)
    .execute(&app.db)
    .await
    .unwrap();
    team_of(&app, client, "cliffs-crew").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    let replied = app_review(&app, pat_id, "castle", Some("Learn to play.")).await;
    let bounty = bounty_on(&app, "castle", pat_id, client).await;
    let decide = |kind: &str, subject: String, reason: &str, action: &str, rule: &str| {
        let app = &app;
        let rita = &rita;
        let admin = &admin;
        let (kind, reason, action, rule) = (
            kind.to_string(),
            reason.to_string(),
            action.to_string(),
            rule.to_string(),
        );
        async move {
            let res = send_report(app, Some(rita), report_of(&kind, &subject, &reason)).await;
            assert_eq!(res.status(), StatusCode::OK, "{kind}");
            let res = act_on_newest(app, admin, decision_body(&action, &rule)).await;
            assert_eq!(res.status(), StatusCode::OK, "{kind} {action}");
            body_json(res).await["decision"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let one = |sql: &'static str| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>(sql)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };

    // A People page: hidden, with the rule as the reason.
    let profile = decide("profile", "cass".into(), "scam", "hide_profile", "scam").await;
    assert_eq!(
        one("select hidden_reason from creatives").await.as_deref(),
        Some("Scams and fraud")
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/people/cass")).await.status(),
        StatusCode::NOT_FOUND
    );
    // A team page.
    let team = decide(
        "team",
        "cliffs-crew".into(),
        "misleading",
        "hide_team",
        "misleading",
    )
    .await;
    assert_eq!(
        one("select hidden_reason from teams").await.as_deref(),
        Some("Misleading pages")
    );
    // A creative's review.
    let review = decide(
        "review",
        job.to_string(),
        "harassment",
        "hide_review",
        "harassment",
    )
    .await;
    assert!(
        one("select hidden_at::text from creative_reviews")
            .await
            .is_some()
    );
    // The maker's reply under a review: a decision about the maker, the review stays.
    let reply = decide(
        "app_review",
        replied.to_string(),
        "harassment",
        "remove_reply",
        "harassment",
    )
    .await;
    let (reply_hidden, review_hidden): (bool, bool) = sqlx::query_as(
        "select reply_hidden_at is not null, hidden_at is not null from app_reviews where id = $1",
    )
    .bind(replied)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((reply_hidden, review_hidden), (true, false));
    let about: Uuid =
        sqlx::query_scalar("select user_id from moderation_decisions where id = $1::uuid")
            .bind(&reply)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(about, maker_id);
    // With the reply gone there's nothing left to remove.
    send_report(
        &app,
        Some(&maker),
        report_of("app_review", &replied.to_string(), "fake_review"),
    )
    .await;
    let res = act_on_newest(&app, &admin, decision_body("remove_reply", "fake_review")).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "action");
    // A player's app review: hidden.
    let res = act_on_newest(&app, &admin, decision_body("hide_review", "fake_review")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let app_review_decision = body_json(res).await["decision"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        one("select hidden_at::text from app_reviews")
            .await
            .is_some()
    );
    // A bounty: hidden, and its pledges released.
    let bounty_decision = decide("bounty", bounty.to_string(), "scam", "hide_bounty", "scam").await;
    let (status, pledge): (String, String) = sqlx::query_as(
        "select b.status, p.status from bounties b join bounty_pledges p on p.bounty_id = b.id",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((status.as_str(), pledge.as_str()), ("hidden", "released"));
    // A warning changes nothing but is a decision like the rest.
    let warning = decide("app", "castle".into(), "misleading", "warn", "misleading").await;
    assert_eq!(
        one("select status::text from apps where slug = 'castle'")
            .await
            .as_deref(),
        Some("listed")
    );

    // Each person is told, with the action and the rule, and each decision is in the audit log.
    assert!(
        notice_titles(&app, creative)
            .await
            .contains(&"Profile hidden: People profile: cass (@cass)".to_string())
    );
    assert!(
        notice_titles(&app, client)
            .await
            .contains(&"Team hidden: Team: The cliffs-crew".to_string())
    );
    assert!(
        notice_titles(&app, maker_id)
            .await
            .contains(&"Warning: App: castle".to_string())
    );
    let actions = audit_actions(&app).await;
    for action in [
        "moderation.hide_profile",
        "moderation.hide_team",
        "moderation.hide_review",
        "moderation.remove_reply",
        "moderation.hide_bounty",
        "moderation.warn",
    ] {
        assert!(actions.contains(&action.to_string()), "{action}");
    }

    // Reversed on appeal, each is undone.
    appeal_and_reverse(&app, &profile, &creative_cookie, &admin).await;
    assert_eq!(one("select hidden_reason from creatives").await, None);
    appeal_and_reverse(&app, &team, &client_cookie, &admin).await;
    assert_eq!(one("select hidden_reason from teams").await, None);
    appeal_and_reverse(&app, &review, &client_cookie, &admin).await;
    assert_eq!(
        one("select hidden_at::text from creative_reviews").await,
        None
    );
    assert_eq!(
        one("select status || ': ' || decision_reason from review_disputes")
            .await
            .as_deref(),
        Some("kept: It broke the rules. (reversed on appeal)"),
        "the dispute that removed it is reversed too"
    );
    appeal_and_reverse(&app, &reply, &maker, &admin).await;
    assert_eq!(
        one("select reply_hidden_at::text from app_reviews").await,
        None
    );
    appeal_and_reverse(&app, &app_review_decision, &pat, &admin).await;
    assert_eq!(one("select hidden_at::text from app_reviews").await, None);
    appeal_and_reverse(&app, &bounty_decision, &pat, &admin).await;
    assert_eq!(
        one("select status || coalesce(hidden_reason, '') from bounties")
            .await
            .as_deref(),
        Some("open")
    );
    appeal_and_reverse(&app, &warning, &maker, &admin).await;
    let reversed: i64 = sqlx::query_scalar(
        "select count(*) from moderation_decisions where appeal_status = 'reversed' and reversed_at is not null",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(reversed, 7);
}

/// A bounty hidden by mistake comes back as it was: open for pledges until its old end, not
/// expired the moment it's restored.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
#[ignore = "n01: reversing hide_bounty leaves expires_at at the moment it was hidden and its pledges released, so the restored bounty expires at the charger's next pass"]
async fn a_bounty_restored_on_appeal_is_still_open_for_its_backers(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    let (backer, _) = app.user("Bea Backer", "US").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    let bounty = bounty_on(&app, "castle", pat_id, backer).await;
    send_report(
        &app,
        Some(&rita),
        report_of("bounty", &bounty.to_string(), "scam"),
    )
    .await;
    let res = act_on_newest(&app, &admin, decision_body("hide_bounty", "scam")).await;
    let decision = body_json(res).await["decision"]
        .as_str()
        .unwrap()
        .to_string();
    appeal_and_reverse(&app, &decision, &pat, &admin).await;
    let (open, pledges): (bool, String) = sqlx::query_as(
        "select b.status = 'open' and b.expires_at > now() + interval '1 day', p.status
         from bounties b join bounty_pledges p on p.bounty_id = b.id",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(open, "still open after the next expiry pass");
    assert_eq!(pledges, "active", "the backer's pledge stands again");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reporters_are_told_the_outcome_in_slop_or_by_email(db: PgPool) {
    let app = test_app_with(db, with_email).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (rita_id, rita) = app.user("Rita Reporter", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    send_report(&app, Some(&rita), report_of("app", "castle", "illegal")).await;
    let mut signed_out = report_of("app", "castle", "illegal");
    signed_out["name"] = json!("Sam");
    signed_out["email"] = json!("sam@example.com");
    let sams = body_json(send_report(&app, None, signed_out.clone()).await).await["reference"]
        .as_str()
        .unwrap()
        .to_string();
    // A signed-out child abuse report without an address can't be told anything.
    let anonymous = report_of("app", "castle", "child_abuse");
    assert_eq!(
        send_report(&app, None, anonymous).await.status(),
        StatusCode::OK
    );
    // A report about something else stays open.
    send_report(&app, Some(&rita), report_of("app", "tower", "broken")).await;
    let castle: Uuid = sqlx::query_scalar(
        "select id from reports where reporter_id is not null and reason = 'illegal'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();

    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/reports/{castle}/act"))
                .cookie(&admin)
                .json(json!({
                    "action": "delist_app", "rule": "illegal", "law": "18 U.S.C. §1030",
                    "facts": "The download is a password stealer.", "outcome": "We took it out of the store.",
                    "external_reference": "IC3-2026-0042"
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let rows: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "select reason, status, outcome, external_reference from reports order by created_at",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let outcome = Some("We took it out of the store.".to_string());
    let reference = Some("IC3-2026-0042".to_string());
    assert_eq!(
        rows,
        [
            (
                "illegal".into(),
                "actioned".into(),
                outcome.clone(),
                reference.clone()
            ),
            (
                "illegal".into(),
                "actioned".into(),
                outcome.clone(),
                reference.clone()
            ),
            ("child_abuse".into(), "actioned".into(), outcome, reference),
            ("broken".into(), "open".into(), None, None),
        ],
        "every open report about the castle closes with the decision"
    );
    let told = notice_titles(&app, rita_id).await;
    assert_eq!(told.len(), 1);
    assert!(
        told[0].starts_with("Your report R-") && told[0].ends_with(" was answered"),
        "{told:?}"
    );
    let mail = outbox(&app).await;
    assert_eq!(mail.len(), 1, "only the reporter who left an address");
    assert_eq!(mail[0].0, "sam@example.com");
    assert_eq!(mail[0].1, format!("Your report {sams} was answered"));
    assert!(mail[0].2.contains("You reported App: castle to Slop."));
    assert!(
        mail[0]
            .2
            .contains("What we decided: We took it out of the store.")
    );
    let law: Option<String> = sqlx::query_scalar("select law from moderation_decisions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(law.as_deref(), Some("18 U.S.C. §1030"));
    let actions = audit_actions(&app).await;
    assert!(actions.contains(&"moderation.delist_app".to_string()));

    // The report isn't open any more.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/reports/{castle}/act"))
                .cookie(&admin)
                .json(decision_body("warn", "illegal")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn acting_on_reports_needs_moderation_and_suspending_needs_two_people(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.approvals_required = 2;
        c.rate_limits.write_per_minute = 10_000;
    })
    .await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, second) = make_admin(&app, "Sid Second").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, finance) = staff(&app, "Fay Finance", "finance").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    let (_, sam) = app.user("Sam Reporter", "US").await;
    listed_with(&app, maker_id, "scammy", "released", "").await;
    send_report(&app, Some(&rita), report_of("app", "scammy", "scam")).await;
    let report = newest_report(&app).await;
    let act = |who: Req, body: Value| {
        let r = who;
        let uri = format!("/v1/admin/reports/{report}/act");
        Req { uri, ..r }.json(body)
    };
    let as_cookie = |c: &str| req(Method::POST, "").cookie(c);
    let as_token = |t: &str| req(Method::POST, "").header("authorization", t.to_string());
    let warn = decision_body("warn", "scam");
    let suspend = decision_body("suspend_account", "scam");

    // Not staff: not there. Staff without moderation rights, or a read-only token: refused.
    assert_eq!(
        app.send(act(as_cookie(&maker), warn.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    for who in [&reviewer, &finance] {
        assert_eq!(
            app.send(act(as_cookie(who), warn.clone())).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    let read = token(&app, &owner, "read").await;
    assert_eq!(
        app.send(act(as_token(&read), warn.clone())).await.status(),
        StatusCode::FORBIDDEN
    );
    // Support handles reports, but suspending someone is sensitive.
    assert_eq!(
        app.send(act(as_cookie(&support), suspend.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let review_token = token(&app, &owner, "review").await;
    assert_eq!(
        app.send(act(as_token(&review_token), suspend.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN,
        "a review token can't ask for a suspension"
    );
    // A rule that isn't one is refused, and nothing happens.
    let res = app
        .send(act(as_cookie(&support), decision_body("warn", "rudeness")))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rule")
    );
    let decisions: i64 = sqlx::query_scalar("select count(*) from moderation_decisions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(decisions, 0);

    // The owner asks to suspend the maker: it waits for a second person.
    let res = app.send(act(as_cookie(&owner), suspend.clone())).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let pending = body_json(res).await["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    let suspended = || async {
        sqlx::query_scalar::<_, bool>("select suspended_at is not null from users where id = $1")
            .bind(maker_id)
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    assert!(!suspended().await);
    let open: String = sqlx::query_scalar("select status from reports where id = $1")
        .bind(report)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(open, "open");
    // A report that isn't open can't be asked about.
    send_report(&app, Some(&sam), report_of("app", "scammy", "illegal")).await;
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/reports/{}/act", Uuid::now_v7()),
            )
            .cookie(&owner)
            .json(suspend.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    // The second owner approves: the maker is suspended and every report about the app closes.
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{pending}/approve")).cookie(&second))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_json(res).await["decision"].is_string());
    assert!(suspended().await);
    let open: i64 = sqlx::query_scalar("select count(*) from reports where status = 'open'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(open, 0);
    let (kind, actor): (String, String) =
        sqlx::query_as("select subject_kind, decided_by from moderation_decisions")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(kind, "account");
    assert!(
        actor.contains("Olive Owner") && actor.contains("approved by Sid Second"),
        "{actor}"
    );
    // Now nothing's open to act on.
    assert_eq!(
        app.send(act(as_cookie(&support), warn.clone()))
            .await
            .status(),
        StatusCode::CONFLICT
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn dismissing_a_report_tells_the_reporter_why(db: PgPool) {
    let app = test_app_with(db, with_email).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (rita_id, rita) = app.user("Rita Reporter", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    send_report(&app, Some(&rita), report_of("app", "castle", "broken")).await;
    let report = newest_report(&app).await;
    let dismiss = |who: Req, body: Value| {
        Req {
            uri: format!("/v1/admin/reports/{report}/dismiss"),
            ..who
        }
        .json(body)
    };
    let as_cookie = |c: &str| req(Method::POST, "").cookie(c);
    let fine = json!({ "outcome": "It runs fine for us; try updating your drivers." });

    assert_eq!(
        app.send(dismiss(as_cookie(&maker), fine.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(dismiss(as_cookie(&reviewer), fine.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let read = token(&app, &owner, "read").await;
    assert_eq!(
        app.send(dismiss(
            req(Method::POST, "").header("authorization", read),
            fine.clone()
        ))
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    for outcome in [String::new(), " ".into(), "x".repeat(2001)] {
        let res = app
            .send(dismiss(as_cookie(&support), json!({ "outcome": outcome })))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "outcome");
    }
    let res = app
        .send(dismiss(
            as_cookie(&support),
            json!({ "outcome": "It runs fine for us; try updating your drivers.", "external_reference": "T-1" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (status, outcome, by, external): (String, String, String, Option<String>) = sqlx::query_as(
        "select status, outcome, decided_by, external_reference from reports where id = $1",
    )
    .bind(report)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(status, "dismissed");
    assert_eq!(outcome, "It runs fine for us; try updating your drivers.");
    assert!(by.starts_with("Sue Support ("));
    assert_eq!(external.as_deref(), Some("T-1"));
    let told: Vec<(String, String)> =
        sqlx::query_as("select title, body from notifications where user_id = $1")
            .bind(rita_id)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(told.len(), 1);
    assert!(told[0].0.ends_with("was answered"));
    assert_eq!(
        told[0].1,
        "App: castle: It runs fine for us; try updating your drivers."
    );
    let decisions: i64 = sqlx::query_scalar("select count(*) from moderation_decisions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(decisions, 0, "nothing was decided about the maker");
    assert!(notice_titles(&app, maker_id).await.is_empty());
    let (action, subject): (String, String) =
        sqlx::query_as("select action, subject from audit_log where action = 'report.dismiss'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (action.as_str(), subject),
        ("report.dismiss", report.to_string())
    );
    // Once.
    assert_eq!(
        app.send(dismiss(as_cookie(&support), fine.clone()))
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // A signed-out reporter is emailed; a review token may dismiss.
    let mut signed_out = report_of("app", "castle", "broken");
    signed_out["name"] = json!("Sam");
    signed_out["email"] = json!("sam@example.com");
    send_report(&app, None, signed_out).await;
    let second = newest_report(&app).await;
    let review = token(&app, &owner, "review").await;
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/reports/{second}/dismiss"))
                .header("authorization", review)
                .json(fine),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let mail = outbox(&app).await;
    assert_eq!(mail.len(), 1);
    assert_eq!(mail[0].0, "sam@example.com");
    assert!(mail[0].2.contains("What we decided: It runs fine for us"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn appeals_are_kept_or_reversed_with_a_reason(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    app.send(
        req(Method::POST, "/v1/admin/apps/castle/delist")
            .cookie(&owner)
            .json(
                json!({ "rule": "misleading", "reason": "The screenshots are of another game." }),
            ),
    )
    .await;
    let decision: Uuid = sqlx::query_scalar("select id from moderation_decisions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let decide = |cookie: &str, body: Value| {
        req(
            Method::POST,
            format!("/v1/admin/decisions/{decision}/appeal"),
        )
        .cookie(cookie)
        .json(body)
    };
    let keep = json!({ "reverse": false, "reason": "The screenshots still aren't of this game." });

    // Nothing to decide before the maker appeals.
    assert_eq!(
        app.send(decide(&owner, keep.clone())).await.status(),
        StatusCode::CONFLICT
    );
    app.send(
        req(Method::POST, format!("/v1/me/decisions/{decision}/appeal"))
            .cookie(&maker)
            .json(json!({ "appeal": "They're from the beta." })),
    )
    .await;
    assert_eq!(
        app.send(decide(&maker, keep.clone())).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(decide(&reviewer, keep.clone())).await.status(),
        StatusCode::FORBIDDEN
    );
    for reason in ["", "  "] {
        let res = app
            .send(decide(
                &support,
                json!({ "reverse": false, "reason": reason }),
            ))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let res = app
        .send(decide(
            &support,
            json!({ "reverse": false, "reason": "x".repeat(2001) }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Kept: nothing is undone, and the maker is told why.
    assert_eq!(
        app.send(decide(&support, keep.clone())).await.status(),
        StatusCode::NO_CONTENT
    );
    let status: String = sqlx::query_scalar("select status::text from apps where slug = 'castle'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "delisted");
    let told: (String, String) = sqlx::query_as(
        "select title, body from notifications where user_id = $1 order by created_at desc limit 1",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        told,
        (
            "Your appeal about App: castle: the decision stands".to_string(),
            "The screenshots still aren't of this game.".to_string()
        )
    );
    let decisions = body_json(
        app.send(req(Method::GET, "/v1/me/decisions").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(decisions[0]["appeal_status"], "upheld");
    assert!(decisions[0]["reversed_at"].is_null());
    assert_eq!(decisions[0]["can_appeal"], false, "an appeal is once");
    assert!(
        audit_actions(&app)
            .await
            .contains(&"appeal.uphold".to_string())
    );
    // Decided once.
    assert_eq!(
        app.send(decide(&support, keep)).await.status(),
        StatusCode::CONFLICT
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_suspended_admin_gets_the_account_back_on_appeal_but_not_admin_rights(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (helper_id, helper) = staff(&app, "Hal Helper", "support").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    listed_with(&app, helper_id, "helper-game", "released", "").await;
    send_report(&app, Some(&rita), report_of("app", "helper-game", "scam")).await;
    let res = act_on_newest(&app, &owner, decision_body("suspend_account", "scam")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let decision = body_json(res).await["decision"]
        .as_str()
        .unwrap()
        .to_string();
    let state = || async {
        sqlx::query_as::<_, (bool, bool)>(
            "select suspended_at is not null, is_admin from users where id = $1",
        )
        .bind(helper_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    assert_eq!(
        state().await,
        (true, false),
        "suspended, and no longer staff"
    );
    appeal_and_reverse(&app, &decision, &helper, &owner).await;
    assert_eq!(
        state().await,
        (false, false),
        "back, but admin rights don't come back on their own"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_decision_that_cannot_be_carried_out_leaves_no_trace(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (creative, _) = app.user("Cass Creative", "US").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    live_creative_row(&app, creative, "cass").await;
    listed_with(&app, maker_id, "draft-game", "released", "status = 'draft'").await;
    let before = audit_actions(&app).await;

    // Hiding a profile under a rule that doesn't exist: the profile isn't hidden either.
    send_report(&app, Some(&rita), report_of("profile", "cass", "spam")).await;
    let res = act_on_newest(&app, &owner, decision_body("hide_profile", "not_a_rule")).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let hidden: Option<String> =
        sqlx::query_scalar("select hidden_reason from creatives where user_id = $1")
            .bind(creative)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(hidden, None);
    // Taking down an app that isn't in the store (a removal order names a draft).
    let res = app
        .send(
            req(Method::POST, "/v1/admin/removal-orders")
                .cookie(&owner)
                .json(json!({
                    "received_at": OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
                    "country": "FR", "authority": "OCLCTIC", "order": "Order 9",
                    "subject_kind": "app", "subject": "draft-game"
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("isn't in the store")
    );
    let (decisions, notices, requests): (i64, i64, i64) = sqlx::query_as(
        "select (select count(*) from moderation_decisions), (select count(*) from notifications),
                (select count(*) from legal_requests)",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((decisions, notices, requests), (0, 0, 0));
    assert_eq!(audit_actions(&app).await, before);
    // The CLI's delist refuses an app that isn't listed, and empty facts.
    assert!(
        admin::delist_app(&app.db, "draft-game", "cli", "misleading", "Fake page.")
            .await
            .is_err()
    );
    listed_with(&app, maker_id, "live-game", "released", "").await;
    assert!(
        admin::delist_app(&app.db, "live-game", "cli", "misleading", "  ")
            .await
            .is_err()
    );
    assert!(
        admin::delist_app(&app.db, "live-game", "cli", "no_such_rule", "Fake page.")
            .await
            .is_err()
    );
    let status: String =
        sqlx::query_scalar("select status::text from apps where slug = 'live-game'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "listed");
}

// ---------------------------------------------------------------------------------------------
// The review queue: releases, scans, listing, stages, changes, pictures, portfolios

/// A small stand-in for code hosts: GitHub's repository API (`/repos/x/broken` errors,
/// `/repos/x/closed` is private, `/repos/x/gone` isn't there, anything else is public), and Git's
/// discovery request on another host answering with the status its first path segment names
/// (`/403/repo/info/refs`).
async fn fake_host() -> String {
    let app = Router::new()
        .route(
            "/repos/{owner}/{repo}",
            get(
                |axum::extract::Path((_, repo)): axum::extract::Path<(String, String)>| async move {
                    match repo.as_str() {
                        "broken" => StatusCode::BAD_GATEWAY.into_response(),
                        "gone" => StatusCode::NOT_FOUND.into_response(),
                        "closed" => axum::Json(json!({ "private": true })).into_response(),
                        _ => axum::Json(json!({ "private": false })).into_response(),
                    }
                },
            ),
        )
        .route(
            "/{status}/{repo}/info/refs",
            get(
                |axum::extract::Path((status, _)): axum::extract::Path<(u16, String)>| async move {
                    (
                        StatusCode::from_u16(status).unwrap(),
                        "001e# service=git-upload-pack\n",
                    )
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

/// The maker's release of `game` (maker_with_app), fetched and waiting for review.
async fn release_waiting(app: &TestApp) -> (Uuid, String, String) {
    let (maker_id, maker) = maker_with_app(app, "released").await;
    let id = submit_and_fetch(app, &maker, "1.0.0").await;
    (maker_id, maker, id)
}

async fn release_state(app: &TestApp, id: &str) -> (String, Option<String>) {
    sqlx::query_as("select status::text, status_reason from releases where id = $1::uuid")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// A request as a signed-in person or with a token ("Bearer …").
fn as_who(method: Method, uri: String, who: &str) -> Req {
    if who.starts_with("Bearer ") {
        req(method, uri).header("authorization", who.to_string())
    } else {
        req(method, uri).cookie(who)
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn approving_a_release_signs_it_and_copies_its_rating_to_the_store_page(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker_id, _, id) = release_waiting(&app).await;
    let approve = |who: &str| {
        as_who(
            Method::POST,
            format!("/v1/admin/releases/{id}/approve"),
            who,
        )
    };
    let read = token(&app, &owner, "read").await;
    assert_eq!(
        app.send(approve(&read)).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(approve(&support)).await.status(),
        StatusCode::FORBIDDEN
    );
    let refusal = |res: Response| async move {
        assert_eq!(res.status(), StatusCode::CONFLICT);
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // A frozen app's releases wait.
    sqlx::query("update apps set frozen_at = now(), frozen_reason = 'Checking a report' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let why = refusal(app.send(approve(&owner)).await).await;
    assert!(why.contains("frozen (Checking a report)"), "{why}");
    sqlx::query("update apps set frozen_at = null, frozen_reason = null")
        .execute(&app.db)
        .await
        .unwrap();
    // A file that was never hashed can't be signed.
    sqlx::query("update release_files set sha256 = null where release_id = $1::uuid")
        .bind(&id)
        .execute(&app.db)
        .await
        .unwrap();
    let why = refusal(app.send(approve(&owner)).await).await;
    assert!(why.contains("windows build hasn't been hashed"), "{why}");
    sqlx::query("update release_files set sha256 = repeat('b', 64) where release_id = $1::uuid")
        .bind(&id)
        .execute(&app.db)
        .await
        .unwrap();

    // The release carries an age rating and content labels: approving puts them on the page.
    sqlx::query(
        "update releases set min_age = 16, content_labels = '{violence,horror}' where id = $1::uuid",
    )
    .bind(&id)
    .execute(&app.db)
    .await
    .unwrap();
    let review = token(&app, &owner, "review").await;
    assert_eq!(
        app.send(approve(&review)).await.status(),
        StatusCode::NO_CONTENT
    );
    let (min_age, labels): (i16, Vec<String>) =
        sqlx::query_as("select min_age, content_labels from apps where slug = 'game'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (min_age, labels),
        (16, vec!["violence".into(), "horror".into()])
    );
    let (status, reviewed_by, signed): (String, String, bool) = sqlx::query_as(
        "select status::text, reviewed_by, signed_manifest is not null from releases where id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(status, "approved");
    assert!(
        reviewed_by.starts_with("AI reviewer \"review agent\""),
        "{reviewed_by}"
    );
    assert!(signed);
    assert!(
        notice_titles(&app, maker_id)
            .await
            .contains(&"Game 1.0.0 is approved".to_string())
    );
    // Not twice.
    let why = refusal(app.send(approve(&owner)).await).await;
    assert!(
        why.contains("release is approved, not awaiting review"),
        "{why}"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn without_a_signing_key_nothing_can_be_approved(db: PgPool) {
    let app = test_app_with(db, |c| c.releases.signing_key = None).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, _, id) = release_waiting(&app).await;
    let res = app
        .send(req(Method::POST, format!("/v1/admin/releases/{id}/approve")).cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("SLOP_SIGNING_KEY")
    );
    assert_eq!(release_state(&app, &id).await.0, "awaiting_review");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn rejecting_a_release_tells_the_maker_why(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker_id, maker, waiting) = release_waiting(&app).await;
    let reject = |id: &str, who: &str, reason: &str| {
        as_who(Method::POST, format!("/v1/admin/releases/{id}/reject"), who)
            .json(json!({ "reason": reason }))
    };

    // Who may: reviewers (and review tokens), not support, not read tokens, not players.
    assert_eq!(
        app.send(reject(&waiting, &maker, "No.")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(reject(&waiting, &support, "No.")).await.status(),
        StatusCode::FORBIDDEN
    );
    let read = token(&app, &owner, "read").await;
    assert_eq!(
        app.send(reject(&waiting, &read, "No.")).await.status(),
        StatusCode::FORBIDDEN
    );
    // A reason, for the maker.
    let res = app.send(reject(&waiting, &reviewer, "  ")).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(release_state(&app, &waiting).await.0, "awaiting_review");

    let res = app
        .send(reject(
            &waiting,
            &reviewer,
            " The Windows build asks for administrator rights. ",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        release_state(&app, &waiting).await,
        (
            "rejected".into(),
            Some("The Windows build asks for administrator rights.".into())
        )
    );
    let told: (String, String) = sqlx::query_as(
        "select title, body from notifications where user_id = $1 and title like '%wasn''t approved'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        told,
        (
            "Game 1.0.0 wasn't approved".to_string(),
            " The Windows build asks for administrator rights. ".to_string()
        )
    );
    let (actor, subject): (String, String) =
        sqlx::query_as("select actor, subject from audit_log where action = 'release.reject'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(actor.starts_with("Rex Reviewer ("));
    assert_eq!(subject, waiting);
    // Rejected already: not again.
    assert_eq!(
        app.send(reject(&waiting, &reviewer, "Again."))
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // Releases still being fetched, or whose checks failed, can be turned down too.
    let still_fetching = body_json(
        app.send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(release_body(&app, "1.1.0", "game-windows.zip")),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(release_state(&app, &still_fetching).await.0, "fetching");
    let review = token(&app, &owner, "review").await;
    assert_eq!(
        app.send(reject(&still_fetching, &review, "Wrong repository."))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let failed = submit_and_fetch(&app, &maker, "1.2.0").await;
    sqlx::query("update releases set status = 'failed' where id = $1::uuid")
        .bind(&failed)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(reject(&failed, &owner, "Broken download."))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    // (One more waiting, for the command line below: once a person approved a release, later
    // ones are approved automatically.)
    let fresh = submit_and_fetch(&app, &maker, "1.4.0").await;
    // An approved release is withdrawn, not rejected; the command line has the same rule.
    let approved = submit_and_fetch(&app, &maker, "1.3.0").await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        approved.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    assert_eq!(
        app.send(reject(&approved, &owner, "Changed my mind."))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert!(
        admin::reject(
            &app.db,
            approved.parse().unwrap(),
            "cli",
            "Changed my mind."
        )
        .await
        .is_err()
    );
    assert!(
        admin::reject(&app.db, Uuid::now_v7(), "cli", "Nothing there.")
            .await
            .is_err()
    );
    assert!(
        admin::reject(&app.db, fresh.parse().unwrap(), " ", "Who?")
            .await
            .is_err(),
        "the reviewer says who they are"
    );
    admin::reject(
        &app.db,
        fresh.parse().unwrap(),
        "cli",
        "From the command line.",
    )
    .await
    .unwrap();
    assert_eq!(
        release_state(&app, &fresh).await,
        ("rejected".into(), Some("From the command line.".into()))
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_reviewer_clears_a_false_positive_with_a_reason(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (_, _, id) = release_waiting(&app).await;
    sqlx::query(
        "update release_files set scan_result = 'on_hold', scan_detail = 'Generic.Trojan (heuristic)'
         where release_id = $1::uuid",
    )
    .bind(&id)
    .execute(&app.db)
    .await
    .unwrap();
    let approve = || req(Method::POST, format!("/v1/admin/releases/{id}/approve")).cookie(&owner);
    let res = app.send(approve()).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("clear it with a reason")
    );
    let clear = |platform: &str, who: &str, reason: &str| {
        as_who(
            Method::POST,
            format!("/v1/admin/releases/{id}/files/{platform}/clear"),
            who,
        )
        .json(json!({ "reason": reason }))
    };
    let reason = "The heuristic flags every Godot export; checked the binary by hand.";
    let read = token(&app, &owner, "read").await;
    for who in [&support, &read] {
        assert_eq!(
            app.send(clear("windows", who, reason)).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    // A reason of 10 to 1000 characters, for a file that's there.
    for short_or_long in ["too short", &"x".repeat(1001)] {
        assert_eq!(
            app.send(clear("windows", &reviewer, short_or_long))
                .await
                .status(),
            StatusCode::CONFLICT
        );
    }
    assert_eq!(
        app.send(clear("macos", &reviewer, reason)).await.status(),
        StatusCode::CONFLICT
    );
    let res = app.send(clear("windows", &reviewer, reason)).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (by, why): (String, String) = sqlx::query_as(
        "select cleared_by, cleared_reason from release_files where release_id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(by.starts_with("Rex Reviewer ("));
    assert_eq!(why, reason);
    let (subject, detail): (String, Value) =
        sqlx::query_as("select subject, detail from audit_log where action = 'release.clear_scan'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(subject, "game");
    assert_eq!(detail["platform"], "windows");
    assert_eq!(detail["reason"], reason);
    // Now it can be approved; after that, nothing is waiting to be cleared.
    assert_eq!(app.send(approve()).await.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        app.send(clear("windows", &reviewer, reason)).await.status(),
        StatusCode::CONFLICT
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_live_release_a_rescan_flagged_can_be_kept_with_a_reason(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (_, _, id) = release_waiting(&app).await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        id.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let keep = |platform: &str, who: &str, reason: &str| {
        as_who(
            Method::POST,
            format!("/v1/admin/releases/{id}/files/{platform}/rescan-clear"),
            who,
        )
        .json(json!({ "reason": reason }))
    };
    let reason = "The new signature matches the engine's updater, not malware.";
    // Nothing flagged yet.
    assert_eq!(
        app.send(keep("windows", &reviewer, reason)).await.status(),
        StatusCode::NOT_FOUND
    );
    sqlx::query(
        "update release_files set rescan_flagged_at = now(), rescan_result = 'on_hold' where release_id = $1::uuid",
    )
    .bind(&id)
    .execute(&app.db)
    .await
    .unwrap();
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(queue["rescans"][0]["release_id"], id.as_str());
    let read = token(&app, &owner, "read").await;
    for who in [&support, &read] {
        assert_eq!(
            app.send(keep("windows", who, reason)).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    for bad in ["short", &"x".repeat(1001)] {
        let res = app.send(keep("windows", &reviewer, bad)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "reason");
    }
    assert_eq!(
        app.send(keep("linux", &reviewer, reason)).await.status(),
        StatusCode::NOT_FOUND
    );
    let review = token(&app, &owner, "review").await;
    assert_eq!(
        app.send(keep("windows", &review, reason)).await.status(),
        StatusCode::NO_CONTENT
    );
    let (flagged, why): (bool, Option<String>) = sqlx::query_as(
        "select rescan_flagged_at is not null, rescan_cleared_reason from release_files where release_id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((flagged, why.as_deref()), (false, Some(reason)));
    assert_eq!(release_state(&app, &id).await.0, "approved");
    let (subject, detail): (String, Value) = sqlx::query_as(
        "select subject, detail from audit_log where action = 'release.rescan_cleared'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(subject, format!("{id} windows"));
    assert_eq!(detail["reason"], reason);
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(queue["rescans"], json!([]));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn listing_an_app_checks_it_is_ready(db: PgPool) {
    let host = fake_host().await;
    let base = host.clone();
    let app = test_app_with(db, move |c| {
        c.releases.github_api_base = base.parse().unwrap()
    })
    .await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let list =
        |slug: &str, who: &str| as_who(Method::POST, format!("/v1/admin/apps/{slug}/list"), who);
    let refusal = |res: Response| async move {
        assert_eq!(res.status(), StatusCode::CONFLICT);
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let draft = |slug: &str, set: &str| {
        let set = if set.is_empty() {
            "status = 'draft', stage = 'coming_soon'".to_string()
        } else {
            format!("status = 'draft', stage = 'coming_soon', {set}")
        };
        let app = &app;
        let slug = slug.to_string();
        async move { listed_with(app, maker, &slug, "coming_soon", &set).await }
    };

    draft("soon", "").await;
    assert_eq!(
        app.send(list("soon", &support)).await.status(),
        StatusCode::FORBIDDEN
    );
    // Frozen, or with pictures waiting: not yet.
    draft(
        "frozen",
        "frozen_at = now(), frozen_reason = 'Repository gone'",
    )
    .await;
    let why = refusal(app.send(list("frozen", &owner)).await).await;
    assert!(why.contains("frozen (Repository gone)"), "{why}");
    draft("pictures", "").await;
    sqlx::query(
        "insert into media_sets (id, app_id, commit_sha, fingerprint, source)
         select $1, id, repeat('a', 40), 'f', 'maker' from apps where slug = 'pictures'",
    )
    .bind(Uuid::now_v7())
    .execute(&app.db)
    .await
    .unwrap();
    let why = refusal(app.send(list("pictures", &owner)).await).await;
    assert!(why.contains("review the app's pictures first"), "{why}");
    // Private Slop goes in the store only when its owner asked.
    draft("private", "visibility = 'private'").await;
    let why = refusal(app.send(list("private", &owner)).await).await;
    assert!(why.contains("Private Slop"), "{why}");
    // Asked, but the repository is still private, or GitHub can't say.
    draft(
        "closed",
        "visibility = 'private', public_requested_at = now(), repo_url = 'https://github.com/x/closed'",
    )
    .await;
    let why = refusal(app.send(list("closed", &owner)).await).await;
    assert!(why.contains("it can't go public yet"), "{why}");
    draft(
        "flaky",
        "visibility = 'private', public_requested_at = now(), repo_url = 'https://github.com/x/broken'",
    )
    .await;
    let why = refusal(app.send(list("flaky", &owner)).await).await;
    assert!(
        why.contains("couldn't confirm the repository is public"),
        "{why}"
    );
    let still_private: i64 = sqlx::query_scalar(
        "select count(*) from apps where slug in ('closed', 'flaky') and visibility = 'private' and status = 'draft'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(still_private, 2);

    // Ready: listed, once.
    let review = token(&app, &owner, "review").await;
    assert_eq!(
        app.send(list("soon", &review)).await.status(),
        StatusCode::NO_CONTENT
    );
    let why = refusal(app.send(list("soon", &owner)).await).await;
    assert_eq!(why, "already listed");
    let why = refusal(app.send(list("nothing", &owner)).await).await;
    assert_eq!(why, "no such app");
    assert!(audit_actions(&app).await.contains(&"app.list".to_string()));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn delisting_needs_a_rule_and_the_facts_and_waits_for_a_second_person(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, moderator) = staff(&app, "Mo Moderator", "moderator").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    listed_with(&app, maker_id, "draft", "released", "status = 'draft'").await;
    let delist = |slug: &str, who: &str, body: Value| {
        as_who(Method::POST, format!("/v1/admin/apps/{slug}/delist"), who).json(body)
    };
    let good =
        json!({ "rule": "misleading", "reason": "The page promises multiplayer; there is none." });

    assert_eq!(
        app.send(delist("castle", &reviewer, good.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let review = token(&app, &owner, "review").await;
    assert_eq!(
        app.send(delist("castle", &review, good.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let res = app
        .send(delist(
            "castle",
            &owner,
            json!({ "rule": "boring", "reason": "Not fun." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "rule");
    let res = app
        .send(delist(
            "castle",
            &owner,
            json!({ "rule": "misleading", "reason": " " }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "reason");
    assert_eq!(
        app.send(delist("draft", &owner, good.clone()))
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // A moderate token's request waits for a second person like anyone's.
    let agent = token(&app, &owner, "moderate").await;
    let res = app.send(delist("castle", &agent, good.clone())).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let body = body_json(res).await;
    assert_eq!(body["approvals_still_needed"], 1);
    let pending = body["pending_action"].as_str().unwrap().to_string();
    let status = || async {
        sqlx::query_scalar::<_, String>("select status::text from apps where slug = 'castle'")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    assert_eq!(status().await, "listed");
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{pending}/approve")).cookie(&moderator))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(status().await, "delisted");
    // The maker gets a statement of reasons they can appeal.
    let decisions = body_json(
        app.send(req(Method::GET, "/v1/me/decisions").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(decisions[0]["action"], "delist_app");
    assert_eq!(decisions[0]["rule_title"], "Misleading pages");
    assert_eq!(
        decisions[0]["facts"],
        "The page promises multiplayer; there is none."
    );
    assert_eq!(decisions[0]["can_appeal"], true);
    // Out of the store already: nothing to ask for.
    assert_eq!(
        app.send(delist("castle", &owner, good)).await.status(),
        StatusCode::CONFLICT
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stages_move_only_with_an_approved_release(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    let stage = |slug: &str, who: &str, body: Value| {
        as_who(Method::POST, format!("/v1/admin/apps/{slug}/stage"), who).json(body)
    };
    assert_eq!(
        app.send(stage("soon", &support, json!({ "stage": "beta" })))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(stage("soon", &owner, json!({ "stage": "finished" })))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for (slug, body, why) in [
        (
            "soon",
            json!({ "stage": "coming_soon" }),
            "already in that stage",
        ),
        (
            "soon",
            json!({ "stage": "beta" }),
            "approve a release first",
        ),
        (
            "soon",
            json!({ "stage": "released" }),
            "approve a release first",
        ),
        ("nothing", json!({ "stage": "beta" }), "no such app"),
    ] {
        let res = app.send(stage(slug, &owner, body.clone())).await;
        assert_eq!(res.status(), StatusCode::CONFLICT, "{body}");
        assert!(
            body_json(res).await["error"]["message"]
                .as_str()
                .unwrap()
                .contains(why),
            "{body}"
        );
    }
    approved_release(&app, "soon", maker_id).await;
    let review = token(&app, &owner, "review").await;
    assert_eq!(
        app.send(stage("soon", &review, json!({ "stage": "beta" })))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let detail: Value =
        sqlx::query_scalar("select detail from audit_log where action = 'app.stage'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(detail["from"], "coming_soon");
    assert_eq!(detail["to"], "beta");
    // Back to Coming soon needs no release.
    assert_eq!(
        app.send(stage("soon", &owner, json!({ "stage": "coming_soon" })))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_changes_are_approved_or_rejected_by_reviewers(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker_id, _, first) = release_waiting(&app).await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        first.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let change = |kind: &str| {
        let db = app.db.clone();
        let kind = kind.to_string();
        let first = first.clone();
        async move {
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into app_changes (id, app_id, kind, release_id, repo_url, note, requested_by)
                 select $1, id, $2, case when $2 = 'release' then $3::uuid end,
                        case when $2 = 'repo' then 'https://github.com/octo/game-v2' end, 'Please.', $4
                 from apps where slug = 'game'",
            )
            .bind(id)
            .bind(kind)
            .bind(first)
            .bind(maker_id)
            .execute(&db)
            .await
            .unwrap();
            id
        }
    };
    let hold = change("release").await;
    let decide = |id: Uuid, what: &str, who: &str| {
        as_who(Method::POST, format!("/v1/admin/changes/{id}/{what}"), who)
    };
    let read = token(&app, &owner, "read").await;
    for who in [&support, &read] {
        assert_eq!(
            app.send(decide(hold, "approve", who)).await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            app.send(decide(hold, "reject", who).json(json!({ "reason": "No." })))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    // Approving holds players at the release, and wakes the release watcher.
    let woken = || {
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            app.state.watch_wakeup.notified(),
        )
    };
    assert!(woken().await.is_err(), "nothing woke it yet");
    assert_eq!(
        app.send(decide(hold, "approve", &reviewer)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(woken().await.is_ok());
    let pinned: Option<Uuid> = sqlx::query_scalar("select pinned_release_id from apps")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(pinned.map(|p| p.to_string()), Some(first.clone()));
    let res = app.send(decide(hold, "approve", &reviewer)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "not pending any more");

    // Rejecting needs a reason, which the maker sees; only pending changes.
    let repo = change("repo").await;
    let res = app
        .send(decide(repo, "reject", &reviewer).json(json!({ "reason": " " })))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let review = token(&app, &owner, "review").await;
    let res = app
        .send(
            decide(repo, "reject", &review)
                .json(json!({ "reason": "The new repository doesn't have the game's history." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (status, why): (String, String) =
        sqlx::query_as("select status, status_reason from app_changes where id = $1")
            .bind(repo)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "rejected");
    assert_eq!(why, "The new repository doesn't have the game's history.");
    let repo_url: String = sqlx::query_scalar("select repo_url from apps")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(repo_url.ends_with("octo/game"), "unchanged");
    assert_eq!(
        app.send(decide(repo, "reject", &reviewer).json(json!({ "reason": "Again." })))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(decide(repo, "approve", &reviewer)).await.status(),
        StatusCode::CONFLICT
    );
    let actions = audit_actions(&app).await;
    assert!(actions.contains(&"app.release_change".to_string()));
    assert!(actions.contains(&"app.change_reject".to_string()));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reviewers_turn_down_pictures_and_portfolios_with_a_reason(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (creative, creative_cookie) = app.user("Cass Creative", "US").await;
    let (_, visitor) = app.user("Vic Visitor", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    let pictures = Uuid::now_v7();
    sqlx::query(
        "insert into media_sets (id, app_id, commit_sha, fingerprint, source)
         select $1, id, repeat('a', 40), 'f', 'maker' from apps where slug = 'castle'",
    )
    .bind(pictures)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into creatives (user_id, handle, name, ai_use, trader) values ($1, 'cass', 'Cass', 'none', false)",
    )
    .bind(creative)
    .execute(&app.db)
    .await
    .unwrap();
    let portfolio = |id: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query(
                "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, source)
                 values ($1, $2, 'https://github.com/octo/game', repeat('a', 40), '[]', 'f', 'creative')",
            )
            .bind(id)
            .bind(creative)
            .execute(&db)
            .await
            .unwrap();
        }
    };
    let first = Uuid::now_v7();
    portfolio(first).await;
    let post = |path: String, who: &str| as_who(Method::POST, format!("/v1/admin/{path}"), who);
    let read = token(&app, &owner, "read").await;
    for who in [&support, &read] {
        for path in [
            format!("media/{pictures}/reject"),
            format!("portfolio/{first}/reject"),
        ] {
            assert_eq!(
                app.send(post(path, who).json(json!({ "reason": "No." })))
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            app.send(post(format!("portfolio/{first}/approve"), who))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    // A reason, for the person.
    for path in [
        format!("media/{pictures}/reject"),
        format!("portfolio/{first}/reject"),
    ] {
        assert_eq!(
            app.send(post(path, &reviewer).json(json!({ "reason": "  " })))
                .await
                .status(),
            StatusCode::CONFLICT
        );
    }
    let res = app
        .send(
            post(format!("media/{pictures}/reject"), &reviewer)
                .json(json!({ "reason": "The thumbnail is another game's logo." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (status, why): (String, String) =
        sqlx::query_as("select status, status_reason from media_sets where id = $1")
            .bind(pictures)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), why.as_str()),
        ("rejected", "The thumbnail is another game's logo.")
    );
    let res = app
        .send(
            post(format!("portfolio/{first}/reject"), &reviewer)
                .json(json!({ "reason": "The works are someone else's." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    // The creative sees why; the page stays hidden from everyone else.
    let overview = body_json(
        app.send(req(Method::GET, "/v1/me/creative/portfolio").cookie(&creative_cookie))
            .await,
    )
    .await;
    assert_eq!(overview["live"], Value::Null);
    assert_eq!(
        overview["rejected"]["status_reason"],
        "The works are someone else's."
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/people/cass").cookie(&visitor))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Decided ones can't be decided again.
    for path in [
        format!("media/{pictures}/reject"),
        format!("portfolio/{first}/reject"),
    ] {
        assert_eq!(
            app.send(post(path, &reviewer).json(json!({ "reason": "Again." })))
                .await
                .status(),
            StatusCode::CONFLICT
        );
    }
    for path in [
        format!("media/{pictures}/approve"),
        format!("portfolio/{first}/approve"),
    ] {
        assert_eq!(
            app.send(post(path, &reviewer)).await.status(),
            StatusCode::CONFLICT
        );
    }

    // A new portfolio, approved: the page goes live.
    let second = Uuid::now_v7();
    portfolio(second).await;
    let review = token(&app, &owner, "review").await;
    assert_eq!(
        app.send(post(format!("portfolio/{second}/approve"), &review))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/people/cass").cookie(&visitor))
            .await
            .status(),
        StatusCode::OK
    );
    let actions = audit_actions(&app).await;
    for action in ["media.reject", "portfolio.reject", "portfolio.approve"] {
        assert!(actions.contains(&action.to_string()), "{action}");
    }
}
