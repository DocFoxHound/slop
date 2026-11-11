// SPDX-License-Identifier: AGPL-3.0-or-later
//! Maker news (news.rs, migration 0097, DESIGN §8): who may post and when (one post per app per
//! 24 hours, only while the app's own page is in the store), who is told under the bell and who
//! isn't, that news is never emailed or pushed, that the bell follows the post (edited, removed,
//! out of the store, hidden), and the per-app switch on an Interested mark.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers for this area

const BODY: &str = "Both crashes from last week are fixed. **Controller support** is next.";
/// The refusal for a hidden post while it can be appealed, and once it can't any more.
const HIDDEN: &str =
    "this post was removed by a moderation decision; you can appeal it from your account";
const HIDDEN_DECIDED: &str =
    "this post was removed by a moderation decision; you can see the decision on your account";

async fn mark(app: &TestApp, cookie: &str, slug: &str) {
    let res = app
        .send(req(Method::PUT, format!("/v1/me/interested/{slug}")).cookie(cookie))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT, "marking {slug}");
}

async fn switch_news(app: &TestApp, cookie: &str, slug: &str, on: bool) -> StatusCode {
    app.send(
        req(Method::PUT, format!("/v1/me/interested/{slug}/news"))
            .cookie(cookie)
            .json(json!({ "on": on })),
    )
    .await
    .status()
}

async fn post_news(
    app: &TestApp,
    cookie: &str,
    slug: &str,
    headline: &str,
    body: &str,
) -> Response {
    app.send(
        req(Method::POST, format!("/v1/apps/{slug}/news"))
            .cookie(cookie)
            .json(json!({ "headline": headline, "body": body })),
    )
    .await
}

/// Posts news that must be taken: the post's ID.
async fn posted(app: &TestApp, cookie: &str, slug: &str, headline: &str) -> String {
    let res = post_news(app, cookie, slug, headline, BODY).await;
    assert_eq!(res.status(), StatusCode::CREATED, "posting on {slug}");
    body_json(res).await["id"].as_str().unwrap().to_string()
}

async fn edit_news(
    app: &TestApp,
    cookie: &str,
    slug: &str,
    id: &str,
    headline: &str,
    body: &str,
) -> Response {
    app.send(
        req(Method::PUT, format!("/v1/apps/{slug}/news/{id}"))
            .cookie(cookie)
            .json(json!({ "headline": headline, "body": body })),
    )
    .await
}

async fn remove_news(app: &TestApp, cookie: &str, slug: &str, id: &str) -> StatusCode {
    app.send(req(Method::DELETE, format!("/v1/apps/{slug}/news/{id}")).cookie(cookie))
        .await
        .status()
}

