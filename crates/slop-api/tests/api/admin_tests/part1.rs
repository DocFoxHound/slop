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
async fn appeal_and_reverse(app: &TestApp, decision: &str, user: &str, admin: &str) -> Value {
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
    assert_eq!(res.status(), StatusCode::OK, "{decision}");
    body_json(res).await
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

/// A bounty hidden by mistake comes back as it was: open for the time it had left (at least a
/// week), not expired the moment it's restored. Its pledges stay released, since their cards were
/// let go, and the backers are told they can pledge again.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_bounty_restored_on_appeal_is_still_open_for_its_backers(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    let (backer, _) = app.user("Bea Backer", "US").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    let bounty = bounty_on(&app, "castle", pat_id, backer).await;
    let hide_and_restore = |bounty: Uuid| {
        let (app, admin, rita, pat) = (&app, &admin, &rita, &pat);
        async move {
            send_report(
                app,
                Some(rita),
                report_of("bounty", &bounty.to_string(), "scam"),
            )
            .await;
            let res = act_on_newest(app, admin, decision_body("hide_bounty", "scam")).await;
            let decision = body_json(res).await["decision"]
                .as_str()
                .unwrap()
                .to_string();
            appeal_and_reverse(app, &decision, pat, admin).await;
        }
    };
    hide_and_restore(bounty).await;
    slop_api::bounties::charge_due(&app.state).await.unwrap();
    let (status, left, pledge): (String, bool, String) = sqlx::query_as(
        "select b.status, b.expires_at > now() + interval '179 days', p.status
         from bounties b join bounty_pledges p on p.bounty_id = b.id where b.id = $1",
    )
    .bind(bounty)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(status, "open", "still open after the next expiry pass");
    assert!(left, "it runs for the time it had left");
    assert_eq!(pledge, "released", "its card was let go");
    let told: Vec<String> = sqlx::query_scalar(
        "select title from notifications where user_id = $1 and title like '%is back'",
    )
    .bind(backer)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(told, ["\"Fix the save bug\" is back"]);

    // One with two days left gets a week.
    let short = bounty_on(&app, "castle", pat_id, backer).await;
    sqlx::query("update bounties set expires_at = now() + interval '2 days' where id = $1")
        .bind(short)
        .execute(&app.db)
        .await
        .unwrap();
    hide_and_restore(short).await;
    let week: bool = sqlx::query_scalar(
        "select status = 'open' and expires_at between now() + interval '6 days 23 hours' and now() + interval '7 days 1 hour'
         from bounties where id = $1",
    )
    .bind(short)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(week, "at least a week");
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
        StatusCode::OK
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
            "The Windows build asks for administrator rights.".to_string()
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
    // The creative was told each time, under the bell (and by email if they want it).
    let told: Vec<(String, String, String, String)> = sqlx::query_as(
        "select category, title, body, link from notifications where user_id = $1 order by created_at",
    )
    .bind(creative)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        told,
        [
            (
                "moderation".to_string(),
                "Your portfolio wasn't approved".to_string(),
                "The works are someone else's.".to_string(),
                "/people/me".to_string()
            ),
            (
                "moderation".to_string(),
                "Your People page is live".to_string(),
                "A reviewer approved your portfolio, so anyone can find your People page now."
                    .to_string(),
                "/people/cass".to_string()
            ),
        ]
    );
    // A later portfolio approved: told it's on the page.
    let third = Uuid::now_v7();
    portfolio(third).await;
    assert_eq!(
        app.send(post(format!("portfolio/{third}/approve"), &reviewer))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let last: String = sqlx::query_scalar(
        "select title from notifications where user_id = $1 order by created_at desc limit 1",
    )
    .bind(creative)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(last, "Your new portfolio is on your People page");
    let actions = audit_actions(&app).await;
    for action in ["media.reject", "portfolio.reject", "portfolio.approve"] {
        assert!(actions.contains(&action.to_string()), "{action}");
    }
}

// ---------------------------------------------------------------------------------------------
// Staff, roles and two-person approval