/// The app's news page for this caller (signed out with "").
async fn news_page(app: &TestApp, cookie: &str, slug: &str, query: &str) -> Value {
    let mut r = req(Method::GET, format!("/v1/apps/{slug}/news{query}"));
    if !cookie.is_empty() {
        r = r.cookie(cookie);
    }
    let res = app.send(r).await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

/// `user`'s news notices, oldest first: (title, body, count, read).
async fn news_notices(app: &TestApp, user: Uuid) -> Vec<(String, String, i32, bool)> {
    sqlx::query_as(
        "select title, body, count, read_at is not null from notifications
         where user_id = $1 and category = 'news' order by created_at, id",
    )
    .bind(user)
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// As if the app's last post was 25 hours ago, so the next one is allowed.
async fn a_day_later(app: &TestApp, slug: &str) {
    sqlx::query("update apps set news_posted_at = now() - interval '25 hours' where slug = $1")
        .bind(slug)
        .execute(&app.db)
        .await
        .unwrap();
}

async fn exec(app: &TestApp, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(&app.db)
        .await
        .unwrap();
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// The error's code and message.
async fn error(res: Response) -> (String, String) {
    let body = body_json(res).await;
    (
        body["error"]["code"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    )
}

/// The inbox the way the bell asks for it (`bell` = true) or the way a service worker does.
async fn inbox(app: &TestApp, cookie: &str, query: &str) -> Value {
    let res = app
        .send(req(Method::GET, format!("/v1/me/notifications{query}")).cookie(cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{query}");
    body_json(res).await
}

fn categories(inbox: &Value) -> Vec<String> {
    let mut c: Vec<String> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["category"].as_str().unwrap().to_string())
        .collect();
    c.sort();
    c
}

/// A report of `kind` about `subject`, signed in as `cookie`.
async fn report(app: &TestApp, cookie: &str, kind: &str, subject: &str, reason: &str) -> Response {
    app.send(req(Method::POST, "/v1/reports").cookie(cookie).json(json!({
        "kind": kind, "subject": subject, "reason": reason,
        "details": "It offers a prize for votes.", "good_faith": true
    })))
    .await
}

/// Staff act on the newest open report with `action` under `rule`. Returns the decision's ID.
async fn act(app: &TestApp, admin: &str, action: &str, rule: &str) -> String {
    let id: Uuid = sqlx::query_scalar(
        "select id from reports where status = 'open' order by created_at desc limit 1",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/reports/{id}/act"))
                .cookie(admin)
                .json(json!({
                    "action": action, "rule": rule,
                    "facts": "It offers a prize for Slaps.", "outcome": "We hid it."
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{action}");
    body_json(res).await["decision"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A news post reported by `reporter` and hidden by staff under "Spam". Returns the decision's ID.
async fn hide_by_decision(app: &TestApp, admin: &str, reporter: &str, post: &str) -> String {
    let res = report(app, reporter, "news", post, "spam").await;
    assert_eq!(res.status(), StatusCode::OK, "reporting {post}");
    act(app, admin, "hide_news", "spam").await
}

/// The admin queue, as staff see it.
async fn queue(app: &TestApp, admin: &str) -> Value {
    let res = app
        .send(req(Method::GET, "/v1/admin/queue").cookie(admin))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

/// The maker appeals a decision.
async fn appeal(app: &TestApp, maker: &str, decision: &str) {
    let res = app
        .send(
            req(Method::POST, format!("/v1/me/decisions/{decision}/appeal"))
                .cookie(maker)
                .json(json!({ "appeal": "Please look again." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "appealing {decision}");
}

/// Staff answer an appeal: reversing the decision, or keeping it.
async fn answer_appeal(app: &TestApp, admin: &str, decision: &str, reverse: bool) {
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/decisions/{decision}/appeal"),
            )
            .cookie(admin)
            .json(json!({ "reverse": reverse, "reason": "We looked again." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "answering {decision}");
}

/// A terrorist content removal order naming the post. Returns the decision's ID.
async fn removal_order(app: &TestApp, admin: &str, post: &str) -> String {
    let res = app
        .send(
            req(Method::POST, "/v1/admin/removal-orders")
                .cookie(admin)
                .json(json!({
                    "received_at": (OffsetDateTime::now_utc() - time::Duration::minutes(5)).format(&Rfc3339).unwrap(),
                    "country": "FR", "authority": "OCLCTIC", "order": "Order 2026-118",
                    "subject_kind": "news", "subject": post
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "removal order for {post}");
    sqlx::query_scalar::<_, Uuid>(
        "select id from moderation_decisions where subject_id = $1::uuid and rule = 'illegal'",
    )
    .bind(post)
    .fetch_one(&app.db)
    .await
    .unwrap()
    .to_string()
}

/// This connection's server process, to see who waits for its locks.
async fn backend_pid(conn: &mut sqlx::PgConnection) -> i32 {
    sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(conn)
        .await
        .unwrap()
}

/// Waits until another connection waits for a lock that connection `pid` holds.
async fn until_blocked_by(app: &TestApp, pid: i32) {
    for _ in 0..1000 {
        let waiting: bool = sqlx::query_scalar(
            "select exists (select 1 from pg_stat_activity where $1 = any(pg_blocking_pids(pid)))",
        )
        .bind(pid)
        .fetch_one(&app.db)
        .await
        .unwrap();
        if waiting {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("nothing waited for connection {pid}");
}

// ---------------------------------------------------------------------------------------------
// Posting and who is told

/// The maker posts on a Coming soon app; only people with a current mark and news on are told,
/// under the bell, with SLAPorSLOP's fixed title and the maker's headline. Everyone reads it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_post_news_and_interested_people_are_told(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    let (quiet_id, quiet) = app.user("Quinn Quiet", "US").await;
    let (owner_id, _) = app.user("Lee Library", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    mark(&app, &quiet, "soon").await;
    assert_eq!(
        switch_news(&app, &quiet, "soon", false).await,
        StatusCode::NO_CONTENT
    );
    // Only in the library: not told.
    sqlx::query(
        "insert into library_entries (user_id, app_id) select $1, id from apps where slug = 'soon'",
    )
    .bind(owner_id)
    .execute(&app.db)
    .await
    .unwrap();

    let res = post_news(&app, &maker, "soon", "  Beta opens on Friday  ", BODY).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let post = body_json(res).await;
    assert_eq!(post["headline"], "Beta opens on Friday", "trimmed");
    assert_eq!(post["body"], BODY);
    assert_eq!(post["edited"], false);
    assert_eq!(post["hidden"], false);
    assert!(post.get("hidden_reason").is_none(), "{post}");
    assert!(post["created_at"].as_str().unwrap().ends_with('Z'));
    let id = post["id"].as_str().unwrap();

    let (title, body, link, collapse_key, email_status, pushed): (
        String,
        String,
        String,
        Option<String>,
        String,
        bool,
    ) = sqlx::query_as(
        "select title, body, link, collapse_key, email_status, pushed_at is not null
         from notifications where user_id = $1 and category = 'news'",
    )
    .bind(fan_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(title, "soon: news from its maker");
    assert_eq!(body, "Beta opens on Friday");
    assert_eq!(link, "/app/soon#news");
    assert!(collapse_key.unwrap().starts_with("news:"));
    assert_eq!(email_status, "skipped");
    assert!(pushed, "written as already pushed");
    for (who, user) in [
        ("news off", quiet_id),
        ("library only", owner_id),
        ("maker", maker_id),
    ] {
        assert!(news_notices(&app, user).await.is_empty(), "{who}");
    }
    // The bell shows it.
    let bell = inbox(&app, &fan, "?bell=true").await;
    assert_eq!(bell["items"][0]["category"], "news");
    assert_eq!(bell["items"][0]["title"], "soon: news from its maker");

    // Signed out, the post is on the page; only its maker gets the posting state.
    let page = news_page(&app, "", "soon", "").await;
    assert_eq!(page["news"][0]["id"], id);
    assert_eq!(page["news"][0]["headline"], "Beta opens on Friday");
    assert_eq!(page["more"], false);
    assert!(page.get("posting").is_none(), "{page}");
    let page = news_page(&app, &fan, "soon", "").await;
    assert!(page.get("posting").is_none(), "{page}");
    let page = news_page(&app, &maker, "soon", "").await;
    assert_eq!(page["posting"]["allowed"], false);
    assert_eq!(page["posting"]["why_not"], "wait");
    assert!(page["posting"]["next_post_at"].as_str().is_some(), "{page}");

    // The audit entry: who was told, by whom, and no text.
    let detail: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'news.post' and subject = $1",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        detail,
        json!({ "app": "soon", "told": 1, "by_agent": false })
    );

    // Before posting there's nothing to wait for.
    listed_with(&app, maker_id, "fresh", "beta", "").await;
    let page = news_page(&app, &maker, "fresh", "").await;
    assert_eq!(
        page["posting"],
        json!({ "allowed": true, "why_not": null, "next_post_at": null })
    );
}

/// Suspended and closed accounts, the maker, and people whose age view no longer includes the app
/// are never told.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_respects_who_may_be_told(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (us_id, _) = app.user("Uma US", "US").await;
    let (gb_id, _) = app.user("Gus GB", "GB").await;
    let (suspended_id, _) = app.user("Sam Suspended", "US").await;
    let (closed_id, _) = app.user("Cleo Closed", "US").await;
    listed_with(&app, maker_id, "grown", "coming_soon", "min_age = 18").await;
    // Marks made before the app's rating went up (or the person moved), straight in the database.
    for user in [us_id, gb_id, suspended_id, closed_id, maker_id] {
        sqlx::query(
            "insert into interests (user_id, app_id) select $1, id from apps where slug = 'grown'",
        )
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
    }
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(suspended_id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(closed_id)
        .execute(&app.db)
        .await
        .unwrap();

    let id = posted(&app, &maker, "grown", "Five new levels are in").await;
    assert_eq!(news_notices(&app, us_id).await.len(), 1, "full view: told");
    for (who, user) in [
        ("under-18 view", gb_id),
        ("suspended", suspended_id),
        ("closed", closed_id),
        ("maker", maker_id),
    ] {
        assert!(news_notices(&app, user).await.is_empty(), "{who}");
    }
    let told: Value = sqlx::query_scalar(
        "select detail -> 'told' from audit_log where action = 'news.post' and subject = $1",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 1);
}

/// One post per app per 24 hours: a second is a 429 that says when, removing a post doesn't give
/// the day back, and two posts sent at once give exactly one.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn one_news_post_per_app_per_day(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "daily", "beta", "").await;

    posted(&app, &maker, "daily", "First news").await;
    let res = post_news(&app, &maker, "daily", "Second news", BODY).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = res.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(retry > 23 * 3600 && retry <= 24 * 3600, "{retry}");
    let (code, message) = error(res).await;
    assert_eq!(code, "rate_limited");
    assert!(
        message
            .starts_with("This app posted news in the last 24 hours. The next post is allowed at "),
        "{message}"
    );
    assert!(message.ends_with(")."), "{message}");

    a_day_later(&app, "daily").await;
    let id = posted(&app, &maker, "daily", "Second news").await;
    let before: OffsetDateTime =
        sqlx::query_scalar("select news_posted_at from apps where slug = 'daily'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        remove_news(&app, &maker, "daily", &id).await,
        StatusCode::NO_CONTENT
    );
    let after: OffsetDateTime =
        sqlx::query_scalar("select news_posted_at from apps where slug = 'daily'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(before, after, "removing doesn't give the day back");
    let res = post_news(&app, &maker, "daily", "Third news", BODY).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);

    // Two at once: the row lock lets exactly one through.
    a_day_later(&app, "daily").await;
    let (a, b) = tokio::join!(
        post_news(&app, &maker, "daily", "Race one", BODY),
        post_news(&app, &maker, "daily", "Race two", BODY)
    );
    let mut statuses = [a.status(), b.status()];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::CREATED, StatusCode::TOO_MANY_REQUESTS]
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from app_news n join apps a on a.id = n.app_id where a.slug = 'daily'"
        )
        .await,
        2
    );
}

/// Posting and editing also count against the account's news allowance (10 an hour): past it both
/// are refused and change nothing. Removing a post has no allowance.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_posts_and_edits_have_a_per_account_limit(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    listed_with(&app, maker_id, "later", "coming_soon", "").await;
    mark(&app, &fan, "later").await;
    let id = posted(&app, &maker, "soon", "Beta opens on Friday").await;
    let now = std::time::Instant::now();
    while app
        .state
        .account_limiter
        .take(maker_id, slop_api::limits::Action::News, now)
        .is_ok()
    {}

    let too_many = (
        "rate_limited".to_string(),
        "too many news posts for now: try again in a while".to_string(),
    );
    let res = post_news(&app, &maker, "later", "Five new levels are in", BODY).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error(res).await, too_many, "posting");
    let res = edit_news(&app, &maker, "soon", &id, "Beta opens on Saturday", BODY).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error(res).await, too_many, "editing");
    let left: Vec<(String, String)> =
        sqlx::query_as("select a.slug, n.headline from app_news n join apps a on a.id = n.app_id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        left,
        [("soon".to_string(), "Beta opens on Friday".to_string())]
    );
    assert!(news_notices(&app, fan_id).await.is_empty(), "nobody told");
    let slot: Option<OffsetDateTime> =
        sqlx::query_scalar("select news_posted_at from apps where slug = 'later'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(slot, None, "the day's post is still there");

    assert_eq!(
        remove_news(&app, &maker, "soon", &id).await,
        StatusCode::NO_CONTENT
    );
}

/// A delisting under way when the maker posts: the post waits for it, then sees the page is out
/// of the store, so nothing is posted and nobody is told.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_post_waits_for_a_delisting_under_way(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;

    // Staff take it out of the store; the change isn't committed yet when the post arrives.
    let mut staff = app.db.begin().await.unwrap();
    let pid = backend_pid(&mut staff).await;
    sqlx::query("update apps set status = 'delisted', updated_at = now() where slug = 'soon'")
        .execute(&mut *staff)
        .await
        .unwrap();
    let (res, ()) = tokio::join!(
        post_news(&app, &maker, "soon", "Beta opens on Friday", BODY),
        async {
            until_blocked_by(&app, pid).await;
            staff.commit().await.unwrap();
        }
    );
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        error(res).await.1,
        "news can be posted or changed only while the app's page is in the store"
    );
    assert_eq!(count(&app, "select count(*) from app_news").await, 0);
    assert!(news_notices(&app, fan_id).await.is_empty());
    let slot: Option<OffsetDateTime> =
        sqlx::query_scalar("select news_posted_at from apps where slug = 'soon'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(slot, None, "the refusal gives the slot back");
}

/// Only while the app's own page is in the store: not a draft, a private app, a delisted or
/// frozen one, one past its stage deadline, or a dedicated server. Any stage and an 18+ app can.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_needs_a_page_in_the_store(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, other) = app.user("Otto Other", "US").await;
    listed_with(&app, maker_id, "game", "released", "").await;
    let game: Uuid = sqlx::query_scalar("select id from apps where slug = 'game'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    for (slug, set) in [
        ("draft", "status = 'draft'".to_string()),
        ("private", "visibility = 'private'".to_string()),
        ("delisted", "status = 'delisted'".to_string()),
        ("frozen", "frozen_at = now()".to_string()),
        (
            "late",
            "stage_entered_at = now() - interval '400 days'".to_string(),
        ),
        (
            "server",
            format!("branch_kind = 'server', parent_app_id = '{game}', platforms = '{{linux}}'"),
        ),
    ] {
        listed_with(&app, maker_id, slug, "coming_soon", &set).await;
        let res = post_news(&app, &maker, slug, "News", BODY).await;
        assert_eq!(res.status(), StatusCode::CONFLICT, "{slug}");
        assert_eq!(
            error(res).await,
            (
                "conflict".into(),
                "news can be posted or changed only while the app's page is in the store".into()
            ),
            "{slug}"
        );
    }
    // The maker still sees why, on an app only they can see.
    let page = news_page(&app, &maker, "delisted", "").await;
    assert_eq!(
        page["posting"],
        json!({ "allowed": false, "why_not": "not_in_store", "next_post_at": null })
    );
    // Not the maker: as if there were no such app. Signed out: sign in.
    let res = post_news(&app, &other, "game", "News", BODY).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/news")
                .json(json!({ "headline": "News", "body": BODY })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/draft/news"))
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "nobody else reads a draft's news"
    );

    // A released app and an 18+ app can.
    posted(&app, &maker, "game", "News").await;
    listed_with(&app, maker_id, "grown", "beta", "min_age = 18").await;
    posted(&app, &maker, "grown", "News").await;
}

/// What the maker types is checked: a headline on one line of 1 to 100 characters, a text of 1 to
/// 2,000, and nothing hidden in either. Each refusal says what to fix.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_text_is_checked(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "words", "coming_soon", "").await;
    let one_line = "1 to 100 characters, on one line";
    let hidden = "one line without tabs or hidden characters";
    let long = "h".repeat(101);
    for (headline, body, field, message) in [
        ("   ", BODY.to_string(), "headline", one_line),
        (long.as_str(), BODY.to_string(), "headline", one_line),
        ("Two\nlines", BODY.to_string(), "headline", one_line),
        ("A\ttab", BODY.to_string(), "headline", hidden),
        // Short and on one line, but it reads backwards from the mark on.
        (
            "Beta \u{202e}yadirf no",
            BODY.to_string(),
            "headline",
            hidden,
        ),
        ("News", " \n ".to_string(), "body", "1 to 2,000 characters"),
        ("News", "b".repeat(2001), "body", "1 to 2,000 characters"),
        (
            "News",
            "A bell\u{7}".to_string(),
            "body",
            "without hidden control characters",
        ),
    ] {
        let res = post_news(&app, &maker, "words", headline, &body).await;
        assert_eq!(
            res.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{headline:?}"
        );
        let err = body_json(res).await;
        assert_eq!(err["error"]["code"], "invalid");
        assert_eq!(err["error"]["field"], field, "{headline:?}");
        assert_eq!(err["error"]["message"], message, "{headline:?}");
    }
    let res = app
        .send(
            req(Method::POST, "/v1/apps/words/news")
                .cookie(&maker)
                .json(json!({ "headline": "News", "body": BODY, "pinned": true })),
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown fields are refused"
    );
    assert_eq!(
        count(&app, "select count(*) from app_news").await,
        0,
        "nothing was posted"
    );

    // The longest allowed, with line breaks and a tab in the text.
    let headline = "h".repeat(100);
    let body = format!("{}\n\n- one\n- two\tand three", "b".repeat(1970));
    assert_eq!(body.chars().count(), 1993);
    let res = post_news(&app, &maker, "words", &headline, &body).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(body_json(res).await["body"], body);
}

// ---------------------------------------------------------------------------------------------
// Never emailed, never pushed

/// News is written as skipped for email and as already pushed, and both workers skip the kind
/// even when a notice says otherwise; other kinds still go out in the same setup.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_is_never_emailed_or_pushed(db: PgPool) {
    let (key, _) = slop_api::push::generate();
    let app = test_app_with(db, |c| {
        c.email = Some(slop_api::config::EmailConfig {
            smtp_url: "smtp://localhost:2525".into(),
            from: "SLAPorSLOP <n@slop.test>".into(),
        });
        c.push = Some(slop_api::config::PushConfig {
            key: slop_api::push::parse_key(&key).unwrap(),
            subject: "mailto:ops@slop.example".into(),
            extra_hosts: vec!["127.0.0.1".into()],
            allow_http: true,
        });
    })
    .await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    sqlx::query("update users set email = 'fay@example.com' where id = $1")
        .bind(fan_id)
        .execute(&app.db)
        .await
        .unwrap();
    // A push service that answers "gone": nothing reaches the shared recorder of other tests.
    let res = app
        .send(req(Method::POST, "/v1/me/push").cookie(&fan).json(json!({
            "endpoint": format!("{}push/gone", app.fake_base), "keys": { "p256dh": "x", "auth": "y" }
        })))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    posted(&app, &maker, "soon", "Beta opens on Friday").await;

    assert_eq!(
        slop_api::notify::queue_emails(&app.db, &app.config)
            .await
            .unwrap(),
        0
    );
    assert_eq!(slop_api::push::run_once(&app.state).await.unwrap(), 0);
    assert_eq!(count(&app, "select count(*) from email_outbox").await, 0);

    // A news notice that says it's waiting for both: the workers' own guard still holds.
    let hand = Uuid::now_v7();
    sqlx::query(
        "insert into notifications (id, user_id, category, title, body, link, email_status, pushed_at)
         values ($1, $2, 'news', 'soon: news from its maker', 'By hand', '/app/soon#news', 'pending', null)",
    )
    .bind(hand)
    .bind(fan_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        slop_api::notify::queue_emails(&app.db, &app.config)
            .await
            .unwrap(),
        0
    );
    assert_eq!(slop_api::push::run_once(&app.state).await.unwrap(), 0);
    let (email_status, pushed): (String, bool) = sqlx::query_as(
        "select email_status, pushed_at is not null from notifications where id = $1",
    )
    .bind(hand)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(email_status, "skipped");
    assert!(!pushed, "never picked for a push");
    assert_eq!(count(&app, "select count(*) from email_outbox").await, 0);

    // Another kind in the same setup is emailed and picked for a push.
    let job = Uuid::now_v7();
    sqlx::query(
        "insert into notifications (id, user_id, category, title, link) values ($1, $2, 'jobs', 'A new job request', '/people')",
    )
    .bind(job)
    .bind(fan_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        slop_api::notify::queue_emails(&app.db, &app.config)
            .await
            .unwrap(),
        1
    );
    let subjects: Vec<String> = sqlx::query_scalar("select subject from email_outbox")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(subjects, ["A new job request"]);
    slop_api::push::run_once(&app.state).await.unwrap();
    let pushed: Vec<String> = sqlx::query_scalar(
        "select category from notifications where user_id = $1 and pushed_at is not null order by category",
    )
    .bind(fan_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        pushed,
        ["jobs", "news"],
        "the posted news only as written, never by the worker"
    );
}

/// News has no email switch on the account page, and the bell can still ask for the kind.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_has_no_email_switch(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    posted(&app, &maker, "soon", "Beta opens on Friday").await;

    let prefs = body_json(
        app.send(req(Method::GET, "/v1/me/email-prefs").cookie(&fan))
            .await,
    )
    .await;
    let kinds: Vec<&str> = prefs["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["category"].as_str().unwrap())
        .collect();
    assert!(!kinds.contains(&"news"), "{kinds:?}");
    assert_eq!(prefs["categories"][1]["category"], "interested");
    let res = app
        .send(
            req(Method::PUT, "/v1/me/email-prefs")
                .cookie(&fan)
                .json(json!({ "news": false })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body_json(res).await["error"]["message"],
        "\"news\" isn't a kind of email"
    );

    let bell = inbox(&app, &fan, "?bell=true&category=news").await;
    assert_eq!(categories(&bell), ["news"]);
    let res = app
        .send(req(Method::GET, "/v1/me/notifications?category=nonsense").cookie(&fan))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// Without `bell=true` (a service worker sends no query) the inbox never holds news, in its items
/// or its counts; the bell itself gets both. The badge count keeps news, as the bell shows it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_inbox_without_bell_never_returns_news(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    posted(&app, &maker, "soon", "Beta opens on Friday").await;
    sqlx::query(
        "insert into notifications (id, user_id, category, title, link) values ($1, $2, 'jobs', 'A new job request', '/people')",
    )
    .bind(Uuid::now_v7())
    .bind(fan_id)
    .execute(&app.db)
    .await
    .unwrap();

    for query in ["", "?bell=false", "?category=news", "?unread=true"] {
        let worker = inbox(&app, &fan, query).await;
        assert!(
            !categories(&worker).contains(&"news".to_string()),
            "{query}: {worker}"
        );
        assert!(worker["counts"].get("news").is_none(), "{query}: {worker}");
    }
    let worker = inbox(&app, &fan, "").await;
    assert_eq!(categories(&worker), ["jobs"]);
    assert_eq!(worker["unread"], 1);
    assert_eq!(worker["counts"], json!({ "jobs": 1 }));

    let bell = inbox(&app, &fan, "?bell=true").await;
    assert_eq!(categories(&bell), ["jobs", "news"]);
    assert_eq!(bell["unread"], 2);
    assert_eq!(bell["counts"], json!({ "jobs": 1, "news": 1 }));
    let me = body_json(app.send(req(Method::GET, "/v1/me").cookie(&fan)).await).await;
    assert_eq!(me["unread_notifications"], 2);
}

// ---------------------------------------------------------------------------------------------
// The bell follows the post

/// Editing changes the post and the headline the bell shows, and tells nobody again. `edited`
/// shows only for a change made over a minute after posting.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn editing_news_tells_nobody_again(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    let (other_id, other) = app.user("Otto Other", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    listed_with(&app, other_id, "theirs", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    let id = posted(&app, &maker, "soon", "Beta opens on Friday").await;
    let posted_at: OffsetDateTime =
        sqlx::query_scalar("select news_posted_at from apps where slug = 'soon'")
            .fetch_one(&app.db)
            .await
            .unwrap();

    // As if it was posted two minutes ago.
    exec(
        &app,
        "update app_news set created_at = created_at - interval '2 minutes', updated_at = updated_at - interval '2 minutes'",
    )
    .await;
    let res = edit_news(&app, &maker, "soon", &id, "Beta opens on Friday", BODY).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_json(res).await["edited"],
        false,
        "the same text isn't an edit"
    );
    let res = edit_news(
        &app,
        &maker,
        "soon",
        &id,
        "Beta opens on Saturday",
        "New text.",
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let post = body_json(res).await;
    assert_eq!(post["headline"], "Beta opens on Saturday");
    assert_eq!(post["body"], "New text.");
    assert_eq!(post["edited"], true);
    assert_eq!(
        news_page(&app, "", "soon", "").await["news"][0]["edited"],
        true
    );
    assert_eq!(
        news_notices(&app, fan_id).await,
        [(
            "soon: news from its maker".to_string(),
            "Beta opens on Saturday".to_string(),
            1,
            false
        )],
        "one notice, re-worded"
    );
    // A change to the text alone leaves the bell as it is: the notice isn't written again (a
    // mark put on it here survives).
    exec(
        &app,
        "update notifications set body = 'Left as it was' where category = 'news'",
    )
    .await;
    let res = edit_news(
        &app,
        &maker,
        "soon",
        &id,
        "Beta opens on Saturday",
        "Newer text.",
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(news_notices(&app, fan_id).await[0].1, "Left as it was");
    let now_at: OffsetDateTime =
        sqlx::query_scalar("select news_posted_at from apps where slug = 'soon'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(posted_at, now_at, "editing isn't a new post");
    let edits = count(
        &app,
        "select count(*) from audit_log where action = 'news.edit'",
    )
    .await;
    assert_eq!(edits, 3);

    // Someone else's post, or an app that isn't yours: not found.
    let theirs = posted(&app, &other, "theirs", "Theirs").await;
    for (slug, post) in [("soon", theirs.as_str()), ("theirs", theirs.as_str())] {
        let res = edit_news(&app, &maker, slug, post, "Mine now", BODY).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{slug}");
    }
    // Out of the store: can't be changed (it can still be removed).
    exec(
        &app,
        "update apps set status = 'delisted' where slug = 'soon'",
    )
    .await;
    let res = edit_news(&app, &maker, "soon", &id, "Gone", BODY).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        error(res).await.1,
        "news can be posted or changed only while the app's page is in the store"
    );
}

/// Removing a post takes it off the page and out of the bell, whatever the app's state now.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn removing_news_takes_it_out_of_the_bell(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    let (other_id, other) = app.user("Otto Other", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    listed_with(&app, maker_id, "later", "coming_soon", "").await;
    listed_with(&app, other_id, "theirs", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    mark(&app, &fan, "later").await;
    let id = posted(&app, &maker, "soon", "Beta opens on Friday").await;
    let theirs = posted(&app, &other, "theirs", "Theirs").await;
    assert_eq!(news_notices(&app, fan_id).await.len(), 1);

    assert_eq!(
        remove_news(&app, &maker, "soon", &theirs).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        remove_news(&app, &maker, "theirs", &theirs).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        remove_news(&app, &maker, "soon", &id).await,
        StatusCode::NO_CONTENT
    );
    assert!(news_notices(&app, fan_id).await.is_empty());
    assert_eq!(news_page(&app, "", "soon", "").await["news"], json!([]));
    assert_eq!(
        remove_news(&app, &maker, "soon", &id).await,
        StatusCode::NOT_FOUND
    );
    let detail: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'news.remove' and subject = $1",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(detail, json!({ "app": "soon", "by_agent": false }));

    // Out of the store, the maker can still take their words down.
    let later = posted(&app, &maker, "later", "Later news").await;
    exec(
        &app,
        "update apps set status = 'delisted' where slug = 'later'",
    )
    .await;
    assert_eq!(
        remove_news(&app, &maker, "later", &later).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        count(&app, "select count(*) from app_news").await,
        1,
        "only theirs"
    );
}

/// Unread news from one app is one notice with a count and the newest headline; once it's read,
/// the next post makes a new one.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn merged_unread_news_is_one_notice(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    posted(&app, &maker, "soon", "First news").await;
    a_day_later(&app, "soon").await;
    posted(&app, &maker, "soon", "Second news").await;
    let title = "soon: news from its maker".to_string();
    assert_eq!(
        news_notices(&app, fan_id).await,
        [(title.clone(), "Second news".to_string(), 2, false)]
    );

    let res = app
        .send(
            req(Method::POST, "/v1/me/notifications/read")
                .cookie(&fan)
                .json(json!({})),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    a_day_later(&app, "soon").await;
    posted(&app, &maker, "soon", "Third news").await;
    assert_eq!(
        news_notices(&app, fan_id).await,
        [
            (title.clone(), "Second news".to_string(), 2, true),
            (title, "Third news".to_string(), 1, false)
        ]
    );
}

/// A notice marked unread again is cut off from the app's thread (its collapse key goes), but it
/// still goes with its own post: when the maker removes the post, and when staff hide it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_notice_marked_unread_again_still_goes_with_its_post(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    // The fan reads the notice of the first post, the second arrives, and the fan marks the
    // first unread again. Returns the first post's ID.
    let read_then_unread = |slug: &'static str, first: &'static str, second: &'static str| {
        let (app, maker, fan) = (&app, &maker, &fan);
        async move {
            listed_with(app, maker_id, slug, "coming_soon", "").await;
            mark(app, fan, slug).await;
            let a = posted(app, maker, slug, first).await;
            let notice_a: Uuid = sqlx::query_scalar(
                "select id from notifications where user_id = $1 and category = 'news' and body = $2",
            )
            .bind(fan_id)
            .bind(first)
            .fetch_one(&app.db)
            .await
            .unwrap();
            let res = app
                .send(
                    req(Method::POST, "/v1/me/notifications/read")
                        .cookie(fan)
                        .json(json!({ "ids": [notice_a] })),
                )
                .await;
            assert_eq!(res.status(), StatusCode::NO_CONTENT);
            a_day_later(app, slug).await;
            posted(app, maker, slug, second).await;
            let res = app
                .send(
                    req(Method::POST, "/v1/me/notifications/unread")
                        .cookie(fan)
                        .json(json!({ "ids": [notice_a] })),
                )
                .await;
            assert_eq!(res.status(), StatusCode::NO_CONTENT);
            let key: Option<String> =
                sqlx::query_scalar("select collapse_key from notifications where id = $1")
                    .bind(notice_a)
                    .fetch_one(&app.db)
                    .await
                    .unwrap();
            assert_eq!(key, None, "the second post's unread notice has the key");
            a
        }
    };
    let bodies = || async {
        let mut b: Vec<String> = news_notices(&app, fan_id)
            .await
            .into_iter()
            .map(|(_, body, _, _)| body)
            .collect();
        b.sort();
        b
    };

    // The maker removes it.
    let a = read_then_unread("soon", "Post A", "Post B").await;
    assert_eq!(
        remove_news(&app, &maker, "soon", &a).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(bodies().await, ["Post B"]);

    // Staff hide it.
    let c = read_then_unread("later", "Post C", "Post D").await;
    hide_by_decision(&app, &admin, &rita, &c).await;
    assert_eq!(bodies().await, ["Post B", "Post D"]);
}

/// The hourly sweep takes news out of the bell when the app's page left the store some other way
/// (a stage deadline passed, the maker made it private), and leaves a listed app's news alone.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_leaves_the_bell_with_its_app(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    for slug in ["kept", "late", "private"] {
        listed_with(&app, maker_id, slug, "coming_soon", "").await;
        mark(&app, &fan, slug).await;
        posted(&app, &maker, slug, &format!("News from {slug}")).await;
    }
    assert_eq!(slop_api::news::cleanup(&app.state).await.unwrap(), (0, 0));
    assert_eq!(news_notices(&app, fan_id).await.len(), 3);

    exec(
        &app,
        "update apps set stage_entered_at = now() - interval '400 days' where slug = 'late'",
    )
    .await;
    exec(
        &app,
        "update apps set visibility = 'private' where slug = 'private'",
    )
    .await;
    assert_eq!(slop_api::news::cleanup(&app.state).await.unwrap(), (2, 0));
    let left: Vec<String> = news_notices(&app, fan_id)
        .await
        .into_iter()
        .map(|(_, body, _, _)| body)
        .collect();
    assert_eq!(left, ["News from kept"]);
    assert_eq!(
        slop_api::news::cleanup(&app.state).await.unwrap(),
        (0, 0),
        "idempotent"
    );
    // The posts themselves stay with their apps.
    assert_eq!(count(&app, "select count(*) from app_news").await, 3);
}

/// Staff taking an app out of the store by a decision, and the end of a frozen app's 14 days, take
/// its news out of the bell at once, without waiting for the hourly sweep.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_leaves_the_bell_at_once_when_staff_delist_or_a_freeze_runs_out(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    for slug in ["kept", "reported", "frozen"] {
        listed_with(&app, maker_id, slug, "coming_soon", "").await;
        mark(&app, &fan, slug).await;
        posted(&app, &maker, slug, &format!("News from {slug}")).await;
    }
    let bodies = || async {
        let mut b: Vec<String> = news_notices(&app, fan_id)
            .await
            .into_iter()
            .map(|(_, body, _, _)| body)
            .collect();
        b.sort();
        b
    };

    let res = report(&app, &rita, "app", "reported", "scam").await;
    assert_eq!(res.status(), StatusCode::OK);
    act(&app, &admin, "delist_app", "scam").await;
    assert_eq!(bodies().await, ["News from frozen", "News from kept"]);

    exec(
        &app,
        "update apps set frozen_at = now() - interval '15 days' where slug = 'frozen'",
    )
    .await;
    assert_eq!(slop_api::frozen::end_grace(&app.state).await.unwrap(), 1);
    assert_eq!(bodies().await, ["News from kept"]);
    // Nothing was left for the sweep, and the posts stay with their apps.
    assert_eq!(slop_api::news::cleanup(&app.state).await.unwrap(), (0, 0));
    assert_eq!(count(&app, "select count(*) from app_news").await, 3);
}

/// The hourly sweep also takes a post's notice from someone who may no longer see the app: its
/// age rating went up, or their account's country now has the under-18 view.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_leaves_the_bell_of_people_who_may_no_longer_see_the_app(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (us_id, us) = app.user("Uma US", "US").await;
    let (gb_id, gb) = app.user("Gus GB", "GB").await;
    let (moved_id, moved) = app.user("Mo Moved", "US").await;
    listed_with(&app, maker_id, "grows", "coming_soon", "").await;
    for fan in [&us, &gb, &moved] {
        mark(&app, fan, "grows").await;
    }
    posted(&app, &maker, "grows", "Five new levels are in").await;
    assert_eq!(slop_api::news::cleanup(&app.state).await.unwrap(), (0, 0));

    exec(&app, "update apps set min_age = 18 where slug = 'grows'").await;
    assert_eq!(slop_api::news::cleanup(&app.state).await.unwrap(), (1, 0));
    assert!(news_notices(&app, gb_id).await.is_empty(), "under-18 view");
    sqlx::query("update users set country_code = 'GB' where id = $1")
        .bind(moved_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(slop_api::news::cleanup(&app.state).await.unwrap(), (1, 0));
    assert!(news_notices(&app, moved_id).await.is_empty(), "moved");
    assert_eq!(news_notices(&app, us_id).await.len(), 1, "full view: kept");
}

/// A Private app has no store page and no news: the people it's shared with read none, its maker
/// still has the posts it made while it was public.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_private_app_shows_its_news_only_to_its_maker(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (friend_id, friend) = app.user("Fran Friend", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    let id = posted(&app, &maker, "soon", "Beta opens on Friday").await;
    // Private, with an approved release: the people it's shared with may open it.
    exec(
        &app,
        "update apps set visibility = 'private' where slug = 'soon'",
    )
    .await;
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by)
         select $1, a.id, '1.0.0', repeat('a', 40), 'approved', a.maker_id from apps a where a.slug = 'soon'",
    )
    .bind(Uuid::now_v7())
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into app_shares (app_id, user_id) select id, $1 from apps where slug = 'soon'",
    )
    .bind(friend_id)
    .execute(&app.db)
    .await
    .unwrap();

    assert_eq!(
        news_page(&app, &friend, "soon", "").await,
        json!({ "news": [], "more": false })
    );
    let mine = news_page(&app, &maker, "soon", "").await;
    assert_eq!(mine["news"][0]["id"], id);
    assert_eq!(mine["posting"]["why_not"], "not_in_store");
    let res = app.send(req(Method::GET, "/v1/apps/soon/news")).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "signed out");
}

// ---------------------------------------------------------------------------------------------
// Hidden posts

/// A post staff hid stays in the maker's list with the rule, and is gone for everyone else; it
/// can't be edited, removed or posted again, nor can another post be edited into its text.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_hidden_post_cant_be_changed_or_posted_again(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    let id = posted(&app, &maker, "soon", "Vote Slaps for a prize").await;
    hide_by_decision(&app, &admin, &fan, &id).await;

    assert_eq!(news_page(&app, "", "soon", "").await["news"], json!([]));
    assert_eq!(news_page(&app, &fan, "soon", "").await["news"], json!([]));
    let mine = news_page(&app, &maker, "soon", "").await;
    assert_eq!(mine["news"][0]["id"], id.as_str());
    assert_eq!(mine["news"][0]["hidden"], true);
    assert_eq!(mine["news"][0]["hidden_reason"], "Spam");
    assert_eq!(mine["news"][0]["can_appeal"], true);

    let res = edit_news(&app, &maker, "soon", &id, "Fixed", BODY).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(error(res).await.1, HIDDEN);
    let res = app
        .send(req(Method::DELETE, format!("/v1/apps/soon/news/{id}")).cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "kept for the appeal");
    assert_eq!(error(res).await.1, HIDDEN);

    a_day_later(&app, "soon").await;
    let res = post_news(&app, &maker, "soon", "Vote Slaps for a prize", BODY).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(error(res).await.1, HIDDEN);
    let other = posted(&app, &maker, "soon", "What is in the next release").await;
    let res = edit_news(&app, &maker, "soon", &other, "Vote Slaps for a prize", BODY).await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "nor through an edit");
    assert_eq!(error(res).await.1, HIDDEN);
}

/// Hidden posts are kept for the appeal, and deleted 213 days after they were hidden.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn hidden_news_is_deleted_after_213_days(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    listed_with(&app, maker_id, "old", "coming_soon", "").await;
    let hidden = posted(&app, &maker, "soon", "Hidden news").await;
    posted(&app, &maker, "old", "Old news").await;
    exec(
        &app,
        "update app_news set created_at = now() - interval '300 days' where headline = 'Old news'",
    )
    .await;
    sqlx::query(
        "update app_news set hidden_at = now() - interval '212 days', hidden_reason = 'Spam' where id = $1::uuid",
    )
    .bind(&hidden)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(slop_api::news::cleanup(&app.state).await.unwrap(), (0, 0));
    assert_eq!(count(&app, "select count(*) from app_news").await, 2);

    exec(
        &app,
        "update app_news set hidden_at = now() - interval '214 days' where hidden_at is not null",
    )
    .await;
    assert_eq!(slop_api::news::cleanup(&app.state).await.unwrap(), (0, 1));
    let left: Vec<String> = sqlx::query_scalar("select headline from app_news")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(left, ["Old news"], "an old post that isn't hidden stays");
}

/// A hidden post whose appeal is still waiting stays past the 213 days, until it's answered.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn hidden_news_waiting_for_its_appeal_is_kept(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    let id = posted(&app, &maker, "soon", "Vote Slaps for a prize").await;
    let decision = hide_by_decision(&app, &admin, &fan, &id).await;
    appeal(&app, &maker, &decision).await;
    exec(
        &app,
        "update app_news set hidden_at = now() - interval '214 days'",
    )
    .await;
    assert_eq!(slop_api::news::cleanup(&app.state).await.unwrap(), (0, 0));
    // The person deciding the appeal still reads it.
    let queued = queue(&app, &admin).await;
    assert_eq!(
        queued["appeals"][0]["quoted"],
        format!("Vote Slaps for a prize\n\n{BODY}")
    );

    answer_appeal(&app, &admin, &decision, false).await;
    assert_eq!(slop_api::news::cleanup(&app.state).await.unwrap(), (0, 1));
    assert_eq!(count(&app, "select count(*) from app_news").await, 0);
}