/// Asks (as `who`) to take `slug` out of the store; returns the pending action's ID.
async fn ask_to_delist(app: &TestApp, slug: &str, who: &str) -> String {
    let res = app
        .send(
            as_who(Method::POST, format!("/v1/admin/apps/{slug}/delist"), who)
                .json(json!({ "rule": "misleading", "reason": "Fake screenshots." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    body_json(res).await["pending_action"]
        .as_str()
        .unwrap()
        .to_string()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn pending_actions_are_listed_and_lapse_after_three_days(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, second) = make_admin(&app, "Sid Second").await;
    let (looker_id, looker) = app.user("Lou Looker", "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(looker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "old", "released", "").await;
    listed_with(&app, maker_id, "new", "released", "").await;
    let stale = ask_to_delist(&app, "old", &owner).await;
    let fresh = ask_to_delist(&app, "new", &owner).await;
    // The other people who could approve were told.
    let told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where title = 'A second approval is needed'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 2, "Sid, once for each");

    // Any staff member or token can look; a player can't.
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/actions").cookie(&maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let read = token(&app, &owner, "read").await;
    let listed = body_json(
        app.send(as_who(Method::GET, "/v1/admin/actions".into(), &read))
            .await,
    )
    .await;
    assert_eq!(listed["approvals_required"], 2);
    let ids: Vec<&str> = listed["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [fresh.as_str(), stale.as_str()], "newest first");
    assert_eq!(listed["actions"][0]["status"], "pending");
    assert_eq!(listed["actions"][0]["kind"], "app.delist");
    assert_eq!(listed["actions"][0]["approvals_needed"], 2);
    assert!(
        listed["actions"][0]["approvals"][0]
            .as_str()
            .unwrap()
            .starts_with("Olive Owner (")
    );

    // Three days on, a request lapses: it leaves the pending list and can't be approved.
    sqlx::query("update staff_actions set requested_at = now() - interval '3 days 1 minute' where id = $1::uuid")
        .bind(&stale)
        .execute(&app.db)
        .await
        .unwrap();
    let listed = body_json(
        app.send(req(Method::GET, "/v1/admin/actions").cookie(&looker))
            .await,
    )
    .await;
    assert_eq!(listed["actions"].as_array().unwrap().len(), 1);
    assert_eq!(listed["actions"][0]["id"], fresh.as_str());
    let all = body_json(
        app.send(req(Method::GET, "/v1/admin/actions?status=all").cookie(&looker))
            .await,
    )
    .await;
    let statuses: Vec<(&str, &str)> = all["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["id"].as_str().unwrap(), a["status"].as_str().unwrap()))
        .collect();
    assert_eq!(
        statuses,
        [(fresh.as_str(), "pending"), (stale.as_str(), "expired")]
    );
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{stale}/approve")).cookie(&second))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(res).await["error"]["message"],
        "that request is expired"
    );
    let status: String = sqlx::query_scalar("select status::text from apps where slug = 'old'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "listed");
    // A staff member without a role can look but not approve.
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/admin/actions/{fresh}/approve")).cookie(&looker))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(
            req(
                Method::POST,
                format!("/v1/admin/actions/{}/approve", Uuid::now_v7())
            )
            .cookie(&second)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_request_is_withdrawn_by_its_requester_or_turned_down_by_an_approver(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, moderator) = staff(&app, "Mo Moderator", "moderator").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    let cancel =
        |id: &str, who: &str| as_who(Method::POST, format!("/v1/admin/actions/{id}/cancel"), who);

    // The requester withdraws their own.
    let mine = ask_to_delist(&app, "castle", &moderator).await;
    assert_eq!(
        app.send(cancel(&mine, &moderator)).await.status(),
        StatusCode::NO_CONTENT
    );
    let (status, result): (String, String) =
        sqlx::query_as("select status, result from staff_actions where id = $1::uuid")
            .bind(&mine)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "cancelled");
    assert!(
        result.starts_with("cancelled by Mo Moderator ("),
        "{result}"
    );
    // Gone: not again, and it can't be approved.
    assert_eq!(
        app.send(cancel(&mine, &moderator)).await.status(),
        StatusCode::NOT_FOUND
    );
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{mine}/approve")).cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(res).await["error"]["message"],
        "that request is cancelled"
    );

    // Someone else's: only people who could approve it may turn it down.
    let theirs = ask_to_delist(&app, "tower", &moderator).await;
    assert_eq!(
        app.send(cancel(&theirs, &reviewer)).await.status(),
        StatusCode::FORBIDDEN
    );
    let agent = token(&app, &owner, "review").await;
    assert_eq!(
        app.send(cancel(&theirs, &agent)).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(cancel(&theirs, &owner)).await.status(),
        StatusCode::NO_CONTENT
    );
    let listed: i64 = sqlx::query_scalar("select count(*) from apps where status = 'listed'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(listed, 2, "nothing was delisted");
    let cancels = audit_actions(&app)
        .await
        .into_iter()
        .filter(|a| a == "staff_action.cancel")
        .count();
    assert_eq!(cancels, 2);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_staff_list_shows_roles_and_suspensions_but_not_closed_accounts(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 3).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (sue_id, _) = staff(&app, "Sue Support", "support").await;
    sqlx::query(
        "insert into staff_roles (user_id, role, granted_by) values ($1, 'finance', 'test')",
    )
    .bind(sue_id)
    .execute(&app.db)
    .await
    .unwrap();
    let (gone_id, _) = staff(&app, "Gus Gone", "reviewer").await;
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(gone_id)
        .execute(&app.db)
        .await
        .unwrap();
    let (sid_id, _) = staff(&app, "Sid Suspended", "moderator").await;
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(sid_id)
        .execute(&app.db)
        .await
        .unwrap();
    let read = token(&app, &owner, "read").await;
    let list = body_json(
        app.send(as_who(Method::GET, "/v1/admin/staff".into(), &read))
            .await,
    )
    .await;
    let staff: Vec<(String, Value, bool)> = list["staff"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["display_name"].as_str().unwrap().to_string(),
                s["roles"].clone(),
                s["suspended"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        staff,
        [
            ("Olive Owner".to_string(), json!(["owner"]), false),
            ("Sid Suspended".to_string(), json!(["moderator"]), true),
            (
                "Sue Support".to_string(),
                json!(["finance", "support"]),
                false
            ),
        ]
    );
    assert_eq!(
        list["roles"],
        json!([
            "owner",
            "moderator",
            "reviewer",
            "support",
            "finance",
            "security"
        ])
    );
    assert_eq!(list["my_roles"], json!(["owner"]));
    assert_eq!(list["approvals_required"], 3);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_roles_are_given_and_taken_by_security_and_owners(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = make_admin(&app, "Olive Owner").await;
    let (_, security) = staff(&app, "Sam Security", "security").await;
    let (_, moderator) = staff(&app, "Mo Moderator", "moderator").await;
    let (pat_id, _) = app.user("Pat Player", "US").await;
    let (closed_id, _) = app.user("Cleo Closed", "US").await;
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(closed_id)
        .execute(&app.db)
        .await
        .unwrap();
    let role = |user: Uuid, who: &str, body: Value| {
        as_who(Method::POST, format!("/v1/admin/staff/{user}/roles"), who).json(body)
    };
    let roles_of = |user: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (bool, Vec<String>)>(
                "select is_admin, coalesce(array(select role from staff_roles r where r.user_id = u.id order by role), '{}')
                 from users u where id = $1",
            )
            .bind(user)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    let reviewer_role = json!({ "role": "reviewer", "grant": true });

    // Only staff who manage staff; tokens that only review or read can't even ask.
    assert_eq!(
        app.send(role(pat_id, &moderator, reviewer_role.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    for scope in ["read", "review"] {
        let t = token(&app, &owner, scope).await;
        assert_eq!(
            app.send(role(pat_id, &t, reviewer_role.clone()))
                .await
                .status(),
            StatusCode::FORBIDDEN,
            "{scope}"
        );
    }
    // The owner role, only by an owner.
    assert_eq!(
        app.send(role(
            pat_id,
            &security,
            json!({ "role": "owner", "grant": true })
        ))
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    // Someone who isn't there, or whose account is closed.
    assert_eq!(
        app.send(role(Uuid::now_v7(), &security, reviewer_role.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(role(closed_id, &security, reviewer_role.clone()))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(roles_of(closed_id).await, (false, vec![]));
    // A session from a while ago needs no "confirm it's you" (SECURITY §4.3, 2026-10-25).
    sqlx::query("update sessions set created_at = now() - interval '1 hour'")
        .execute(&app.db)
        .await
        .unwrap();

    // Security gives a role: Pat is staff with it.
    assert_eq!(
        app.send(role(pat_id, &security, reviewer_role.clone()))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(roles_of(pat_id).await, (true, vec!["reviewer".into()]));
    // Taking the last role leaves someone who can only look.
    assert_eq!(
        app.send(role(
            pat_id,
            &security,
            json!({ "role": "reviewer", "grant": false })
        ))
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(roles_of(pat_id).await, (true, vec![]));
    // The last owner stays an owner, even by their own hand.
    let res = app
        .send(role(
            owner_id,
            &owner,
            json!({ "role": "owner", "grant": false }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(res).await["error"]["message"],
        "Slop needs at least one owner"
    );
    let failed: String =
        sqlx::query_scalar("select status from staff_actions order by requested_at desc limit 1")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(failed, "failed", "recorded as tried and refused");
    // With a second owner, the first can step down.
    assert_eq!(
        app.send(role(
            pat_id,
            &owner,
            json!({ "role": "owner", "grant": true })
        ))
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(role(
            owner_id,
            &owner,
            json!({ "role": "owner", "grant": false })
        ))
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(roles_of(owner_id).await, (true, vec![]));
    let actions = audit_actions(&app).await;
    assert!(actions.contains(&"staff.grant_role".to_string()));
    assert!(actions.contains(&"staff.remove_role".to_string()));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn removing_someone_from_staff_ends_their_roles_and_tokens(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (second_id, second) = make_admin(&app, "Sid Second").await;
    let (_, security) = staff(&app, "Sam Security", "security").await;
    let (rex_id, rex) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (pat_id, _) = app.user("Pat Player", "US").await;
    // Rex had a token (made while he could; tokens need the security permission).
    sqlx::query(
        "insert into staff_roles (user_id, role, granted_by) values ($1, 'security', 'test')",
    )
    .bind(rex_id)
    .execute(&app.db)
    .await
    .unwrap();
    let rex_token = token(&app, &rex, "review").await;
    sqlx::query("delete from staff_roles where user_id = $1 and role = 'security'")
        .bind(rex_id)
        .execute(&app.db)
        .await
        .unwrap();
    let remove = |user: Uuid, who: &str, reason: &str| {
        as_who(Method::POST, format!("/v1/admin/staff/{user}/remove"), who)
            .json(json!({ "reason": reason }))
    };
    let reason = "Left the team at the end of the month.";

    for bad in ["Too short", &"x".repeat(2001)] {
        let res = app.send(remove(rex_id, &security, bad)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "reason");
    }
    assert_eq!(
        app.send(remove(pat_id, &security, reason)).await.status(),
        StatusCode::CONFLICT,
        "not staff"
    );
    assert_eq!(
        app.send(remove(Uuid::now_v7(), &security, reason))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(remove(second_id, &security, reason))
            .await
            .status(),
        StatusCode::FORBIDDEN,
        "only an owner removes an owner"
    );
    assert_eq!(
        app.send(remove(rex_id, &rex, reason)).await.status(),
        StatusCode::FORBIDDEN
    );

    // An agent can only ask: it waits for a person, even with one approval enough.
    let agent = token(&app, &owner, "moderate").await;
    let res = app.send(remove(rex_id, &agent, reason)).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let pending = body_json(res).await;
    assert_eq!(pending["approvals_still_needed"], 1);
    let action = pending["pending_action"].as_str().unwrap().to_string();
    let still_staff = |id: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, bool>("select is_admin from users where id = $1")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert!(still_staff(rex_id).await);
    assert_eq!(
        app.send(as_who(Method::GET, "/v1/admin/queue".into(), &rex_token))
            .await
            .status(),
        StatusCode::OK,
        "his token still works while it waits"
    );
    let asked: Vec<String> = sqlx::query_scalar(
        "select u.display_name from notifications n join users u on u.id = n.user_id
         where n.title = 'An AI agent asked for a staff change' order by u.display_name",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(asked, ["Olive Owner", "Sam Security", "Sid Second"]);
    // A person approves: every role and token goes.
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/admin/actions/{action}/approve")).cookie(&second))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert!(!still_staff(rex_id).await);
    let (roles, live_tokens): (i64, i64) = sqlx::query_as(
        "select (select count(*) from staff_roles where user_id = $1),
                (select count(*) from admin_tokens where user_id = $1 and revoked_at is null)",
    )
    .bind(rex_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((roles, live_tokens), (0, 0));
    assert_eq!(
        app.send(as_who(Method::GET, "/v1/admin/queue".into(), &rex_token))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let detail: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'admin.revoke' and subject = $1",
    )
    .bind(rex_id.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(detail, json!({ "reason": reason, "tokens_revoked": 1 }));

    // An owner may remove another owner.
    assert_eq!(
        app.send(remove(second_id, &owner, reason)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(!still_staff(second_id).await);
}

/// Two people approving the same request at the same moment do it once.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn two_approvers_at_once_carry_out_an_action_once(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, ben) = make_admin(&app, "Ben Owner").await;
    let (_, cat) = make_admin(&app, "Cat Owner").await;
    let res = app
        .send(
            req(Method::POST, "/v1/admin/legal/privacy")
                .cookie(&owner)
                .json(json!({ "summary": "Adds a section on chat.", "body_md": "New words about chat." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let action = body_json(res).await["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    let approve =
        |who: &str| req(Method::POST, format!("/v1/admin/actions/{action}/approve")).cookie(who);
    // Both reach the request while something else holds it, so they read it one right after
    // the other.
    let mut hold = app.db.begin().await.unwrap();
    sqlx::query("select 1 from staff_actions where id = $1::uuid for update")
        .bind(&action)
        .execute(&mut *hold)
        .await
        .unwrap();
    let release = async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        hold.commit().await.unwrap();
    };
    let (a, b, ()) = tokio::join!(app.send(approve(&ben)), app.send(approve(&cat)), release);
    let mut statuses = [a.status(), b.status()];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
    let versions: i64 =
        sqlx::query_scalar("select count(*) from legal_documents where kind = 'privacy'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(versions, 2, "the repository's version and one new one");
    let (status, approvals): (String, i64) = sqlx::query_as(
        "select status, (select count(*) from staff_action_approvals p where p.action_id = a.id)
         from staff_actions a where id = $1::uuid",
    )
    .bind(&action)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((status.as_str(), approvals), ("done", 2));
}

// ---------------------------------------------------------------------------------------------
// Legal documents

/// Adds a version of a legal document, published `published` ago and in effect from
/// `effective` (both SQL intervals, e.g. '-1 day' or '15 days'), from the repository (no
/// publisher) unless `by` is given.
async fn legal_version(
    app: &TestApp,
    kind: &str,
    version: &str,
    published: &str,
    effective: &str,
    by: Option<&str>,
) {
    sqlx::query(
        "insert into legal_documents (kind, version, title, summary, body_md, sha256, published_at, effective_at, published_by)
         values ($1, $2, 'Doc', 'Changes ' || $2, 'Words of ' || $2, md5($2), now() + $3::interval, now() + $4::interval, $5)",
    )
    .bind(kind)
    .bind(version)
    .bind(published)
    .bind(effective)
    .bind(by)
    .execute(&app.db)
    .await
    .unwrap();
}

fn entry<'a>(list: &'a Value, kind: &str) -> &'a Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == kind)
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_legal_index_splits_what_is_in_effect_from_what_is_announced(db: PgPool) {
    let app = test_app(db).await;
    legal_version(&app, "privacy", "2099-01-01", "0 days", "15 days", None).await;
    legal_version(
        &app,
        "privacy",
        "2026-01-01",
        "-300 days",
        "-280 days",
        None,
    )
    .await;
    let index = body_json(app.send(req(Method::GET, "/v1/legal")).await).await;
    let kinds: Vec<(&str, &str, bool)> = index
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["kind"].as_str().unwrap(),
                d["title"].as_str().unwrap(),
                d["accepted"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("terms", "Terms of Service", true),
            ("privacy", "Privacy Policy", false),
            ("content", "Content Policy", false),
            ("publisher", "Publisher Agreement", true),
            ("people", "People Terms", true),
        ]
    );
    for d in index.as_array().unwrap() {
        assert!(d["current"]["version"].is_string(), "{d}");
    }
    let privacy = entry(&index, "privacy");
    assert_eq!(privacy["current"]["version"], "2026-10-20");
    assert_eq!(privacy["upcoming"]["version"], "2099-01-01");
    assert_eq!(privacy["upcoming"]["summary"], "Changes 2099-01-01");
    assert!(entry(&index, "terms")["upcoming"].is_null());

    // One document: in effect now by default, or a named version from the past or the future.
    let doc = |q: &str| app.send(req(Method::GET, format!("/v1/legal/{q}")));
    let now = body_json(doc("privacy").await).await;
    assert_eq!(now["version"], "2026-10-20");
    assert_eq!(now["in_effect"], true);
    assert_eq!(now["title"], "Privacy Policy");
    let versions: Vec<&str> = now["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["version"].as_str().unwrap())
        .collect();
    assert_eq!(versions, ["2099-01-01", "2026-10-20", "2026-01-01"]);
    let future = body_json(doc("privacy?version=2099-01-01").await).await;
    assert_eq!(future["in_effect"], false);
    assert_eq!(future["body_md"], "Words of 2099-01-01");
    let past = body_json(doc("privacy?version=2026-01-01").await).await;
    assert_eq!(past["in_effect"], true);
    assert_eq!(past["summary"], "Changes 2026-01-01");
    for q in [
        "weather",
        "privacy?version=2026-02-02",
        "privacy?version=nonsense",
    ] {
        assert_eq!(doc(q).await.status(), StatusCode::NOT_FOUND, "{q}");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn people_are_told_about_agreements_they_are_party_to_and_new_policies(db: PgPool) {
    let app = test_app(db).await;
    let (_, newcomer) = app.user("Nell Newcomer", "US").await;
    let (old_id, old) = app.user("Olga Oldtimer", "US").await;
    sqlx::query("update users set created_at = now() - interval '60 days' where id = $1")
        .bind(old_id)
        .execute(&app.db)
        .await
        .unwrap();
    let (player_id, player) = app.user("Pat Player", "US").await;
    sqlx::query(
        "delete from legal_acceptances where user_id = $1 and kind in ('publisher', 'people')",
    )
    .bind(player_id)
    .execute(&app.db)
    .await
    .unwrap();
    let notices = |cookie: &str| {
        let r = req(Method::GET, "/v1/me/legal").cookie(cookie);
        let app = &app;
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::OK);
            body_json(res).await
        }
    };
    let kinds = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|n| n["kind"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/legal")).await.status(),
        StatusCode::UNAUTHORIZED
    );
    // Someone who joined after everything was published, and accepted it all: nothing to say.
    assert_eq!(notices(&newcomer).await, json!([]));
    // Someone who joined before the policies were published hears about them, as new documents.
    let olga = notices(&old).await;
    assert_eq!(kinds(&olga), ["privacy", "content"]);
    assert_eq!(olga[0]["action"], "read");
    assert_eq!(olga[0]["first"], true);
    assert_eq!(olga[0]["in_effect"], true);
    // A policy change in effect for more than 30 days is old news.
    sqlx::query(
        "update legal_documents set published_at = now() - interval '40 days', effective_at = now() - interval '31 days'
         where kind = 'privacy'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(kinds(&notices(&old).await), ["content"]);

    // A new version of the Terms is announced: everyone is party to them.
    legal_version(&app, "terms", "2099-01-01", "0 days", "15 days", None).await;
    let nell = notices(&newcomer).await;
    assert_eq!(kinds(&nell), ["terms"]);
    assert_eq!(nell[0]["action"], "accept");
    assert_eq!(nell[0]["in_effect"], false);
    assert_eq!(nell[0]["first"], false);
    assert_eq!(nell[0]["version"], "2099-01-01");
    // A new Publisher Agreement: only for people who accepted one before.
    legal_version(&app, "publisher", "2099-01-01", "0 days", "15 days", None).await;
    assert_eq!(kinds(&notices(&newcomer).await), ["terms", "publisher"]);
    assert_eq!(kinds(&notices(&player).await), ["terms"]);
    // Accepting the announced version ends the notice.
    let res = app
        .send(
            req(Method::POST, "/v1/me/legal/accept")
                .cookie(&newcomer)
                .json(json!({ "kind": "terms", "version": "2099-01-01" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(kinds(&notices(&newcomer).await), ["publisher"]);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn actions_wait_for_the_agreement_in_effect(db: PgPool) {
    let app = test_app(db).await;
    let (user_id, cookie) = app.user("Cass Creative", "US").await;
    assert!(
        slop_api::legal::require(&app.db, user_id, "people")
            .await
            .is_ok()
    );
    // A kind with no document at all asks nothing.
    sqlx::query("delete from legal_acceptances where kind = 'people'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("delete from legal_documents where kind = 'people'")
        .execute(&app.db)
        .await
        .unwrap();
    assert!(
        slop_api::legal::require(&app.db, user_id, "people")
            .await
            .is_ok()
    );
    // A version in effect that they haven't accepted: People actions wait.
    legal_version(&app, "people", "2026-10-01", "-20 days", "-1 day", None).await;
    assert!(matches!(
        slop_api::legal::require(&app.db, user_id, "people").await,
        Err(slop_api::error::ApiError::TermsRequired("people"))
    ));
    let mut body = profile("cass", false);
    body["confirm_adult"] = json!(true);
    let res = app
        .send(
            req(Method::PUT, "/v1/me/creative")
                .cookie(&cookie)
                .json(body.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let err = body_json(res).await;
    assert_eq!(err["error"]["code"], "terms_required");
    assert_eq!(err["error"]["field"], "people");
    // An announced one doesn't count until it's in effect.
    legal_version(&app, "people", "2099-01-01", "0 days", "15 days", None).await;
    app.send(
        req(Method::POST, "/v1/me/legal/accept")
            .cookie(&cookie)
            .json(json!({ "kind": "people", "version": "2026-10-01" })),
    )
    .await;
    assert!(
        slop_api::legal::require(&app.db, user_id, "people")
            .await
            .is_ok()
    );
    let res = app
        .send(
            req(Method::PUT, "/v1/me/creative")
                .cookie(&cookie)
                .json(body),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn start_up_publishes_new_versions_and_never_changes_a_published_one(db: PgPool) {
    let app = test_app(db).await;
    let rows = || async {
        sqlx::query_as::<_, (String, String, bool, bool)>(
            "select kind, version, effective_at <= now(), effective_at > now() + interval '14 days'
             from legal_documents order by kind, version",
        )
        .fetch_all(&app.db)
        .await
        .unwrap()
    };
    // Every document's first version is in effect at once.
    let first = rows().await;
    assert_eq!(first.len(), 5);
    assert!(first.iter().all(|r| r.2), "{first:?}");

    // Published words never change under people who accepted them, even if the file is edited.
    sqlx::query(
        "update legal_documents set body_md = 'The words people accepted.', sha256 = 'accepted' where kind = 'terms'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    slop_api::legal::sync(&app.db).await.unwrap();
    let body: String =
        sqlx::query_scalar("select body_md from legal_documents where kind = 'terms'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(body, "The words people accepted.");
    assert_eq!(rows().await, first);

    // A version from the file that's new to a document already published waits 15 days.
    sqlx::query("delete from legal_documents where kind = 'privacy'")
        .execute(&app.db)
        .await
        .unwrap();
    legal_version(
        &app,
        "privacy",
        "2026-01-01",
        "-300 days",
        "-300 days",
        None,
    )
    .await;
    slop_api::legal::sync(&app.db).await.unwrap();
    let privacy: Vec<(String, String, bool, bool)> = rows()
        .await
        .into_iter()
        .filter(|r| r.0 == "privacy")
        .collect();
    assert_eq!(
        privacy,
        [
            ("privacy".into(), "2026-01-01".into(), true, false),
            ("privacy".into(), "2026-10-20".into(), false, true),
        ]
    );
    // Nothing new: nothing changes.
    let before = rows().await;
    slop_api::legal::sync(&app.db).await.unwrap();
    assert_eq!(rows().await, before);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_publish_a_new_legal_version_with_notice(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, player) = app.user("Pat Player", "US").await;
    let publish = |kind: &str, who: &str, body: Value| {
        as_who(Method::POST, format!("/v1/admin/legal/{kind}"), who).json(body)
    };
    let change = json!({ "summary": "Explains chat reports.", "body_md": "New privacy words." });

    // The overview: repository versions have no file text to copy.
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/legal").cookie(&player))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let read = token(&app, &owner, "read").await;
    let docs = body_json(
        app.send(as_who(Method::GET, "/v1/admin/legal".into(), &read))
            .await,
    )
    .await;
    assert_eq!(docs.as_array().unwrap().len(), 5);
    let privacy = entry(&docs, "privacy");
    assert!(privacy["file_text"].is_null());
    assert_eq!(privacy["latest_version"], "2026-10-20");
    assert_eq!(privacy["accepted"], false);
    assert!(privacy["versions"][0]["published_by"].is_null());
    assert!(!privacy["latest_body_md"].as_str().unwrap().is_empty());

    // Who may, and what's refused.
    assert_eq!(
        app.send(publish("privacy", &reviewer, change.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let review = token(&app, &owner, "review").await;
    assert_eq!(
        app.send(publish("privacy", &review, change.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(publish("weather", &owner, change.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    for (body, field) in [
        (
            json!({ "summary": "Two\nlines", "body_md": "x" }),
            "summary",
        ),
        (json!({ "summary": "", "body_md": "x" }), "summary"),
        (
            json!({ "summary": "Same", "body_md": privacy["latest_body_md"] }),
            "body_md",
        ),
    ] {
        let res = app.send(publish("privacy", &owner, body)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        assert_eq!(body_json(res).await["error"]["field"], field);
    }

    // Published (an agent with a moderate token may): dated today, in effect in 15 days.
    let agent = token(&app, &owner, "moderate").await;
    let res = app.send(publish("privacy", &agent, change.clone())).await;
    assert_eq!(res.status(), StatusCode::OK);
    let published = body_json(res).await;
    let today = OffsetDateTime::now_utc().date().to_string();
    assert_eq!(published["version"], today.as_str());
    let in_days = (published["effective_at"].as_i64().unwrap()
        - OffsetDateTime::now_utc().unix_timestamp()) as f64
        / 86_400.0;
    assert!((14.99..15.01).contains(&in_days), "{in_days}");
    let docs = body_json(
        app.send(req(Method::GET, "/v1/admin/legal").cookie(&owner))
            .await,
    )
    .await;
    let privacy = entry(&docs, "privacy");
    let file = privacy["file_text"].as_str().unwrap();
    let parsed = slop_api::legal::parse(file).unwrap();
    assert_eq!(
        (parsed.version, parsed.summary, parsed.body),
        (
            today.as_str(),
            "Explains chat reports.",
            "New privacy words."
        )
    );
    assert!(
        privacy["versions"][0]["published_by"]
            .as_str()
            .unwrap()
            .starts_with("AI reviewer")
    );
    let subject: String =
        sqlx::query_scalar("select subject from audit_log where action = 'legal.published'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(subject, format!("privacy {today}"));

    // A second version the same day, once the first is in effect, gets a letter.
    sqlx::query("update legal_documents set effective_at = now() - interval '1 second' where kind = 'privacy' and version = $1")
        .bind(&today)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(publish(
            "privacy",
            &owner,
            json!({ "summary": "Fixes a typo.", "body_md": "New privacy words, fixed." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["version"], format!("{today}b"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn publishing_a_legal_version_waits_for_a_second_person(db: PgPool) {
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, second) = make_admin(&app, "Sid Second").await;
    let res = app
        .send(
            req(Method::POST, "/v1/admin/legal/content")
                .cookie(&owner)
                .json(json!({ "summary": "Bans loot boxes.", "body_md": "No loot boxes." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let action = body_json(res).await["pending_action"]
        .as_str()
        .unwrap()
        .to_string();
    let versions = || async {
        sqlx::query_scalar::<_, i64>("select count(*) from legal_documents where kind = 'content'")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    assert_eq!(versions().await, 1);
    let res = app
        .send(req(Method::POST, format!("/v1/admin/actions/{action}/approve")).cookie(&second))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(versions().await, 2);
    let by: String = sqlx::query_scalar(
        "select published_by from legal_documents where kind = 'content' and published_by is not null",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(
        by.contains("Olive Owner") && by.contains("approved by Sid Second"),
        "{by}"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_announced_version_can_be_withdrawn_before_it_takes_effect(db: PgPool) {
    // One person is enough to withdraw, even when publishing needs two.
    let app = test_app_with(db, |c| c.approvals_required = 2).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    legal_version(
        &app,
        "terms",
        "2099-01-01",
        "0 days",
        "15 days",
        Some("Olive Owner"),
    )
    .await;
    legal_version(&app, "people", "2099-01-01", "0 days", "15 days", None).await;
    legal_version(
        &app,
        "content",
        "2026-10-21",
        "-5 days",
        "-1 day",
        Some("Olive Owner"),
    )
    .await;
    // Pat already accepted the announced Terms.
    let res = app
        .send(
            req(Method::POST, "/v1/me/legal/accept")
                .cookie(&pat)
                .json(json!({ "kind": "terms", "version": "2099-01-01" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let withdraw = |path: &str, who: &str, reason: &str| {
        as_who(
            Method::POST,
            format!("/v1/admin/legal/{path}/withdraw"),
            who,
        )
        .json(json!({ "reason": reason }))
    };
    let why = "Published by mistake.";

    assert_eq!(
        app.send(withdraw("terms/2099-01-01", &reviewer, why))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    for bad in ["", "  ", &"x".repeat(2001)] {
        let res = app.send(withdraw("terms/2099-01-01", &owner, bad)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "reason");
    }
    for (path, status) in [
        ("terms/2098-01-01", StatusCode::NOT_FOUND),
        ("weather/2099-01-01", StatusCode::NOT_FOUND),
        // In effect already, or from the repository's file: not from here.
        ("content/2026-10-21", StatusCode::CONFLICT),
        ("people/2099-01-01", StatusCode::CONFLICT),
    ] {
        assert_eq!(
            app.send(withdraw(path, &owner, why)).await.status(),
            status,
            "{path}"
        );
    }

    let agent = token(&app, &owner, "moderate").await;
    assert_eq!(
        app.send(withdraw("terms/2099-01-01", &agent, why))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let (left, accepted): (i64, i64) = sqlx::query_as(
        "select (select count(*) from legal_documents where kind = 'terms' and version = '2099-01-01'),
                (select count(*) from legal_acceptances where kind = 'terms' and version = '2099-01-01')",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((left, accepted), (0, 0));
    let still: i64 = sqlx::query_scalar(
        "select count(*) from legal_acceptances where user_id = $1 and kind = 'terms'",
    )
    .bind(pat_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(still, 1, "the acceptance of the version before stays");
    let detail: Value =
        sqlx::query_scalar("select detail from audit_log where action = 'legal.withdrawn'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(detail["reason"], why);
    assert_eq!(detail["acceptances_removed"], 1);
    assert_eq!(detail["body_md"], "Words of 2099-01-01");
    assert_eq!(
        app.send(withdraw("terms/2099-01-01", &owner, why))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

// ---------------------------------------------------------------------------------------------
// Repository access: the daily check, freezing, Check now, making Private Slop public

/// A test app whose GitHub API is `fake_host`, with the host's base URL.
async fn app_with_fake_host(db: PgPool) -> (TestApp, String) {
    let host = fake_host().await;
    let base = host.clone();
    let app = test_app_with(db, move |c| {
        c.releases.github_api_base = base.parse().unwrap();
        c.rate_limits.write_per_minute = 10_000;
    })
    .await;
    (app, host)
}

/// (frozen reason, by staff, status, frozen and delisted for it) of an app.
async fn freeze_state(app: &TestApp, slug: &str) -> (Option<String>, bool, String, bool) {
    sqlx::query_as(
        "select case when frozen_at is not null then coalesce(frozen_reason, '') end, frozen_by is not null,
                status::text, frozen_delisted
         from apps where slug = $1",
    )
    .bind(slug)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_daily_check_freezes_only_on_a_clear_answer(db: PgPool) {
    let (app, host) = app_with_fake_host(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    for (slug, set) in [
        ("gh-fine", "repo_url = 'https://github.com/x/fine'".to_string()),
        ("gh-private-now", "repo_url = 'https://github.com/x/closed'".into()),
        (
            "gh-private-slop",
            "repo_url = 'https://github.com/y/closed', visibility = 'private'".into(),
        ),
        ("gh-down", "repo_url = 'https://github.com/x/broken'".into()),
        (
            "gh-down-frozen",
            "repo_url = 'https://github.com/y/broken', frozen_at = now() - interval '1 day', frozen_reason = 'Gone yesterday'"
                .into(),
        ),
        ("git-200", format!("repo_url = '{host}200/a'")),
        ("git-401", format!("repo_url = '{host}401/a'")),
        ("git-403", format!("repo_url = '{host}403/a'")),
        ("git-404", format!("repo_url = '{host}404/a'")),
        ("git-410", format!("repo_url = '{host}410/a'")),
        ("git-500", format!("repo_url = '{host}500/a'")),
        // Private Slop on another host, with no token saved: Slop can't tell.
        (
            "git-private",
            format!("repo_url = '{host}404/b', visibility = 'private'"),
        ),
        // Private Slop whose saved read-only token stopped working.
        (
            "token-revoked",
            format!(
                "repo_url = '{}ann/secret-game', visibility = 'private'",
                app.fake_base
            ),
        ),
        // A draft nobody has seen isn't checked.
        ("draft", format!("repo_url = '{host}404/c', status = 'draft'")),
    ] {
        listed_with(&app, maker, slug, "released", &set).await;
    }
    let revoked = app_id(&app, "token-revoked").await;
    let mut conn = app.db.acquire().await.unwrap();
    slop_api::repo_tokens::store(
        &mut conn,
        &[9; 32],
        revoked,
        &format!("{}ann/secret-game", app.fake_base),
        &slop_api::repo_tokens::Given {
            kind: slop_api::repo_tokens::Kind::Gitea,
            token: "revoked".into(),
        },
        maker,
    )
    .await
    .unwrap();
    drop(conn);

    assert_eq!(slop_api::frozen::run_once(&app.state).await.unwrap(), 13);
    let mut frozen = Vec::new();
    for slug in [
        "gh-fine",
        "gh-private-now",
        "gh-private-slop",
        "gh-down",
        "gh-down-frozen",
        "git-200",
        "git-401",
        "git-403",
        "git-404",
        "git-410",
        "git-500",
        "git-private",
        "token-revoked",
        "draft",
    ] {
        if let (Some(reason), ..) = freeze_state(&app, slug).await {
            frozen.push((slug, reason));
        }
    }
    let unreadable =
        "the repository can't be read without signing in: it was deleted, made private, or moved";
    assert_eq!(
        frozen,
        [
            (
                "gh-private-now",
                "the repository is private now, and apps in the store need public source".to_string()
            ),
            ("gh-down-frozen", "Gone yesterday".to_string()),
            ("git-401", unreadable.to_string()),
            ("git-403", unreadable.to_string()),
            ("git-404", unreadable.to_string()),
            ("git-410", unreadable.to_string()),
            (
                "token-revoked",
                "Slop's token can't read the repository any more: it was revoked or expired, or the repository moved. Save a new one on the Manage page"
                    .to_string()
            ),
        ],
        "an unclear answer (a host error, or no way to check) changes nothing"
    );
    // Each newly frozen app's maker is told once; the one frozen before isn't told again.
    let told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and title like '% is frozen: %'",
    )
    .bind(maker)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 6);
    // Checked today: nothing is due again until tomorrow.
    assert_eq!(slop_api::frozen::run_once(&app.state).await.unwrap(), 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_daily_check_works_in_batches_unchecked_first(db: PgPool) {
    let (app, host) = app_with_fake_host(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, status, maker_id, repo_url, license_spdx)
         select gen_random_uuid(), 'app-' || i, 'App ' || i, 'An app', 'game', 'released', 'listed', $1,
                $2 || '200/r' || i, 'MIT'
         from generate_series(1, 201) i",
    )
    .bind(maker)
    .bind(&host)
    .execute(&app.db)
    .await
    .unwrap();
    listed_with(
        &app,
        maker,
        "checked-before",
        "released",
        &format!("repo_url = '{host}200/old', repo_checked_at = now() - interval '2 days'"),
    )
    .await;
    let checked_before = || async {
        sqlx::query_scalar::<_, bool>(
            "select repo_checked_at < now() - interval '1 day' from apps where slug = 'checked-before'",
        )
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    assert_eq!(slop_api::frozen::run_once(&app.state).await.unwrap(), 200);
    assert!(checked_before().await, "never-checked apps go first");
    assert_eq!(slop_api::frozen::run_once(&app.state).await.unwrap(), 2);
    assert!(!checked_before().await);
    assert_eq!(slop_api::frozen::run_once(&app.state).await.unwrap(), 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_daily_check_puts_back_an_app_the_grace_period_took_out(db: PgPool) {
    let (app, host) = app_with_fake_host(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(
        &app,
        maker,
        "castle",
        "released",
        &format!("repo_url = '{host}404/castle'"),
    )
    .await;
    slop_api::frozen::run_once(&app.state).await.unwrap();
    sqlx::query("update apps set frozen_at = now() - interval '15 days'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::frozen::end_grace(&app.state).await.unwrap(), 1);
    let state = freeze_state(&app, "castle").await;
    assert_eq!((state.2.as_str(), state.3), ("delisted", true));
    // Access comes back; tomorrow's check finds it.
    sqlx::query("update apps set repo_url = $1, repo_checked_at = now() - interval '25 hours'")
        .bind(format!("{host}200/castle"))
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::frozen::run_once(&app.state).await.unwrap(), 1);
    assert_eq!(
        freeze_state(&app, "castle").await,
        (None, false, "listed".into(), false)
    );
    let told: Vec<(String, String)> = sqlx::query_as(
        "select title, body from notifications where user_id = $1 order by created_at desc limit 1",
    )
    .bind(maker)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(told[0].0, "castle isn't frozen any more");
    assert!(
        told[0]
            .1
            .ends_with("takes new releases and is back in the store.")
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn check_now_answers_with_the_freeze_still_in_place(db: PgPool) {
    let (app, host) = app_with_fake_host(db).await;
    let (maker, cookie) = app.user("Mia Maker", "US").await;
    listed_with(
        &app,
        maker,
        "castle",
        "released",
        &format!("repo_url = '{host}404/castle'"),
    )
    .await;
    let check = |r: Req| r;
    assert_eq!(
        app.send(req(Method::POST, "/v1/apps/castle/repo-check"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    // Still unreadable: the freeze comes back, with its reason and when the grace period ends.
    let res = app
        .send(check(
            req(Method::POST, "/v1/apps/castle/repo-check").cookie(&cookie),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let frozen = body_json(res).await;
    assert!(
        frozen["reason"]
            .as_str()
            .unwrap()
            .contains("can't be read without signing in")
    );
    assert_eq!(frozen["by_staff"], false);
    let since = OffsetDateTime::parse(frozen["since"].as_str().unwrap(), &Rfc3339).unwrap();
    let until = OffsetDateTime::parse(frozen["until"].as_str().unwrap(), &Rfc3339).unwrap();
    assert_eq!(until - since, time::Duration::days(14));

    // A maker's agent with a publishing token can press it too.
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(&cookie)
                .json(json!({ "name": "Editor agent", "scope": "publish" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let agent = format!(
        "Bearer {}",
        body_json(res).await["secret"].as_str().unwrap()
    );
    sqlx::query("update apps set repo_url = $1")
        .bind(format!("{host}200/castle"))
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(req(Method::POST, "/v1/apps/castle/repo-check").header("authorization", agent))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await, Value::Null, "readable again: lifted");

    // A freeze staff placed isn't lifted by it, however readable the repository is.
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    app.send(
        req(Method::POST, "/v1/admin/apps/castle/freeze")
            .cookie(&owner)
            .json(json!({ "reason": "Checking a malware report." })),
    )
    .await;
    let res = app
        .send(req(Method::POST, "/v1/apps/castle/repo-check").cookie(&cookie))
        .await;
    let frozen = body_json(res).await;
    assert_eq!(frozen["by_staff"], true);
    assert_eq!(frozen["reason"], "Checking a malware report.");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn owners_ask_to_make_private_slop_public_and_can_take_it_back(db: PgPool) {
    let (app, host) = app_with_fake_host(db).await;
    let (owner_id, owner) = app.user("Olive Owner", "US").await;
    let (_, other) = app.user("Otto Other", "US").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    listed_with(
        &app,
        owner_id,
        "tool",
        "coming_soon",
        "status = 'draft', visibility = 'private', repo_url = 'https://github.com/x/broken'",
    )
    .await;
    listed_with(&app, owner_id, "public-tool", "coming_soon", "").await;
    let ask = |slug: &str, cookie: Option<&str>| {
        let mut r = req(Method::POST, format!("/v1/apps/{slug}/make-public"));
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        r
    };
    let requested = || async {
        sqlx::query_scalar::<_, Option<OffsetDateTime>>(
            "select public_requested_at from apps where slug = 'tool'",
        )
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    assert_eq!(
        app.send(ask("tool", None)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(ask("tool", Some(&other))).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(ask("public-tool", Some(&owner))).await.status(),
        StatusCode::CONFLICT,
        "already public"
    );
    // GitHub can't say whether the repository is public: ask again later.
    let res = app.send(ask("tool", Some(&owner))).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("couldn't confirm the repository is public")
    );
    // A licence that's an SPDX ID is needed.
    sqlx::query("update apps set repo_url = 'https://github.com/x/open', license_spdx = 'MIT; or not' where slug = 'tool'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(ask("tool", Some(&owner))).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(requested().await.is_none());
    sqlx::query("update apps set license_spdx = 'MIT' where slug = 'tool'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(ask("tool", Some(&owner))).await.status(),
        StatusCode::NO_CONTENT
    );
    let first = requested().await.unwrap();
    // Asking again keeps the first request's place.
    assert_eq!(
        app.send(ask("tool", Some(&owner))).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(requested().await, Some(first));
    let ready = |admin: &str| {
        let r = req(Method::GET, "/v1/admin/queue").cookie(admin);
        let app = &app;
        async move {
            body_json(app.send(r).await).await["ready_to_list"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["slug"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };
    assert!(ready(&admin).await.contains(&"tool".to_string()));

    // The owner changes their mind: it leaves the reviewers' list.
    let cancel = |cookie: Option<&str>| {
        let mut r = req(Method::DELETE, "/v1/apps/tool/make-public");
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        r
    };
    assert_eq!(
        app.send(cancel(None)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(cancel(Some(&other))).await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(requested().await.is_some());
    assert_eq!(
        app.send(cancel(Some(&owner))).await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(requested().await.is_none());
    assert!(!ready(&admin).await.contains(&"tool".to_string()));
    let _ = host;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_freeze_an_app_by_hand(db: PgPool) {
    let (app, host) = app_with_fake_host(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let freeze = |slug: &str, who: &str, reason: &str| {
        as_who(Method::POST, format!("/v1/admin/apps/{slug}/freeze"), who)
            .json(json!({ "reason": reason }))
    };
    let reason = "A player reported the last build steals passwords; we're checking.";
    assert_eq!(
        app.send(freeze("game", &reviewer, reason)).await.status(),
        StatusCode::FORBIDDEN
    );
    for bad in ["", " ", &"x".repeat(2001)] {
        let res = app.send(freeze("game", &owner, bad)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "reason");
    }
    assert_eq!(
        app.send(freeze("nothing", &owner, reason)).await.status(),
        StatusCode::NOT_FOUND
    );
    let agent = token(&app, &owner, "review").await;
    let res = app.send(freeze("game", &agent, reason)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let frozen = body_json(res).await;
    assert_eq!(frozen["by_staff"], true);
    assert_eq!(frozen["reason"], reason);
    let told: (String, String) = sqlx::query_as(
        "select title, body from notifications where user_id = $1 and title like 'Slop froze%'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told.0, "Slop froze Game");
    assert!(told.1.contains(reason));
    // Once: lift it first to change the reason.
    assert_eq!(
        app.send(freeze("game", &owner, "Another reason entirely."))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    // New releases are refused, with staff's words.
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(release_body(&app, "2.0.0", "game-windows.zip")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let message = body_json(res).await["error"]["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        message.starts_with(
            "Slop staff froze this app, so it can't take new releases: A player reported"
        ),
        "{message}"
    );

    // An app the automatic check froze becomes staff's, keeping when it was frozen.
    let (other_maker, _) = app.user("Otto Maker", "US").await;
    listed_with(
        &app,
        other_maker,
        "lost",
        "released",
        &format!("repo_url = '{host}404/lost'"),
    )
    .await;
    slop_api::frozen::run_once(&app.state).await.unwrap();
    sqlx::query("update apps set frozen_at = now() - interval '3 days' where slug = 'lost'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(freeze("lost", &owner, reason)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let frozen = body_json(res).await;
    assert_eq!(frozen["by_staff"], true);
    let since = OffsetDateTime::parse(frozen["since"].as_str().unwrap(), &Rfc3339).unwrap();
    assert!(OffsetDateTime::now_utc() - since > time::Duration::days(2));
    assert_eq!(
        audit_actions(&app)
            .await
            .iter()
            .filter(|a| *a == "app.frozen")
            .count(),
        2
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn lifting_a_freeze_rechecks_an_automatic_one(db: PgPool) {
    let (app, host) = app_with_fake_host(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(
        &app,
        maker,
        "fine",
        "released",
        &format!("repo_url = '{host}200/fine'"),
    )
    .await;
    listed_with(
        &app,
        maker,
        "lost",
        "released",
        &format!("repo_url = '{host}404/lost'"),
    )
    .await;
    let unfreeze = |slug: &str, who: &str, note: &str| {
        as_who(Method::POST, format!("/v1/admin/apps/{slug}/unfreeze"), who)
            .json(json!({ "note": note }))
    };
    let note = "Looked into it.";
    assert_eq!(
        app.send(unfreeze("fine", &owner, note)).await.status(),
        StatusCode::CONFLICT,
        "it isn't frozen"
    );
    assert_eq!(
        app.send(unfreeze("nothing", &owner, note)).await.status(),
        StatusCode::NOT_FOUND
    );
    slop_api::frozen::run_once(&app.state).await.unwrap();
    assert!(freeze_state(&app, "lost").await.0.is_some());
    assert_eq!(
        app.send(unfreeze("lost", &reviewer, note)).await.status(),
        StatusCode::FORBIDDEN
    );
    for bad in ["", &"x".repeat(2001)] {
        let res = app.send(unfreeze("lost", &owner, bad)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "note");
    }
    // An automatic freeze is only lifted by reading the repository: still unreadable, it stays.
    let res = app.send(unfreeze("lost", &owner, note)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["by_staff"], false);
    let rechecked: i64 = sqlx::query_scalar(
        "select count(*) from audit_log where action = 'app.repo_rechecked' and subject = 'lost'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(rechecked, 1);
    // Readable again: lifted.
    sqlx::query("update apps set repo_url = $1 where slug = 'lost'")
        .bind(format!("{host}200/lost"))
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(unfreeze("lost", &owner, note)).await;
    assert_eq!(body_json(res).await, Value::Null);
    assert_eq!(freeze_state(&app, "lost").await.0, None);
}

/// An app the 14-day grace period took out of the store, then frozen by staff on top: once staff
/// lift their freeze and Slop can read the repository, it goes back in the store.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn lifting_a_staff_freeze_on_a_delisted_app_lets_it_come_back(db: PgPool) {
    let (app, host) = app_with_fake_host(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(
        &app,
        maker,
        "castle",
        "released",
        &format!("repo_url = '{host}404/castle'"),
    )
    .await;
    slop_api::frozen::run_once(&app.state).await.unwrap();
    sqlx::query("update apps set frozen_at = now() - interval '15 days'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::frozen::end_grace(&app.state).await.unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/admin/apps/castle/freeze")
                .cookie(&owner)
                .json(json!({ "reason": "A malware report too; checking." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    // The repository is back and staff are done.
    sqlx::query("update apps set repo_url = $1")
        .bind(format!("{host}200/castle"))
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/admin/apps/castle/unfreeze")
                .cookie(&owner)
                .json(json!({ "note": "The report was false." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    slop_api::frozen::run_once(&app.state).await.unwrap();
    assert_eq!(
        freeze_state(&app, "castle").await,
        (None, false, "listed".into(), false)
    );
}

// ---------------------------------------------------------------------------------------------
// An app's record, the money overview, and staff fixes to profiles and teams

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_apps_record_holds_its_money_reports_decisions_and_release_history(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (looker_id, looker) = app.user("Lou Looker", "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(looker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let (donor, _) = app.user("Dana Donor", "US").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    let (_, maker, approved) = release_waiting(&app).await;
    let rejected = submit_and_fetch(&app, &maker, "1.1.0").await;
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/releases/{approved}/approve"),
            )
            .cookie(&owner),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/releases/{rejected}/reject"),
            )
            .cookie(&owner)
            .json(json!({ "reason": "Crashes at start." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    sqlx::query("update release_files set rescan_flagged_at = now() where release_id = $1::uuid")
        .bind(&approved)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/releases/{approved}/files/windows/rescan-clear"),
            )
            .cookie(&owner)
            .json(json!({ "reason": "A false positive in the new signatures." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    sqlx::query("update apps set status = 'listed' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    for (currency, amount, fee, status, refunded) in [
        ("usd", 500, 25, "paid", 0),
        ("usd", 300, 15, "partially_refunded", 100),
        ("usd", 900, 45, "pending", 0),
        ("eur", 400, 20, "paid", 0),
    ] {
        sqlx::query(
            "insert into donations (id, kind, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                    slop_fee_minor, processing_minor, total_minor, donor_covers_fees, status, refunded_minor)
             select $1, 'one_off', id, $2, 'acct_1', $3, $4, $5, 0, $4, false, $6, $7 from apps where slug = 'game'",
        )
        .bind(Uuid::now_v7())
        .bind(donor)
        .bind(currency)
        .bind(amount)
        .bind(fee)
        .bind(status)
        .bind(refunded)
        .execute(&app.db)
        .await
        .unwrap();
    }
    send_report(&app, Some(&rita), report_of("app", "game", "broken")).await;
    let report = newest_report(&app).await;
    app.send(
        req(Method::POST, format!("/v1/admin/reports/{report}/dismiss"))
            .cookie(&owner)
            .json(json!({ "outcome": "Works for us." })),
    )
    .await;
    send_report(&app, Some(&rita), report_of("app", "game", "misleading")).await;
    let res = app
        .send(
            req(Method::POST, "/v1/admin/apps/game/delist")
                .cookie(&owner)
                .json(json!({ "rule": "misleading", "reason": "The page shows another game." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/apps/nothing/record").cookie(&owner))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let read = token(&app, &owner, "read").await;
    assert_eq!(
        app.send(as_who(
            Method::GET,
            "/v1/admin/apps/game/record".into(),
            &read
        ))
        .await
        .status(),
        StatusCode::OK
    );
    // Staff without a role can look too.
    let record = body_json(
        app.send(req(Method::GET, "/v1/admin/apps/game/record").cookie(&looker))
            .await,
    )
    .await;
    assert_eq!(record["app"]["status"], "delisted");
    let versions: Vec<(&str, &str)> = record["releases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["version"].as_str().unwrap(),
                r["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(versions, [("1.1.0", "rejected"), ("1.0.0", "approved")]);
    assert_eq!(
        record["donations"],
        json!([
            { "currency": "eur", "donations": 1, "donated_minor": 400, "slop_fee_minor": 20, "refunded_minor": 0 },
            { "currency": "usd", "donations": 2, "donated_minor": 800, "slop_fee_minor": 40, "refunded_minor": 100 }
        ]),
        "pending donations aren't counted"
    );
    let reports: Vec<(&str, &str)> = record["reports"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["reason"].as_str().unwrap(), r["status"].as_str().unwrap()))
        .collect();
    assert_eq!(reports, [("misleading", "open"), ("broken", "dismissed")]);
    assert_eq!(record["decisions"][0]["action"], "delist_app");
    assert_eq!(record["decisions"][0]["rule"], "misleading");
    assert!(
        record["decisions"][0]["decided_by"]
            .as_str()
            .unwrap()
            .starts_with("Olive Owner (")
    );
    // The audit section: everything done to the app, its releases included.
    let audit: Vec<&str> = record["audit"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["action"].as_str().unwrap())
        .collect();
    for action in [
        "release.approve",
        "release.reject",
        "release.rescan_cleared",
        "app.delist",
        "moderation.delist_app",
    ] {
        assert!(audit.contains(&action), "{action} in {audit:?}");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_money_overview_is_for_finance_staff_only(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, finance) = staff(&app, "Fay Finance", "finance").await;
    let (_, moderator) = staff(&app, "Mo Moderator", "moderator").await;
    for (kind, amount, fee, currency, ago) in [
        ("donation", 1000, 50, "usd", "0 days"),
        ("donation", 500, 25, "usd", "0 days"),
        ("people_job", 2000, 100, "eur", "0 days"),
        ("donation", 700, 35, "usd", "400 days"),
    ] {
        sqlx::query(
            "insert into payments (id, kind, amount_minor, slop_fee_minor, currency, stripe_account_id, created_at)
             values ($1, $2, $3, $4, $5, 'acct_1', now() - $6::interval)",
        )
        .bind(Uuid::now_v7())
        .bind(kind)
        .bind(amount)
        .bind(fee)
        .bind(currency)
        .bind(ago)
        .execute(&app.db)
        .await
        .unwrap();
    }
    sqlx::query("insert into stripe_events (id, kind) values ('evt_new', 'charge.succeeded')")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into stripe_events (id, kind, received_at) values ('evt_old', 'charge.succeeded', now() - interval '2 days')",
    )
    .execute(&app.db)
    .await
    .unwrap();

    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/money").cookie(&moderator))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    // No token sees money, whatever its level.
    for scope in ["read", "review", "moderate"] {
        let t = token(&app, &owner, scope).await;
        assert_eq!(
            app.send(as_who(Method::GET, "/v1/admin/money".into(), &t))
                .await
                .status(),
            StatusCode::FORBIDDEN,
            "{scope}"
        );
    }
    let res = app
        .send(req(Method::GET, "/v1/admin/money").cookie(&finance))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let money = body_json(res).await;
    let month = OffsetDateTime::now_utc().date();
    let month = format!("{}-{:02}", month.year(), u8::from(month.month()));
    assert_eq!(
        money["months"],
        json!([
            { "month": month, "kind": "donation", "currency": "usd", "payments": 2, "amount_minor": 1500, "slop_fee_minor": 75 },
            { "month": month, "kind": "people_job", "currency": "eur", "payments": 1, "amount_minor": 2000, "slop_fee_minor": 100 }
        ]),
        "the last twelve months only"
    );
    assert_eq!(money["recent"].as_array().unwrap().len(), 4);
    assert_eq!(money["recent"][3]["amount_minor"], 700, "newest first");
    assert_eq!(money["recent"][0]["stripe_account_id"], "acct_1");
    assert_eq!(money["stripe_events_today"], 1);
    assert_eq!(money["active_server_donations"], 0);
    assert_eq!(money["payments"], "off");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_fixes_to_a_profile_are_checked_and_logged(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (cass, _) = app.user("Valve Official", "US").await;
    let (plain, _) = app.user("Pat Player", "US").await;
    live_creative_row(&app, cass, "cass").await;
    let profile =
        |id: Uuid, who: &str| as_who(Method::GET, format!("/v1/admin/users/{id}/profile"), who);
    let edit = |id: Uuid, who: &str, body: Value| {
        as_who(Method::PATCH, format!("/v1/admin/users/{id}/profile"), who).json(body)
    };

    assert_eq!(
        app.send(profile(cass, &reviewer)).await.status(),
        StatusCode::FORBIDDEN
    );
    let read = token(&app, &owner, "read").await;
    assert_eq!(
        app.send(profile(cass, &read)).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(profile(Uuid::now_v7(), &support)).await.status(),
        StatusCode::NOT_FOUND
    );
    let res = app.send(profile(plain, &support)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let seen = body_json(res).await;
    assert_eq!(seen["display_name"], "Pat Player");
    assert!(seen["creative"].is_null());
    assert_eq!(seen["teams"], json!([]));

    let reason = "Impersonating Valve.";
    assert_eq!(
        app.send(edit(
            cass,
            &reviewer,
            json!({ "display_name": "Cass", "reason": reason })
        ))
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(edit(
            Uuid::now_v7(),
            &support,
            json!({ "display_name": "Cass", "reason": reason })
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    for (who, body, field) in [
        (
            cass,
            json!({ "display_name": "Cass", "reason": "" }),
            "reason",
        ),
        (
            cass,
            json!({ "display_name": "Cass", "reason": "x".repeat(2001) }),
            "reason",
        ),
        (
            cass,
            json!({ "display_name": "", "reason": reason }),
            "display_name",
        ),
        (
            cass,
            json!({ "display_name": "x".repeat(41), "reason": reason }),
            "display_name",
        ),
        (
            cass,
            json!({ "display_name": "Bad\u{7}name", "reason": reason }),
            "display_name",
        ),
        (
            cass,
            json!({ "creative": { "name": "Two\nlines" }, "reason": reason }),
            "name",
        ),
        (
            cass,
            json!({ "creative": { "headline": "Tab\there" }, "reason": reason }),
            "headline",
        ),
        (
            cass,
            json!({ "creative": { "location": "x".repeat(61) }, "reason": reason }),
            "location",
        ),
        // Nothing that differs from what's there.
        (
            cass,
            json!({ "display_name": "Valve Official", "reason": reason }),
            "reason",
        ),
        (
            cass,
            json!({ "creative": { "remove_links": true }, "reason": reason }),
            "reason",
        ),
        // No People page to fix.
        (
            plain,
            json!({ "creative": { "headline": "Hi" }, "reason": reason }),
            "creative",
        ),
    ] {
        let res = app.send(edit(who, &support, body.clone())).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body_json(res).await["error"]["field"], field, "{body}");
    }
    let nothing_logged: i64 =
        sqlx::query_scalar("select count(*) from audit_log where action = 'user.profile_edited'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(nothing_logged, 0);

    // A review token's agent may fix it; the old values are kept in the audit log.
    let agent = token(&app, &owner, "review").await;
    let res = app
        .send(edit(
            cass,
            &agent,
            json!({
                "display_name": " Cass ",
                "creative": { "name": "Cass", "about_md": "I make music.", "location": "Leeds" },
                "reason": reason
            }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_json(res).await["changed"],
        json!(["display name", "People name", "about", "location"])
    );
    let detail: Value =
        sqlx::query_scalar("select detail from audit_log where action = 'user.profile_edited'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        detail["changed"]["display_name"],
        json!({ "from": "Valve Official", "to": "Cass" })
    );
    assert_eq!(
        detail["changed"]["name"],
        json!({ "from": "cass", "to": "Cass" })
    );
    assert_eq!(detail["reason"], reason);
    let told: (String, String) =
        sqlx::query_as("select title, body from notifications where user_id = $1")
            .bind(cass)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(told.0, "Slop changed your profile");
    assert_eq!(
        told.1,
        format!("Changed: display name, People name, about, location.\n\n{reason}")
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_fixes_to_a_team_page_tell_whoever_runs_it(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (tess, _) = app.user("Tess Teamowner", "US").await;
    let (ade, _) = app.user("Ade Admin", "US").await;
    let (mel, _) = app.user("Mel Member", "US").await;
    let team = team_of(&app, tess, "crew").await;
    for (user, role) in [(ade, "admin"), (mel, "member")] {
        sqlx::query("insert into team_members (team_id, user_id, role) values ($1, $2, $3)")
            .bind(team)
            .bind(user)
            .bind(role)
            .execute(&app.db)
            .await
            .unwrap();
    }
    let edit = |slug: &str, who: &str, body: Value| {
        as_who(Method::PATCH, format!("/v1/admin/teams/{slug}"), who).json(body)
    };
    let reason = "Not affiliated with Valve.";
    assert_eq!(
        app.send(edit(
            "crew",
            &reviewer,
            json!({ "name": "Crew", "reason": reason })
        ))
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(edit(
            "nobody",
            &owner,
            json!({ "name": "Crew", "reason": reason })
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    for (body, field) in [
        (json!({ "name": "Crew", "reason": " " }), "reason"),
        (
            json!({ "about": "x".repeat(2001), "reason": reason }),
            "about",
        ),
        (json!({ "name": "Two\nlines", "reason": reason }), "name"),
        (
            json!({ "name": "The crew", "about": "", "reason": reason }),
            "reason",
        ),
        (json!({ "reason": reason }), "reason"),
    ] {
        let res = app.send(edit("crew", &owner, body.clone())).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body_json(res).await["error"]["field"], field, "{body}");
    }
    let res = app
        .send(edit(
            "crew",
            &owner,
            json!({ "about": "We play co-op games on Fridays.", "reason": reason }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["changed"], json!(["description"]));
    let told: Vec<Uuid> = sqlx::query_scalar(
        "select user_id from notifications where title = 'Slop changed your team''s page' order by user_id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let mut runners = vec![tess, ade];
    runners.sort();
    assert_eq!(told, runners, "the owner and admins, not members");
    let (subject, detail): (String, Value) =
        sqlx::query_as("select subject, detail from audit_log where action = 'team.edited'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(subject, "team crew");
    assert_eq!(
        detail["changed"]["about"],
        json!({ "from": "", "to": "We play co-op games on Fridays." })
    );
}

// ---------------------------------------------------------------------------------------------
// Abuse signals

/// `n` votes on an app from accounts that don't exist as users (so they're never "new
/// accounts"), `ago` (an SQL interval) ago.
async fn votes(app: &TestApp, slug: &str, n: i32, vote: i16, prev: Option<i16>, ago: &str) {
    sqlx::query(
        "insert into vote_events (id, user_id, app_id, vote, prev_vote, weight, created_at)
         select gen_random_uuid(), gen_random_uuid(), a.id, $2, $3, 1, now() - $4::interval
         from apps a, generate_series(1, $5) where a.slug = $1",
    )
    .bind(slug)
    .bind(vote)
    .bind(prev)
    .bind(ago)
    .bind(n)
    .execute(&app.db)
    .await
    .unwrap();
}

/// The open signals: (kind, subject label), sorted.
async fn signals(app: &TestApp) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> =
        sqlx::query_as("select kind, subject_label from abuse_signals where status = 'open'")
            .fetch_all(&app.db)
            .await
            .unwrap();
    out.sort();
    out
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn abuse_checks_find_review_bombs_thin_votes_and_swings_at_their_thresholds(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    for slug in [
        "bombed",
        "nearly-bombed",
        "bombed-at-five-times",
        "usually-hated",
        "thin",
        "nearly-thin",
        "played",
        "swung",
        "nearly-swung",
    ] {
        listed_with(&app, maker, slug, "released", "").await;
    }
    // Review bombs: ten 👎 in a day, and five times the usual (at least one a day).
    votes(&app, "bombed", 10, -1, None, "1 hour").await;
    votes(&app, "nearly-bombed", 9, -1, None, "1 hour").await;
    votes(&app, "bombed-at-five-times", 10, -1, None, "1 hour").await;
    votes(&app, "bombed-at-five-times", 60, -1, None, "10 days").await;
    votes(&app, "usually-hated", 10, -1, None, "1 hour").await;
    votes(&app, "usually-hated", 61, -1, None, "10 days").await;
    // Thin votes: fifteen voters in a week who barely played, at least half of them.
    votes(&app, "thin", 15, 1, None, "3 days").await;
    votes(&app, "nearly-thin", 14, 1, None, "3 days").await;
    votes(&app, "played", 7, 1, None, "3 days").await;
    let played: Vec<Uuid> = sqlx::query_scalar(
        "insert into users (id, display_name, country_code, age_16_confirmed_at, created_at)
         select gen_random_uuid(), 'Player ' || i, 'US', now(), now() - interval '1 year'
         from generate_series(1, 8) i returning id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let played_id = app_id(&app, "played").await;
    for (i, user) in played.iter().enumerate() {
        // Half an hour's play, or a few minutes on several days: either is really playing.
        let (seconds, days) = if i % 2 == 0 { (1800, 1) } else { (300, 3) };
        sqlx::query(
            "insert into user_app_usage (user_id, app_id, playtime_seconds, days_launched) values ($1, $2, $3, $4)",
        )
        .bind(user)
        .bind(played_id)
        .bind(seconds)
        .bind(days)
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query(
            "insert into vote_events (id, user_id, app_id, vote, weight, created_at) values ($1, $2, $3, 1, 1, now() - interval '2 days')",
        )
        .bind(Uuid::now_v7())
        .bind(user)
        .bind(played_id)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Vote swings: ten players changing their vote the same way in a day.
    votes(&app, "swung", 10, 1, Some(-1), "2 hours").await;
    votes(&app, "nearly-swung", 9, 1, Some(-1), "2 hours").await;
    votes(&app, "nearly-swung", 5, -1, Some(1), "2 hours").await;

    slop_api::abuse::run_once(&app.state).await.unwrap();
    assert_eq!(
        signals(&app).await,
        [
            ("review_bomb".to_string(), "bombed".to_string()),
            ("review_bomb".into(), "bombed-at-five-times".into()),
            ("thin_votes".into(), "thin".into()),
            ("vote_swing".into(), "swung".into()),
        ]
    );
    let detail: Value = sqlx::query_scalar(
        "select detail from abuse_signals where kind = 'review_bomb' and subject_label = 'bombed-at-five-times'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        detail,
        json!({ "slop_votes_today": 10, "usual_per_day": 2.0 })
    );
    let detail: Value =
        sqlx::query_scalar("select detail from abuse_signals where kind = 'vote_swing'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        detail,
        json!({ "changed_to": "not_slop", "players": 10, "window": "24 hours" })
    );
    // Each is raised once.
    assert_eq!(slop_api::abuse::run_once(&app.state).await.unwrap(), 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn abuse_checks_find_lookalike_names_bad_links_and_accounts_hitting_limits(db: PgPool) {
    put_raw(
        "admin-1-feed",
        "bad.txt",
        b"# a malware list\nhttps://evil.example/steal\n".to_vec(),
    );
    let fake = fake_github().await;
    let app = test_app_with(db, move |c| {
        c.bad_link_feeds = vec![
            format!("{fake}raw/octo/game/admin-1-feed/.slop/bad.txt")
                .parse()
                .unwrap(),
        ]
    })
    .await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let (rival, _) = app.user("Rob Rival", "US").await;
    let (modder, _) = app.user("Mo Modder", "US").await;
    let (cass, _) = app.user("Cass Creative", "US").await;
    let (pat, _) = app.user("Pat Player", "US").await;
    let (busy, _) = app.user("Busy Body", "US").await;
    let (calm, _) = app.user("Calm Person", "US").await;
    listed_with(
        &app,
        maker,
        "stardew",
        "released",
        "title = 'Stardew Valley'",
    )
    .await;
    // The same maker's sequel, and a mod of it: not lookalikes.
    listed_with(
        &app,
        maker,
        "stardew-2",
        "released",
        "title = 'Stardew Va1ley', status = 'draft'",
    )
    .await;
    let original = app_id(&app, "stardew").await;
    listed_with(
        &app,
        modder,
        "stardew-mod",
        "released",
        "title = 'Stardew Valley'",
    )
    .await;
    sqlx::query(
        "update apps set parent_app_id = $1, branch_kind = 'mod' where slug = 'stardew-mod'",
    )
    .bind(original)
    .execute(&app.db)
    .await
    .unwrap();
    listed_with(
        &app,
        rival,
        "stardew-copy",
        "released",
        "title = 'Stardew  Valley!', status = 'draft'",
    )
    .await;
    // Bad links on a creative's page and in a review.
    live_creative_row(&app, cass, "cass").await;
    sqlx::query("update creatives set about_md = 'Free assets at https://evil.example/steal' where user_id = $1")
        .bind(cass)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into app_reviews (id, user_id, app_id, body) values ($1, $2, $3, 'Get it cheaper: https://EVIL.example/steal/')",
    )
    .bind(Uuid::now_v7())
    .bind(pat)
    .bind(original)
    .execute(&app.db)
    .await
    .unwrap();
    // Accounts that keep hitting their limits: twenty refusals in a day is a signal.
    let now = std::time::Instant::now();
    for (user, tries) in [(busy, 30), (calm, 29)] {
        for _ in 0..tries {
            let _ = app
                .state
                .account_limiter
                .take(user, slop_api::limits::Action::Review, now);
        }
    }

    slop_api::abuse::run_once(&app.state).await.unwrap();
    assert_eq!(
        signals(&app).await,
        [
            ("bad_link".to_string(), "Pat Player".to_string()),
            ("bad_link".into(), "cass".into()),
            ("lookalike_name".into(), "Stardew  Valley!".into()),
            ("rate_limited".into(), "Busy Body".into()),
        ]
    );
    let (kind, detail): (String, Value) = sqlx::query_as(
        "select subject_kind, detail from abuse_signals where kind = 'rate_limited'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(kind, "account");
    assert_eq!(detail, json!({ "limit_hits_today": 20 }));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_bad_link_list_that_cannot_be_fetched_does_not_stop_the_other_checks(db: PgPool) {
    let fake = fake_github().await;
    let app = test_app_with(db, move |c| {
        c.bad_link_feeds = vec![
            format!("{fake}raw/octo/game/no-such-feed/.slop/bad.txt")
                .parse()
                .unwrap(),
        ]
    })
    .await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker, "bombed", "released", "").await;
    votes(&app, "bombed", 12, -1, None, "1 hour").await;
    assert_eq!(slop_api::abuse::run_once(&app.state).await.unwrap(), 1);
    let lists: i64 = sqlx::query_scalar("select count(*) from bad_links")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(lists, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_abuse_signal_is_closed_with_a_note(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker, "bombed", "released", "").await;
    votes(&app, "bombed", 12, -1, None, "1 hour").await;
    slop_api::abuse::run_once(&app.state).await.unwrap();
    let signal: Uuid = sqlx::query_scalar("select id from abuse_signals")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let close = |id: Uuid, who: &str, body: Value| {
        as_who(Method::POST, format!("/v1/admin/abuse/{id}/dismiss"), who).json(body)
    };
    let note = json!({ "note": "A streamer told their chat to play it; real players." });
    assert_eq!(
        app.send(close(signal, &reviewer, note.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let read = token(&app, &owner, "read").await;
    assert_eq!(
        app.send(close(signal, &read, note.clone())).await.status(),
        StatusCode::FORBIDDEN
    );
    for bad in ["", "  ", &"x".repeat(1001)] {
        let res = app
            .send(close(signal, &support, json!({ "note": bad })))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "note");
    }
    assert_eq!(
        app.send(close(Uuid::now_v7(), &support, note.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Nothing wrong: dismissed (acted defaults to false).
    assert_eq!(
        app.send(close(signal, &support, note.clone()))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let (status, by, why): (String, String, String) =
        sqlx::query_as("select status, decided_by, decision_note from abuse_signals where id = $1")
            .bind(signal)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "dismissed");
    assert!(by.starts_with("Sue Support ("));
    assert_eq!(why, "A streamer told their chat to play it; real players.");
    let detail: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'abuse.close' and subject = $1",
    )
    .bind(signal.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(detail["acted"], false);
    // Closed already.
    assert_eq!(
        app.send(close(signal, &support, note)).await.status(),
        StatusCode::NOT_FOUND
    );
}

// ---------------------------------------------------------------------------------------------
// Ops alerts

/// How many times admin `user` was told something with this title (repeats collapse into one
/// notification with a count).
async fn alerted(app: &TestApp, user: Uuid, title: &str) -> i64 {
    sqlx::query_scalar(
        "select coalesce(sum(count), 0)::int8 from notifications where user_id = $1 and category = 'ops' and title = $2",
    )
    .bind(user)
    .bind(title)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn urgent_reports_are_alerted_once_and_old_ones_never(db: PgPool) {
    let app = test_app(db).await;
    let (admin_id, _) = make_admin(&app, "Ada Admin").await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let (_, rita) = app.user("Rita", "US").await;
    let (_, sam) = app.user("Sam", "US").await;
    listed_with(&app, maker, "castle", "released", "").await;
    // A report from before the alerts started isn't news.
    send_report(&app, Some(&rita), report_of("app", "castle", "child_abuse")).await;
    slop_api::ops::run_once(&app.state).await.unwrap();
    let urgent = "Urgent report: act within the hour";
    assert_eq!(alerted(&app, admin_id, urgent).await, 0);
    // A new one is, once.
    send_report(
        &app,
        Some(&sam),
        report_of("app", "castle", "intimate_image"),
    )
    .await;
    slop_api::ops::run_once(&app.state).await.unwrap();
    assert_eq!(alerted(&app, admin_id, urgent).await, 1);
    for _ in 0..3 {
        slop_api::ops::run_once(&app.state).await.unwrap();
    }
    assert_eq!(
        alerted(&app, admin_id, urgent).await,
        1,
        "not again every five minutes"
    );
    // Serious ones too; everything else waits for the queue.
    let (_, tom) = app.user("Tom", "US").await;
    let (_, una) = app.user("Una", "US").await;
    send_report(&app, Some(&tom), report_of("app", "castle", "malware")).await;
    send_report(&app, Some(&una), report_of("app", "castle", "broken")).await;
    slop_api::ops::run_once(&app.state).await.unwrap();
    slop_api::ops::run_once(&app.state).await.unwrap();
    assert_eq!(
        alerted(&app, admin_id, "Serious report: act within 24 hours").await,
        1
    );
    let all: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and title like '%report:%'",
    )
    .bind(admin_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(all, 2);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn conditions_needing_a_person_are_alerted_daily_while_they_last(db: PgPool) {
    let deploy = std::env::temp_dir().join(format!("slop-deploy-test-{}", Uuid::now_v7()));
    std::fs::create_dir_all(deploy.join("builds/failed-0123456789abcdef")).unwrap();
    std::fs::create_dir_all(deploy.join("builds/failed-aaaaaaaaaaaaaaaa")).unwrap();
    let old = std::fs::File::open(deploy.join("builds/failed-aaaaaaaaaaaaaaaa")).unwrap();
    old.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86_400))
        .unwrap();
    std::fs::create_dir_all(deploy.join("builds/ok-bbbbbbb")).unwrap();
    std::fs::write(
        deploy.join("audit.json"),
        r#"{"vulnerabilities": [{"package": "zip", "version": "4.6.1", "id": "RUSTSEC-1"}, {"package": "tar", "version": "0.4", "id": "RUSTSEC-2"}]}"#,
    )
    .unwrap();
    let dir = deploy.clone();
    let app = test_app_with(db, move |c| c.ops.deploy_dir = Some(dir)).await;
    let (admin_id, admin) = make_admin(&app, "Ada Admin").await;
    let (_, player) = app.user("Pat Player", "US").await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    // A report past its deadline, an open abuse signal, a release waiting two days, a scanner
    // that failed, and a removal order due within the hour.
    sqlx::query("update apps set status = 'listed'")
        .execute(&app.db)
        .await
        .unwrap();
    send_report(&app, Some(&player), report_of("app", "game", "broken")).await;
    sqlx::query("update reports set due_at = now() - interval '1 hour'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into abuse_signals (id, kind, key, subject_kind, subject_id, subject_label, detail)
         values ($1, 'vote_burst', 'k', 'app', $1, 'game', '{}')",
    )
    .bind(Uuid::now_v7())
    .execute(&app.db)
    .await
    .unwrap();
    let waiting = submit_and_fetch(&app, &maker, "1.0.0").await;
    sqlx::query("update releases set created_at = now() - interval '2 days' where id = $1::uuid")
        .bind(&waiting)
        .execute(&app.db)
        .await
        .unwrap();
    let failed = submit_and_fetch(&app, &maker, "1.1.0").await;
    sqlx::query(
        "update release_files set scan_detail = 'the scanner couldn''t finish: timed out' where release_id = $1::uuid",
    )
    .bind(&failed)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into legal_requests (id, reference, received_at, country, authority, kind, asked, deadline, created_by)
         values ($1, 'LR-1', now(), 'FR', 'OCLCTIC', 'removal_order', 'Take down the castle', now() + interval '40 minutes', 'test')",
    )
    .bind(Uuid::now_v7())
    .execute(&app.db)
    .await
    .unwrap();
    let _ = maker_id;

    slop_api::ops::run_once(&app.state).await.unwrap();
    let alerts = body_json(
        app.send(req(Method::GET, "/v1/admin/ops").cookie(&admin))
            .await,
    )
    .await;
    let mut titles: Vec<&str> = alerts
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["title"].as_str().unwrap())
        .filter(|t| !t.starts_with("The disk at"))
        .collect();
    titles.sort();
    assert_eq!(
        titles,
        [
            "1 possible abuse pattern to look at",
            "1 release waiting over a day for review",
            "1 report past its deadline",
            "2 known-vulnerable dependencies",
            "Removal order LR-1: act within the hour",
        ],
        "conditions are listed while they last; events (the deploy, the scanner) are sent and done"
    );
    for title in [
        "Commit 0123456 didn't deploy",
        "The scanner failed on game 1.1.0",
        "1 report past its deadline",
    ] {
        assert_eq!(alerted(&app, admin_id, title).await, 1, "{title}");
    }
    assert_eq!(
        alerted(&app, admin_id, "Commit aaaaaaa didn't deploy").await,
        0,
        "older than two days"
    );
    // Five minutes later: nothing new is sent.
    assert_eq!(slop_api::ops::run_once(&app.state).await.unwrap(), 0);
    // A day later, what still holds is sent again; the events aren't.
    sqlx::query("update ops_alerts set last_sent = now() - interval '25 hours'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::ops::run_once(&app.state).await.unwrap();
    assert_eq!(
        alerted(&app, admin_id, "1 report past its deadline").await,
        2
    );
    assert_eq!(
        alerted(&app, admin_id, "Commit 0123456 didn't deploy").await,
        1
    );
    // Dealt with: the alert clears.
    sqlx::query("update reports set status = 'dismissed'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::ops::run_once(&app.state).await.unwrap();
    let alerts = body_json(
        app.send(req(Method::GET, "/v1/admin/ops").cookie(&admin))
            .await,
    )
    .await;
    assert!(
        !alerts
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["key"] == "reports-overdue")
    );
    std::fs::remove_dir_all(&deploy).unwrap();
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn events_from_the_audit_log_are_alerted_with_their_own_titles(db: PgPool) {
    let app = test_app(db).await;
    let (admin_id, admin) = make_admin(&app, "Ada Admin").await;
    let (pat_id, _) = app.user("Pat Player", "US").await;
    slop_api::ops::run_once(&app.state).await.unwrap();
    for (action, subject) in [
        ("admin.grant", "user-1"),
        ("admin.revoke", "user-2"),
        ("payments.mismatch", "pi_123"),
        ("release.rescan_flagged", "rel-1 windows"),
        ("app.list", "castle"),
    ] {
        sqlx::query("insert into audit_log (actor, action, subject) values ('Someone', $1, $2)")
            .bind(action)
            .bind(subject)
            .execute(&app.db)
            .await
            .unwrap();
    }
    // Making someone staff from the panel.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/staff/{pat_id}/roles"))
                .cookie(&admin)
                .json(json!({ "role": "reviewer", "grant": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    slop_api::ops::run_once(&app.state).await.unwrap();
    let told: Vec<(String, String)> = sqlx::query_as(
        "select title, body from notifications where user_id = $1 and category = 'ops' order by created_at",
    )
    .bind(admin_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    let titles: Vec<&str> = told.iter().map(|t| t.0.as_str()).collect();
    assert!(titles.contains(&"Someone was made an admin"), "{titles:?}");
    assert!(titles.contains(&"Admin rights were removed"), "{titles:?}");
    assert!(
        titles.contains(&"Payments don't match Stripe"),
        "{titles:?}"
    );
    let mismatch = told
        .iter()
        .filter(|t| t.0 == "Payments don't match Stripe")
        .count();
    assert_eq!(mismatch, 1, "only the payments event: {titles:?}");
    let flagged = told
        .iter()
        .find(|t| t.1.contains("rel-1 windows"))
        .expect("the re-scan is alerted");
    assert!(flagged.0.contains("re-scan"), "{flagged:?}");
    let granted = told
        .iter()
        .find(|t| t.1.contains(&pat_id.to_string()))
        .expect("a staff role given from the panel is alerted");
    assert!(granted.1.contains("reviewer"), "{granted:?}");
    assert!(
        !told.iter().any(|t| t.1.contains("castle")),
        "routine actions aren't alerted"
    );
    // Each once.
    assert_eq!(slop_api::ops::run_once(&app.state).await.unwrap(), 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_ops_page_shows_the_newest_fifty_unresolved_alerts(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    sqlx::query(
        "insert into ops_alerts (key, title, detail, link, first_seen, resolved_at)
         select 'k' || i, 'Alert ' || i, 'd', '/admin', now() - make_interval(mins => i),
                case when i <= 3 then now() end
         from generate_series(1, 60) i",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let read = token(&app, &admin, "read").await;
    let shown = body_json(
        app.send(as_who(Method::GET, "/v1/admin/ops".into(), &read))
            .await,
    )
    .await;
    let keys: Vec<&str> = shown
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys.len(), 50);
    assert_eq!(keys[0], "k4", "resolved ones are gone; newest first");
    assert_eq!(keys[49], "k53");
}

// ---------------------------------------------------------------------------------------------
// Rate limits: per client address, and per account

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn writes_are_limited_per_client_address(db: PgPool) {
    let app = test_app(db).await;
    let write = |ip: Option<&str>| {
        let mut r = req(Method::POST, "/v1/reports").json(json!({}));
        if let Some(ip) = ip {
            r = r.header("x-test-client-ip", ip.to_string());
        }
        r
    };
    // Sixty writes a minute; the sixty-first waits.
    for _ in 0..60 {
        let res = app.send(write(Some("10.0.0.1"))).await;
        assert_ne!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    }
    let res = app.send(write(Some("10.0.0.1"))).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = res.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=2).contains(&wait), "{wait}");
    assert_eq!(body_json(res).await["error"]["code"], "rate_limited");
    // Reads are a separate allowance; another client has its own.
    assert_eq!(
        app.send(req(Method::GET, "/v1/legal").header("x-test-client-ip", "10.0.0.1"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_ne!(
        app.send(write(Some("10.0.0.2"))).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // Behind a load balancer the client is the last address in the list.
    assert_eq!(
        app.send(write(Some("192.0.2.9, 10.0.0.1"))).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_ne!(
        app.send(write(Some("10.0.0.1, 192.0.2.9"))).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // GitHub's webhooks and the health check are never limited.
    for _ in 0..70 {
        let res = app
            .send(
                req(Method::POST, "/v1/github/webhook")
                    .header("x-test-client-ip", "10.0.0.1")
                    .json(json!({})),
            )
            .await;
        assert_ne!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    }
    let res = app
        .send(req(Method::GET, "/healthz").header("x-test-client-ip", "10.0.0.1"))
        .await;
    assert_ne!(res.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn clients_the_server_cannot_tell_apart_share_one_allowance(db: PgPool) {
    let app = test_app(db).await;
    let write = |ip: Option<&str>| {
        let mut r = req(Method::POST, "/v1/reports").json(json!({}));
        if let Some(ip) = ip {
            r = r.header("x-test-client-ip", ip.to_string());
        }
        r
    };
    for i in 0..60 {
        let ip = if i % 2 == 0 {
            Some("not an address")
        } else {
            None
        };
        assert_ne!(
            app.send(write(ip)).await.status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    for ip in [Some("garbage"), Some(""), None] {
        assert_eq!(
            app.send(write(ip)).await.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "{ip:?}"
        );
    }
    assert_ne!(
        app.send(write(Some("10.0.0.3"))).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn every_account_action_has_its_own_allowance(db: PgPool) {
    use slop_api::limits::{AccountLimiter, Action};
    let _ = db;
    let limiter = AccountLimiter::default();
    let user = Uuid::now_v7();
    let t0 = std::time::Instant::now();
    let hour = std::time::Duration::from_secs(3600);
    let day = std::time::Duration::from_secs(86_400);
    let minute = std::time::Duration::from_secs(60);
    for (action, n, per, name) in [
        (Action::Vote, 60, hour, "votes"),
        (Action::Review, 10, hour, "reviews"),
        (Action::Report, 10, hour, "reports"),
        (Action::ChatMessage, 60, minute, "chat messages"),
        (Action::FriendRequest, 40, day, "friend requests"),
        (Action::Donation, 10, hour, "donations"),
        (Action::Share, 60, hour, "shares"),
        (Action::Interest, 120, hour, "Interested marks"),
        (Action::TeamInvite, 50, day, "team invitations"),
        (Action::NewApp, 10, day, "new apps"),
        (Action::Release, 30, hour, "releases"),
        (Action::Hire, 20, day, "hiring requests"),
        (Action::JobMessage, 60, minute, "job messages"),
        (Action::Offer, 5, day, "offers to other makers"),
        (Action::Lookup, 30, hour, "repository lookups"),
    ] {
        assert_eq!(action.limit(), (n, per), "{name}");
        assert_eq!(action.name(), name);
        for _ in 0..n {
            assert!(limiter.take(user, action, t0).is_ok(), "{name}");
        }
        let wait = limiter.take(user, action, t0).unwrap_err();
        // One comes back after per / n.
        let one = per.as_secs_f64() / f64::from(n);
        assert_eq!(wait.as_secs(), one.ceil().max(1.0) as u64, "{name}");
        assert!(
            limiter
                .take(
                    user,
                    action,
                    t0 + std::time::Duration::from_secs_f64(one + 0.01)
                )
                .is_ok(),
            "{name} refills"
        );
    }
    // Fifteen kinds of refusal, each counted towards the abuse checks' signal.
    assert_eq!(limiter.frequent(15), vec![(user, 15)]);
    assert!(limiter.frequent(16).is_empty());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn players_vote_at_most_sixty_times_an_hour(db: PgPool) {
    let app = test_app_with(db, |c| c.rate_limits.write_per_minute = 10_000).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    let (_, player) = app.user("Pat Player", "US").await;
    listed_with(&app, maker, "castle", "released", "").await;
    app.send(req(Method::POST, "/v1/apps/castle/launch?platform=windows").cookie(&player))
        .await;
    let vote = |v: &str| {
        req(Method::POST, "/v1/apps/castle/vote")
            .cookie(&player)
            .json(json!({ "vote": v }))
    };
    for i in 0..60 {
        let res = app
            .send(vote(if i % 2 == 0 { "slop" } else { "not_slop" }))
            .await;
        assert_eq!(res.status(), StatusCode::OK, "{i}");
    }
    let res = app.send(vote("slop")).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("votes")
    );
}

// ---------------------------------------------------------------------------------------------
// The hourly cleanup

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn cleanup_ends_expired_sign_in_flows_tokens_and_idle_desktops(db: PgPool) {
    let app = test_app(db).await;
    let (user, _) = app.user("Pat Player", "US").await;
    for (hash, expires) in [(vec![1u8], "-1 minute"), (vec![2u8], "10 minutes")] {
        sqlx::query(
            "insert into pending_signups (token_hash, provider, provider_subject, suggested_name, return_to, expires_at)
             values ($1, 'github', $2, 'Pat', '/', now() + $3::interval)",
        )
        .bind(&hash)
        .bind(format!("{}", hash[0]))
        .bind(expires)
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query(
            "insert into desktop_auth_requests (id, code_challenge, redirect_port, state, device_name, expires_at)
             values ($1, 'c', 49152, 's', 'Laptop', now() + $2::interval)",
        )
        .bind(Uuid::now_v7())
        .bind(expires)
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query(
            "insert into webauthn_challenges (id, kind, user_id, state, expires_at)
             values ($1, 'register', $2, '{}', now() + $3::interval)",
        )
        .bind(Uuid::now_v7())
        .bind(user)
        .bind(expires)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Desktop sessions: one in use, one idle past its 30 days and the week kept after.
    let desktop = |last_seen: &str| {
        let db = app.db.clone();
        let last_seen = last_seen.to_string();
        async move {
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into sessions (id, user_id, kind, expires_at, last_seen_at)
                 values ($1, $2, 'desktop', now() + interval '1 year', now() - $3::interval)",
            )
            .bind(id)
            .bind(user)
            .bind(last_seen)
            .execute(&db)
            .await
            .unwrap();
            id
        }
    };
    let in_use = desktop("20 days").await;
    let idle = desktop("38 days").await;
    for (hash, expires) in [(vec![3u8], "-1 minute"), (vec![4u8], "10 minutes")] {
        sqlx::query("insert into access_tokens (token_hash, session_id, expires_at) values ($1, $2, now() + $3::interval)")
            .bind(&hash)
            .bind(in_use)
            .bind(expires)
            .execute(&app.db)
            .await
            .unwrap();
        sqlx::query("insert into refresh_tokens (token_hash, session_id, expires_at) values ($1, $2, now() + $3::interval)")
            .bind(&hash)
            .bind(in_use)
            .bind(expires)
            .execute(&app.db)
            .await
            .unwrap();
    }

    let deleted = slop_api::cleanup::run_once(&app.db).await.unwrap();
    assert_eq!(
        deleted,
        slop_api::cleanup::Deleted {
            oauth_flows: 0,
            pending_signups: 1,
            desktop_requests: 1,
            access_tokens: 1,
            refresh_tokens: 1,
            sessions: 1,
        }
    );
    let left: Vec<Uuid> = sqlx::query_scalar("select id from sessions where kind = 'desktop'")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(left, [in_use]);
    let _ = idle;
    let (signups, requests, challenges, access, refresh): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "select (select count(*) from pending_signups), (select count(*) from desktop_auth_requests),
                (select count(*) from webauthn_challenges), (select count(*) from access_tokens),
                (select count(*) from refresh_tokens)",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (signups, requests, challenges, access, refresh),
        (1, 1, 1, 1, 1)
    );
    assert_eq!(
        slop_api::cleanup::run_once(&app.db).await.unwrap(),
        slop_api::cleanup::Deleted::default()
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn cleanup_forgets_old_notifications_emails_and_chat_deliveries(db: PgPool) {
    let app = test_app(db).await;
    let (user, _) = app.user("Pat Player", "US").await;
    // Notifications: read ones after 180 days, any after a year.
    for (title, created, read) in [
        ("read long ago", "200 days", Some("181 days")),
        ("read lately", "200 days", Some("170 days")),
        ("never read, a year", "366 days", None),
        ("never read", "300 days", None),
    ] {
        sqlx::query(
            "insert into notifications (id, user_id, category, title, link, created_at, read_at)
             values ($1, $2, 'moderation', $3, '/', now() - $4::interval, now() - $5::interval)",
        )
        .bind(Uuid::now_v7())
        .bind(user)
        .bind(title)
        .bind(created)
        .bind(read)
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Emails: 30 days after they went (or failed for good).
    for (to, sent, failed) in [
        ("sent-old@example.com", Some("31 days"), None),
        ("sent-new@example.com", Some("29 days"), None),
        ("failed-old@example.com", None, Some("31 days")),
        ("waiting@example.com", None, None),
    ] {
        sqlx::query(
            "insert into email_outbox (id, to_address, subject, body, created_at, sent_at, failed_at)
             values ($1, $2, 's', 'b', now() - interval '40 days', now() - $3::interval, now() - $4::interval)",
        )
        .bind(Uuid::now_v7())
        .bind(to)
        .bind(sent)
        .bind(failed)
        .execute(&app.db)
        .await
        .unwrap();
    }
    slop_api::notify::cleanup(&app.db).await.unwrap();
    let mut titles = notice_titles(&app, user).await;
    titles.sort();
    assert_eq!(titles, ["never read", "read lately"]);
    let mut kept: Vec<String> = sqlx::query_scalar("select to_address from email_outbox")
        .fetch_all(&app.db)
        .await
        .unwrap();
    kept.sort();
    assert_eq!(kept, ["sent-new@example.com", "waiting@example.com"]);

    // Chat: encrypted deliveries after 30 days, key packages of revoked devices, and devices
    // nobody approved within a day.
    let device = |status: &str, created: &str, key: u8| {
        let db = app.db.clone();
        let (status, created) = (status.to_string(), created.to_string());
        async move {
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into chat_devices (id, user_id, name, signature_key, status, created_at)
                 values ($1, $2, 'Phone', $3, $4, now() - $5::interval)",
            )
            .bind(id)
            .bind(user)
            .bind(vec![key; 32])
            .bind(status)
            .bind(created)
            .execute(&db)
            .await
            .unwrap();
            sqlx::query(
                "insert into chat_key_packages (id, device_id, key_package) values ($1, $2, '\\x01')",
            )
            .bind(Uuid::now_v7())
            .bind(id)
            .execute(&db)
            .await
            .unwrap();
            id
        }
    };
    let active = device("active", "60 days", 1).await;
    let revoked = device("revoked", "60 days", 2).await;
    let waiting = device("pending", "2 days", 3).await;
    let fresh = device("pending", "1 hour", 4).await;
    let conversation = Uuid::now_v7();
    sqlx::query("insert into chat_conversations (id, kind, created_by) values ($1, 'group', $2)")
        .bind(conversation)
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    for age in ["31 days", "29 days"] {
        sqlx::query(
            "insert into chat_messages (conversation_id, sender_user_id, kind, epoch, body, created_at)
             values ($1, $2, 'application', 0, '\\x01', now() - $3::interval)",
        )
        .bind(conversation)
        .bind(user)
        .bind(age)
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query(
            "insert into chat_welcomes (id, device_id, conversation_id, welcome, after_seq, created_at)
             values ($1, $2, $3, '\\x01', 0, now() - $4::interval)",
        )
        .bind(Uuid::now_v7())
        .bind(active)
        .bind(conversation)
        .bind(age)
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query(
            "insert into chat_attachments (id, conversation_id, uploader_id, size_bytes, body, created_at)
             values ($1, $2, $3, 1, '\\x01', now() - $4::interval)",
        )
        .bind(Uuid::now_v7())
        .bind(conversation)
        .bind(user)
        .bind(age)
        .execute(&app.db)
        .await
        .unwrap();
    }
    assert_eq!(slop_api::chat::cleanup(&app.db).await.unwrap(), 4);
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "select (select count(*) from chat_messages), (select count(*) from chat_welcomes),
                (select count(*) from chat_attachments),
                (select count(*) from chat_key_packages where device_id = $1)",
    )
    .bind(revoked)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(counts, (1, 1, 1, 0));
    assert_eq!(slop_api::chat::expire_pending(&app.db).await.unwrap(), 1);
    let statuses: Vec<(Uuid, String)> =
        sqlx::query_as("select id, status from chat_devices where status = 'pending'")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(statuses, [(fresh, "pending".to_string())]);
    let revoked_now: bool = sqlx::query_scalar(
        "select status = 'revoked' and revoked_at is not null from chat_devices where id = $1",
    )
    .bind(waiting)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(revoked_now);
    assert_eq!(
        slop_api::chat::cleanup(&app.db).await.unwrap(),
        1,
        "its key package"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn old_store_thumbnails_get_hashed_a_few_at_a_time(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Mia Maker", "US").await;
    put_raw("admin-1-thumbs", "thumb.png", png(460, 215));
    let readable = format!(
        "{}raw/octo/game/admin-1-thumbs/.slop/thumb.png",
        app.fake_base
    );
    let missing = format!(
        "{}raw/octo/game/admin-1-thumbs/.slop/gone.png",
        app.fake_base
    );
    for (slug, thumbnail) in [
        ("readable", json!({ "path": "thumb.png", "url": readable })),
        ("missing", json!({ "path": "gone.png", "url": missing })),
        (
            "hashed",
            json!({ "path": "x.png", "url": readable, "dhash": "00ff00ff00ff00ff" }),
        ),
        ("no-thumbnail", Value::Null),
    ] {
        listed_with(&app, maker, slug, "released", "").await;
        let set = Uuid::now_v7();
        sqlx::query(
            "insert into media_sets (id, app_id, commit_sha, thumbnail, fingerprint, status, source)
             select $1, id, repeat('a', 40), $2, 'f', 'approved', 'maker' from apps where slug = $3",
        )
        .bind(set)
        .bind(if thumbnail.is_null() { None } else { Some(thumbnail) })
        .bind(slug)
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query("update apps set media_set_id = $1 where slug = $2")
            .bind(set)
            .bind(slug)
            .execute(&app.db)
            .await
            .unwrap();
    }
    assert_eq!(
        slop_api::media::hash_old_thumbnails(&app.state, 1)
            .await
            .unwrap(),
        1,
        "a few at a time"
    );
    assert_eq!(
        slop_api::media::hash_old_thumbnails(&app.state, 20)
            .await
            .unwrap(),
        1
    );
    let hashes: Vec<(String, Option<String>)> = sqlx::query_as(
        "select a.slug, ms.thumbnail->>'dhash' from apps a join media_sets ms on ms.id = a.media_set_id order by a.slug",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let get = |slug: &str| hashes.iter().find(|h| h.0 == slug).unwrap().1.clone();
    assert_eq!(
        get("hashed").as_deref(),
        Some("00ff00ff00ff00ff"),
        "left alone"
    );
    assert_eq!(get("no-thumbnail"), None);
    assert_eq!(
        get("missing").as_deref(),
        Some(""),
        "an unreadable picture isn't tried again"
    );
    let hash = get("readable").unwrap();
    assert_eq!(hash.len(), 16);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(
        slop_api::media::hash_old_thumbnails(&app.state, 20)
            .await
            .unwrap(),
        0
    );
}

// ---------------------------------------------------------------------------------------------
// What a viewer may see

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_viewers_country_decides_the_under_18_view(db: PgPool) {
    use slop_api::policy::ViewerPolicy;
    let app = test_app(db).await;
    let user = |country: &str| slop_api::auth::session::CurrentUser {
        id: Uuid::now_v7(),
        session_id: Uuid::now_v7(),
        display_name: "Viewer".into(),
        country_code: country.into(),
        suspended: false,
        agent: None,
    };
    let headers = |country: Option<&str>| {
        let mut h = axum::http::HeaderMap::new();
        if let Some(c) = country {
            h.insert("x-country", c.parse().unwrap());
        }
        h
    };
    let resolve = |config: Config, u: Option<slop_api::auth::session::CurrentUser>, h| {
        let db = app.db.clone();
        async move {
            ViewerPolicy::resolve(&db, &config, u.as_ref(), &h)
                .await
                .unwrap()
        }
    };
    let config = app.config.clone();
    // Signed in, the account's country counts, not where the request seems to come from.
    assert_eq!(
        resolve(config.clone(), Some(user("GB")), headers(Some("US"))).await,
        ViewerPolicy::UNDER_18_VIEW
    );
    assert_eq!(
        resolve(config.clone(), Some(user("US")), headers(Some("GB"))).await,
        ViewerPolicy::FULL_VIEW
    );
    // Signed out: the CDN's country header, in any case and with spaces.
    assert_eq!(
        resolve(config.clone(), None, headers(Some(" de "))).await,
        ViewerPolicy::FULL_VIEW
    );
    assert_eq!(
        resolve(config.clone(), None, headers(Some("GB"))).await,
        ViewerPolicy::UNDER_18_VIEW
    );
    // No header, a country Slop has no policy for, or no header configured: the safe view.
    assert_eq!(
        resolve(config.clone(), None, headers(None)).await,
        ViewerPolicy::UNDER_18_VIEW
    );
    assert_eq!(
        resolve(config.clone(), None, headers(Some("ZZ"))).await,
        ViewerPolicy::UNDER_18_VIEW
    );
    let mut no_header = config.clone();
    no_header.country_header = None;
    assert_eq!(
        resolve(no_header, None, headers(Some("US"))).await,
        ViewerPolicy::UNDER_18_VIEW
    );
    assert_eq!(ViewerPolicy::UNDER_18_VIEW.max_min_age, 16);
    assert_eq!(ViewerPolicy::FULL_VIEW.max_min_age, 18);
}

// ---------------------------------------------------------------------------------------------
// The simulation sandbox

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_simulations_sign_in_is_only_there_in_the_simulation(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Sim maker 1", "US").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/sim/users")).await.status(),
        StatusCode::NOT_FOUND
    );
    let res = app
        .send(req(Method::POST, format!("/v1/sim/login/{maker}")))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert!(set_cookie(&res, "slop_session").is_none());
    let sessions: i64 = sqlx::query_scalar("select count(*) from sessions where user_id = $1")
        .bind(maker)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(sessions, 1, "only the one the test made");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_simulation_lets_anyone_view_as_its_own_users_only(db: PgPool) {
    let app = test_app_with(db, |c| c.simulation = true).await;
    slop_api::sim::generate(
        &app.db,
        slop_api::sim::Settings {
            apps: 12,
            users: 40,
            days: 3,
            seed: 5,
        },
    )
    .await
    .unwrap();
    let res = app.send(req(Method::GET, "/v1/sim/users")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let users = body_json(res).await;
    let users = users.as_array().unwrap();
    let mut per_persona: std::collections::HashMap<&str, usize> = Default::default();
    for u in users {
        *per_persona
            .entry(u["personas"].as_str().unwrap())
            .or_default() += 1;
    }
    // Up to three of each persona, and the simulation's makers.
    for (persona, n) in &per_persona {
        if !persona.starts_with("People:") {
            assert!(*n <= 3, "{persona}: {n}");
        }
    }
    assert!(
        per_persona.get("People: maker").copied().unwrap_or(0) > 0,
        "{per_persona:?}"
    );

    // Viewing as one of them: a session for that user.
    let sim_user = users
        .iter()
        .find(|u| !u["personas"].as_str().unwrap().starts_with("People:"))
        .unwrap();
    let id = sim_user["id"].as_str().unwrap();
    let res = app
        .send(req(Method::POST, format!("/v1/sim/login/{id}")))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let cookie = set_cookie(&res, "slop_session").unwrap();
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&cookie)).await).await;
    assert_eq!(me["id"], id);

    // Never a real account, even one that calls itself a simulated maker.
    let (real, _) = app.user("Pat Player", "US").await;
    let (impostor, _) = app.user("Sim maker 999", "US").await;
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id) values ('github', '4242', $1)",
    )
    .bind(impostor)
    .execute(&app.db)
    .await
    .unwrap();
    for user in [real, impostor, Uuid::now_v7()] {
        assert_eq!(
            app.send(req(Method::POST, format!("/v1/sim/login/{user}")))
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{user}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Reversing a delisting on appeal

/// An app taken out of the store, then frozen (Slop lost its repository): reversing the
/// delisting on appeal mustn't put a frozen app back in the store. The appeal is recorded, the
/// maker and staff are told why it stays out, and it goes back once the freeze ends.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reversing_a_delisting_keeps_the_listing_rules(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    app.send(
        req(Method::POST, "/v1/admin/apps/castle/delist")
            .cookie(&owner)
            .json(json!({ "rule": "misleading", "reason": "Fake screenshots." })),
    )
    .await;
    sqlx::query("update apps set frozen_at = now(), frozen_reason = 'Repository gone'")
        .execute(&app.db)
        .await
        .unwrap();
    let decision: Uuid = sqlx::query_scalar("select id from moderation_decisions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let res = appeal_and_reverse(&app, &decision.to_string(), &maker, &owner).await;
    let stays_out = res["stays_out"].as_str().unwrap();
    assert!(
        stays_out.contains("frozen (Repository gone)"),
        "{stays_out}"
    );
    let app_status = || async {
        sqlx::query_scalar::<_, String>("select status::text from apps where slug = 'castle'")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    assert_ne!(
        app_status().await,
        "listed",
        "a frozen app stays out of the store"
    );
    let reversed: bool = sqlx::query_scalar(
        "select appeal_status = 'reversed' and reversed_at is not null from moderation_decisions",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(reversed, "the appeal's outcome is recorded");
    let body: String = sqlx::query_scalar(
        "select body from notifications where user_id = $1 and title like 'Your appeal%'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(body.ends_with(stays_out), "{body}");

    // The freeze ends: it's listed the usual way.
    let id: Uuid = sqlx::query_scalar("select id from apps where slug = 'castle'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    slop_api::frozen::apply(&app.state, id, slop_api::frozen::Access::Readable)
        .await
        .unwrap();
    assert_eq!(app_status().await, "listed");
}

/// Reversed while new pictures wait for review: it stays out and waits under Ready to list,
/// and a reviewer lists it once they're reviewed.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_reinstated_app_with_pictures_waiting_waits_for_a_reviewer(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    approved_release(&app, "castle", maker_id).await;
    app.send(
        req(Method::POST, "/v1/admin/apps/castle/delist")
            .cookie(&owner)
            .json(json!({ "rule": "misleading", "reason": "Fake screenshots." })),
    )
    .await;
    sqlx::query(
        "insert into media_sets (id, app_id, commit_sha, fingerprint, source)
         select $1, id, 'abc123', 'fp', 'maker' from apps where slug = 'castle'",
    )
    .bind(Uuid::now_v7())
    .execute(&app.db)
    .await
    .unwrap();
    let decision: Uuid = sqlx::query_scalar("select id from moderation_decisions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let res = appeal_and_reverse(&app, &decision.to_string(), &maker, &owner).await;
    assert!(
        res["stays_out"]
            .as_str()
            .unwrap()
            .contains("new pictures are reviewed"),
        "{res}"
    );
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(&owner))
            .await,
    )
    .await;
    let ready = &queue["ready_to_list"][0];
    assert_eq!(ready["slug"], "castle");
    assert_eq!(ready["after_appeal"], true);
    assert_eq!(ready["pictures_pending"], true);
    // Listing still waits for the pictures, then goes through.
    let list = || app.send(req(Method::POST, "/v1/admin/apps/castle/list").cookie(&owner));
    assert_eq!(list().await.status(), StatusCode::CONFLICT);
    sqlx::query("update media_sets set status = 'approved'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(list().await.status(), StatusCode::NO_CONTENT);
    let (status, waiting): (String, bool) =
        sqlx::query_as("select status::text, relist_after_appeal from apps where slug = 'castle'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((status.as_str(), waiting), ("listed", false));
}