// ---------------------------------------------------------------------------------------------
// Reports, decisions and removal orders

/// Anyone but its maker can report a post; staff hide it with `hide_news`: it leaves the page and
/// the bell, the maker is told and sees it hidden with the rule, it can't be changed or come back
/// as another post, and an appeal puts it back without telling anyone again.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_can_be_reported_and_hidden(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    let other = posted(&app, &maker, "soon", "What is in the next release").await;
    a_day_later(&app, "soon").await;
    let id = posted(&app, &maker, "soon", "Vote Slaps for a prize").await;
    assert_eq!(news_notices(&app, fan_id).await.len(), 1);

    // Not its maker; not as a fake review; as a misleading page, yes.
    let res = report(&app, &maker, "news", &id, "misleading").await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "subject");
    let res = report(&app, &fan, "news", &id, "fake_review").await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "reason");
    let res = report(&app, &fan, "news", &id, "misleading").await;
    assert_eq!(res.status(), StatusCode::OK);

    // Staff see the post as it was when it was reported, and where it is.
    let queued = queue(&app, &admin).await;
    let r = &queued["reports"][0];
    assert_eq!(r["subject_kind"], "news");
    assert_eq!(r["subject_label"], "News from soon: Vote Slaps for a prize");
    assert_eq!(r["quoted"], format!("Vote Slaps for a prize\n\n{BODY}"));
    assert_eq!(r["link"], format!("/app/soon#news-{id}"));
    assert_eq!(
        r["actions"],
        json!(["hide_news", "warn", "suspend_account"])
    );
    assert_eq!(r["account"]["id"], maker_id.to_string());

    let decision = act(&app, &admin, "hide_news", "misleading").await;
    // Gone from the page and the bell; the maker is told and sees why.
    let public = news_page(&app, "", "soon", "").await;
    assert_eq!(public["news"].as_array().unwrap().len(), 1);
    assert_eq!(public["news"][0]["id"], other.as_str());
    assert!(
        news_notices(&app, fan_id).await.is_empty(),
        "out of the bell"
    );
    let mine = news_page(&app, &maker, "soon", "").await;
    assert_eq!(mine["news"][0]["id"], id.as_str());
    assert_eq!(mine["news"][0]["hidden"], true);
    assert_eq!(mine["news"][0]["hidden_reason"], "Misleading pages");
    assert_eq!(mine["news"][0]["can_appeal"], true);
    assert!(public["news"][0].get("can_appeal").is_none(), "{public}");
    let told: Vec<String> = sqlx::query_scalar(
        "select title from notifications where user_id = $1 and category = 'moderation'",
    )
    .bind(maker_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        told,
        ["News post hidden: News from soon: Vote Slaps for a prize"]
    );
    let (action, rule): (String, String) = sqlx::query_as(
        "select action, rule from moderation_decisions where id = $1::uuid and subject_kind = 'news'",
    )
    .bind(&decision)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (action.as_str(), rule.as_str()),
        ("hide_news", "misleading")
    );
    assert_eq!(
        count(
            &app,
            &format!(
                "select count(*) from audit_log where action = 'moderation.hide_news' and subject = '{id}'"
            )
        )
        .await,
        1
    );
    // Hidden, it can't be reported again either.
    let (_, rita) = app.user("Rita Reporter", "US").await;
    let res = report(&app, &rita, "news", &id, "spam").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // It can't be changed, removed or posted again, nor can another post take its text.
    let res = edit_news(&app, &maker, "soon", &id, "A fair headline", BODY).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(error(res).await.1, HIDDEN);
    assert_eq!(
        remove_news(&app, &maker, "soon", &id).await,
        StatusCode::CONFLICT
    );
    a_day_later(&app, "soon").await;
    let res = post_news(&app, &maker, "soon", "Vote Slaps for a prize", BODY).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let res = edit_news(&app, &maker, "soon", &other, "Vote Slaps for a prize", BODY).await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "nor through an edit");

    // The appeal: the reviewer reads what was hidden. Reversed, the post is back, and nobody is
    // told about it a second time.
    let res = app
        .send(
            req(Method::POST, format!("/v1/me/decisions/{decision}/appeal"))
                .cookie(&maker)
                .json(json!({ "appeal": "The prize is for a bug-hunting contest, not votes." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let queued = queue(&app, &admin).await;
    let appeal = &queued["appeals"][0];
    assert_eq!(appeal["action"], "hide_news");
    assert_eq!(
        appeal["quoted"],
        format!("Vote Slaps for a prize\n\n{BODY}")
    );
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/decisions/{decision}/appeal"),
            )
            .cookie(&admin)
            .json(json!({ "reverse": true, "reason": "It's a contest for bug reports." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let public = news_page(&app, "", "soon", "").await;
    assert_eq!(public["news"][0]["id"], id.as_str());
    assert_eq!(public["news"][0]["hidden"], false);
    assert!(public["news"][0].get("hidden_reason").is_none());
    assert!(
        news_notices(&app, fan_id).await.is_empty(),
        "nobody is told"
    );
}

/// Staff hiding a post while its maker changes it: the decision waits for the maker's change to
/// finish (both take the post before its notices), instead of the two waiting on each other and
/// one of them failing.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_hiding_a_post_waits_for_the_makers_change(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    let (_, rita) = app.user("Rita Reporter", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    let id = posted(&app, &maker, "soon", "Vote Slaps for a prize").await;
    let res = report(&app, &rita, "news", &id, "spam").await;
    assert_eq!(res.status(), StatusCode::OK);

    // The maker's edit, step by step as news.rs runs it: the post's row, then its notices.
    let mut edit = app.db.begin().await.unwrap();
    let pid = backend_pid(&mut edit).await;
    sqlx::query("select 1 from app_news where id = $1::uuid for update")
        .bind(&id)
        .execute(&mut *edit)
        .await
        .unwrap();
    let (_, ()) = tokio::join!(act(&app, &admin, "hide_news", "spam"), async {
        until_blocked_by(&app, pid).await;
        sqlx::query(
            "update notifications set body = 'Vote Slaps for a key' where id in (
                 select id from notifications where news_id = $1::uuid
                 order by user_id, id for update)",
        )
        .bind(&id)
        .execute(&mut *edit)
        .await
        .unwrap();
        edit.commit().await.unwrap();
    });
    let hidden: bool =
        sqlx::query_scalar("select hidden_at is not null from app_news where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(hidden);
    assert!(news_notices(&app, fan_id).await.is_empty());
}

/// A removal order can name a post a report already hid. Reversing the report's decision on appeal
/// then leaves the post hidden under the order; only when no decision hides it is it back.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_post_stays_hidden_while_another_decision_hides_it(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Owner").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    let id = posted(&app, &maker, "soon", "Join our cause").await;
    let by_report = hide_by_decision(&app, &admin, &fan, &id).await;
    let by_order = removal_order(&app, &admin, &id).await;
    let state = || async {
        sqlx::query_as::<_, (Option<OffsetDateTime>, Option<String>)>(
            "select hidden_at, hidden_reason from app_news where id = $1::uuid",
        )
        .bind(&id)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    assert_eq!(state().await.1.as_deref(), Some("Illegal content"));

    appeal(&app, &maker, &by_report).await;
    answer_appeal(&app, &admin, &by_report, true).await;
    let (at, reason) = state().await;
    assert_eq!(
        reason.as_deref(),
        Some("Illegal content"),
        "the order stands"
    );
    let ordered_at: OffsetDateTime =
        sqlx::query_scalar("select created_at from moderation_decisions where id = $1::uuid")
            .bind(&by_order)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(at, Some(ordered_at), "kept for the order's own appeal");
    assert_eq!(news_page(&app, "", "soon", "").await["news"], json!([]));
    let mine = news_page(&app, &maker, "soon", "").await;
    assert_eq!(mine["news"][0]["hidden"], true);
    assert_eq!(mine["news"][0]["hidden_reason"], "Illegal content");
    assert_eq!(mine["news"][0]["can_appeal"], true, "the order can be");

    appeal(&app, &maker, &by_order).await;
    answer_appeal(&app, &admin, &by_order, true).await;
    assert_eq!(state().await, (None, None));
    assert_eq!(news_page(&app, "", "soon", "").await["news"][0]["id"], id);
    assert!(
        news_notices(&app, fan_id).await.is_empty(),
        "nobody is told"
    );
}

/// The maker's list says whether a hidden post can still be appealed, and so does the refusal:
/// not once its appeal was answered, nor six months after the decision.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn hidden_posts_say_whether_they_can_still_be_appealed(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "kept", "coming_soon", "").await;
    listed_with(&app, maker_id, "late", "coming_soon", "").await;
    let kept = posted(&app, &maker, "kept", "Vote Slaps for a prize").await;
    let late = posted(&app, &maker, "late", "Vote Slaps for a key").await;
    let kept_decision = hide_by_decision(&app, &admin, &fan, &kept).await;
    hide_by_decision(&app, &admin, &fan, &late).await;
    let first = |slug: &'static str| {
        let (app, maker) = (&app, &maker);
        async move { news_page(app, maker, slug, "").await["news"][0].clone() }
    };
    assert_eq!(first("kept").await["can_appeal"], true);

    // Appealed and kept: there's no second appeal.
    appeal(&app, &maker, &kept_decision).await;
    assert_eq!(first("kept").await["can_appeal"], false, "while waiting");
    answer_appeal(&app, &admin, &kept_decision, false).await;
    assert_eq!(first("kept").await["can_appeal"], false);
    let res = edit_news(&app, &maker, "kept", &kept, "A fair headline", BODY).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(error(res).await.1, HIDDEN_DECIDED);
    let res = app
        .send(req(Method::DELETE, format!("/v1/apps/kept/news/{kept}")).cookie(&maker))
        .await;
    assert_eq!(error(res).await.1, HIDDEN_DECIDED);

    // Six months on, nor that one.
    sqlx::query(
        "update moderation_decisions set created_at = now() - interval '184 days' where subject_id = $1::uuid",
    )
    .bind(&late)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(first("late").await["can_appeal"], false);
    a_day_later(&app, "late").await;
    let res = post_news(&app, &maker, "late", "Vote Slaps for a key", BODY).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(error(res).await.1, HIDDEN_DECIDED);
}

/// A post can be reported only by someone who can see its app's page, as for the app itself.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_reports_follow_the_page(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, us) = app.user("Uma", "US").await;
    let (_, gb) = app.user("Gil", "GB").await;
    listed_with(&app, maker_id, "grown", "coming_soon", "min_age = 18").await;
    listed_with(&app, maker_id, "gone", "coming_soon", "").await;
    let grown = posted(&app, &maker, "grown", "Five new levels are in").await;
    let gone = posted(&app, &maker, "gone", "Fixes for the crashes you reported").await;
    exec(
        &app,
        "update apps set status = 'delisted' where slug = 'gone'",
    )
    .await;

    // An 18+ app is hidden in the under-18 view, so its news can't be reported from there.
    let res = report(&app, &gb, "news", &grown, "spam").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    // Signed out with no country: the under-18 view too; from a full-view country, yes.
    let signed_out = json!({
        "kind": "news", "subject": grown, "reason": "spam", "details": "Spam.",
        "good_faith": true, "name": "Sam", "email": "sam@example.com"
    });
    let res = app
        .send(req(Method::POST, "/v1/reports").json(signed_out.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .header("x-country", "US")
                .json(signed_out),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = report(&app, &us, "news", &grown, "spam").await;
    assert_eq!(res.status(), StatusCode::OK);
    // A post on an app out of the store, or something that isn't a post.
    let res = report(&app, &us, "news", &gone, "spam").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    for subject in ["grown".to_string(), Uuid::now_v7().to_string()] {
        let res = report(&app, &us, "news", &subject, "spam").await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{subject}");
    }
    assert_eq!(
        count(
            &app,
            "select count(*) from reports where subject_kind = 'news'"
        )
        .await,
        2
    );
}

/// A terrorist content removal order can name a news post: hidden as a `hide_news` decision at
/// once, out of the bell, with a sealed copy kept.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_removal_order_can_name_a_news_post(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Owner").await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    mark(&app, &fan, "soon").await;
    let id = posted(&app, &maker, "soon", "Join our cause").await;
    let order = |kind: &str, subject: &str| {
        req(Method::POST, "/v1/admin/removal-orders")
            .cookie(&admin)
            .json(json!({
                "received_at": (OffsetDateTime::now_utc() - time::Duration::minutes(5)).format(&Rfc3339).unwrap(),
                "country": "FR", "authority": "OCLCTIC", "order": "Order 2026-118",
                "subject_kind": kind, "subject": subject
            }))
    };
    let res = app.send(order("bounty", &id)).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body_json(res).await["error"]["message"],
        "app, app_review, news, profile, team or work_request"
    );

    let res = app.send(order("news", &id)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_json(res).await["label"],
        "News from soon: Join our cause"
    );
    let (hidden, reason): (bool, Option<String>) = sqlx::query_as(
        "select hidden_at is not null, hidden_reason from app_news where id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((hidden, reason.as_deref()), (true, Some("Illegal content")));
    let decision: (Uuid, String, String) =
        sqlx::query_as("select user_id, action, subject_kind from moderation_decisions")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(decision, (maker_id, "hide_news".into(), "news".into()));
    let (kind, headline): (String, String) = sqlx::query_as(
        "select subject_kind, snapshot->>'headline' from removed_content where subject_id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (kind.as_str(), headline.as_str()),
        ("news", "Join our cause")
    );
    assert!(news_notices(&app, fan_id).await.is_empty());
    assert_eq!(news_page(&app, "", "soon", "").await["news"], json!([]));
}

/// The export has the maker's posts (hidden ones too) and each mark's switch; closing the maker's
/// account deletes their posts, and everyone's notices about them go too.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_is_exported_and_goes_when_the_account_closes(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (otto_id, otto) = app.user("Otto Other", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    let (_, quiet) = app.user("Quinn Quiet", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    listed_with(&app, otto_id, "ottos", "coming_soon", "").await;
    for slug in ["soon", "ottos"] {
        mark(&app, &fan, slug).await;
    }
    mark(&app, &quiet, "soon").await;
    assert_eq!(
        switch_news(&app, &quiet, "soon", false).await,
        StatusCode::NO_CONTENT
    );
    let first = posted(&app, &maker, "soon", "What is in the next release").await;
    a_day_later(&app, "soon").await;
    posted(&app, &maker, "soon", "Vote Slaps for a prize").await;
    exec(
        &app,
        "update app_news set hidden_at = now(), hidden_reason = 'Spam' where headline like 'Vote%'",
    )
    .await;
    posted(&app, &otto, "ottos", "Otto's news").await;
    let export = |cookie: &str| {
        let r = req(Method::GET, "/v1/me/export").cookie(cookie);
        let app = &app;
        async move {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::OK);
            body_json(res).await
        }
    };

    let mine = export(&maker).await;
    let posts = mine["app_news"].as_array().unwrap();
    assert_eq!(posts.len(), 2, "{mine}");
    assert_eq!(posts[0]["app"], "soon");
    assert_eq!(posts[0]["id"], first.as_str());
    assert_eq!(posts[0]["headline"], "What is in the next release");
    assert_eq!(posts[0]["body"], BODY);
    assert!(posts[0].get("app_id").is_none());
    assert_eq!(posts[1]["hidden_reason"], "Spam");
    let fans = export(&fan).await;
    let mut marks: Vec<(String, bool)> = fans["interested"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["app"].as_str().unwrap().to_string(),
                m["news"].as_bool().unwrap(),
            )
        })
        .collect();
    marks.sort();
    assert_eq!(marks, [("ottos".into(), true), ("soon".into(), true)]);
    assert!(
        fans["notifications"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["category"] == "news"),
        "news notices are exported like every notice"
    );
    assert_eq!(export(&quiet).await["interested"][0]["news"], false);

    let res = app
        .send(
            req(Method::POST, "/v1/me/close")
                .cookie(&maker)
                .json(json!({ "confirm": "Close my account" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let left: Vec<String> = sqlx::query_scalar(
        "select a.slug from app_news n join apps a on a.id = n.app_id order by a.slug",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(left, ["ottos"], "only another maker's post is left");
    let bodies: Vec<String> = news_notices(&app, fan_id)
        .await
        .into_iter()
        .map(|(_, body, _, _)| body)
        .collect();
    assert_eq!(bodies, ["Otto's news"]);
}

/// The hourly bad-link scan reads maker news too, and names the post under its app; a hidden post
/// isn't scanned.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_links_join_the_bad_link_scan(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    listed_with(&app, maker_id, "quiet", "coming_soon", "").await;
    exec(
        &app,
        "insert into bad_links (url, host, source) values ('evil.example/steal', 'evil.example', 'URLhaus')",
    )
    .await;
    let body = "Beta keys for the first hundred: https://evil.example/steal";
    for slug in ["soon", "quiet"] {
        let res = post_news(&app, &maker, slug, "Beta keys", body).await;
        assert_eq!(res.status(), StatusCode::CREATED, "{slug}");
    }
    exec(
        &app,
        "update app_news n set hidden_at = now(), hidden_reason = 'Scams and fraud'
         from apps a where a.id = n.app_id and a.slug = 'quiet'",
    )
    .await;

    slop_api::abuse::run_once(&app.state).await.unwrap();
    let found: Vec<(String, String, Uuid, String, Value)> = sqlx::query_as(
        "select kind, subject_kind, subject_id, subject_label, detail from abuse_signals",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        found,
        [(
            "bad_link".to_string(),
            "app".to_string(),
            app_id(&app, "soon").await,
            "soon: news “Beta keys”".to_string(),
            json!({ "link": "https://evil.example/steal", "listed_by": "URLhaus" })
        )]
    );
}

// ---------------------------------------------------------------------------------------------
// Reading

/// The page is newest first, ten at a time unless asked, continued from the last post shown.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn news_pages_newest_first(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    // Five posts a day apart, straight in the database (one a day through the API).
    for n in 1..=5 {
        sqlx::query(
            "insert into app_news (id, app_id, headline, body, created_at, updated_at)
             select $1, id, $2, 'Text', now() - make_interval(days => 10 - $3), now() - make_interval(days => 10 - $3)
             from apps where slug = 'soon'",
        )
        .bind(Uuid::now_v7())
        .bind(format!("News {n}"))
        .bind(n)
        .execute(&app.db)
        .await
        .unwrap();
    }
    exec(
        &app,
        "update app_news set hidden_at = now(), hidden_reason = 'Spam' where headline = 'News 4'",
    )
    .await;
    let headlines = |page: &Value| -> Vec<String> {
        page["news"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["headline"].as_str().unwrap().to_string())
            .collect()
    };

    let first = news_page(&app, "", "soon", "?limit=2").await;
    assert_eq!(
        headlines(&first),
        ["News 5", "News 3"],
        "hidden ones left out"
    );
    assert_eq!(first["more"], true);
    let last = first["news"][1]["id"].as_str().unwrap();
    let next = news_page(&app, "", "soon", &format!("?limit=2&before={last}")).await;
    assert_eq!(headlines(&next), ["News 2", "News 1"]);
    assert_eq!(next["more"], false);
    assert_eq!(headlines(&news_page(&app, "", "soon", "").await).len(), 4);
    assert_eq!(
        headlines(&news_page(&app, "", "soon", "?limit=0").await).len(),
        1
    );
    // The maker's page has the hidden one too.
    assert_eq!(
        headlines(&news_page(&app, &maker, "soon", "").await),
        ["News 5", "News 4", "News 3", "News 2", "News 1"]
    );
    // A hidden post can't be a place to continue from, except for its maker.
    let hidden: String =
        sqlx::query_scalar("select id::text from app_news where headline = 'News 4'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    let query = format!("?before={hidden}");
    assert_eq!(
        headlines(&news_page(&app, "", "soon", &query).await).len(),
        0
    );
    assert_eq!(
        headlines(&news_page(&app, &maker, "soon", &query).await),
        ["News 3", "News 2", "News 1"]
    );
    let unknown = format!("?before={}", Uuid::now_v7());
    assert_eq!(
        headlines(&news_page(&app, "", "soon", &unknown).await).len(),
        0
    );
    let res = app
        .send(req(Method::GET, "/v1/apps/soon/news?before=yesterday"))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "before");
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/nowhere/news"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

// ---------------------------------------------------------------------------------------------
// The switch

/// "Tell me about news from this app" is the person's own switch on their own mark: shown on the
/// app page, works after release, back on with a new mark, and never for an agent token.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn people_switch_news_per_app(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (fan_id, fan) = app.user("Fay Fan", "US").await;
    let (_, stranger) = app.user("Sol Stranger", "US").await;
    listed_with(&app, maker_id, "soon", "coming_soon", "").await;
    let detail = |cookie: &str| {
        let mut r = req(Method::GET, "/v1/apps/soon");
        if !cookie.is_empty() {
            r = r.cookie(cookie);
        }
        r
    };
    let flags = |page: &Value| {
        (
            page["is_interested"].clone(),
            page["interested_news"].clone(),
        )
    };

    let page = body_json(app.send(detail("")).await).await;
    assert_eq!(flags(&page), (json!(false), json!(false)), "signed out");
    mark(&app, &fan, "soon").await;
    let page = body_json(app.send(detail(&fan)).await).await;
    assert_eq!(flags(&page), (json!(true), json!(true)), "on with the mark");
    assert_eq!(
        switch_news(&app, &fan, "soon", false).await,
        StatusCode::NO_CONTENT
    );
    let page = body_json(app.send(detail(&fan)).await).await;
    assert_eq!(flags(&page), (json!(true), json!(false)));

    // Only on your own mark.
    assert_eq!(
        switch_news(&app, &stranger, "soon", false).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        switch_news(&app, &fan, "nowhere", false).await,
        StatusCode::NOT_FOUND
    );
    for body in [
        json!({ "on": "yes" }),
        json!({ "on": true, "app": "soon" }),
        json!({}),
    ] {
        let res = app
            .send(
                req(Method::PUT, "/v1/me/interested/soon/news")
                    .cookie(&fan)
                    .json(body.clone()),
            )
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
    let res = app
        .send(req(Method::PUT, "/v1/me/interested/soon/news").json(json!({ "on": true })))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Switched off: not told.
    posted(&app, &maker, "soon", "Beta opens on Friday").await;
    assert!(news_notices(&app, fan_id).await.is_empty());

    // Removing the mark and marking again starts with news on.
    let res = app
        .send(req(Method::DELETE, "/v1/me/interested/soon").cookie(&fan))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    mark(&app, &fan, "soon").await;
    let page = body_json(app.send(detail(&fan)).await).await;
    assert_eq!(flags(&page), (json!(true), json!(true)));

    // A released app keeps the mark and its switch.
    exec(
        &app,
        "update apps set stage = 'released' where slug = 'soon'",
    )
    .await;
    assert_eq!(
        switch_news(&app, &fan, "soon", false).await,
        StatusCode::NO_CONTENT
    );
    let page = body_json(app.send(detail(&fan)).await).await;
    assert_eq!(flags(&page), (json!(true), json!(false)));
    let on: bool = sqlx::query_scalar(
        "select news from interests i join apps a on a.id = i.app_id where i.user_id = $1 and a.slug = 'soon'",
    )
    .bind(fan_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(!on);

    // The switch is the person's: a maker's AI agent token can't reach it.
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
    let res = app
        .send(
            req(Method::PUT, "/v1/me/interested/soon/news")
                .header("authorization", token)
                .json(json!({ "on": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}
