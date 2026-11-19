// SPDX-License-Identifier: AGPL-3.0-or-later
//! The beta's invite code for going public (invites.rs): publishing, putting an app in the store
//! and making a private app public; entering the code; staff's settings.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers

async fn status_of(app: &TestApp, r: Req) -> StatusCode {
    app.send(r).await.status()
}

/// Staff switch the code on (or off) with `code`.
async fn set_invite(app: &TestApp, admin: &str, required: bool, code: &str) -> Value {
    let res = app
        .send(
            req(Method::PUT, "/v1/admin/publish-invite")
                .cookie(admin)
                .json(json!({ "required": required, "code": code })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

async fn enter(app: &TestApp, cookie: &str, code: &str) -> Response {
    app.send(
        req(Method::POST, "/v1/me/publish-invite")
            .cookie(cookie)
            .json(json!({ "code": code })),
    )
    .await
}

async fn mine(app: &TestApp, cookie: &str) -> Value {
    body_json(
        app.send(req(Method::GET, "/v1/me/publish-invite").cookie(cookie))
            .await,
    )
    .await
}

/// The refusal says what to do, with its own code the website catches.
async fn assert_invite_required(res: Response) {
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let body = body_json(res).await;
    assert_eq!(body["error"]["code"], "invite_required", "{body}");
    assert_eq!(
        body["error"]["message"],
        "SLAPorSLOP is in beta: going public needs an invite code. Enter it when you publish on the website."
    );
}

/// A release of `game` that passed its checks and has its source check's result.
async fn ready_release(app: &TestApp, maker: &str, version: &str) -> String {
    let id = submit_and_fetch(app, maker, version).await;
    sqlx::query(
        "update releases set repo_scan = '{\"status\": \"not_checked\"}', repo_scanned_at = now()
         where id = $1::uuid",
    )
    .bind(&id)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

fn publish(id: &str, cookie: &str) -> Req {
    req(Method::POST, format!("/v1/apps/game/releases/{id}/publish")).cookie(cookie)
}

/// A private app `slug` made by the user signed in with `cookie`, through the API.
async fn private_app(app: &TestApp, cookie: &str, slug: &str) {
    let mut body = new_app(slug);
    body["visibility"] = json!("private");
    let res = app
        .send(req(Method::POST, "/v1/apps").cookie(cookie).json(body))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

fn make_public(slug: &str, cookie: &str) -> Req {
    req(Method::POST, format!("/v1/apps/{slug}/make-public")).cookie(cookie)
}

fn has_invite_step(steps: &Value) -> bool {
    steps["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["what"].as_str().unwrap().contains("needs an invite code") && s["who"] == "you")
}

async fn next_steps(app: &TestApp, r: Req) -> Value {
    body_json(app.send(r).await).await
}

// ---------------------------------------------------------------------------------------------
// Tests

/// Off by default: publishing works as before. Switched on, a maker without the code can't
/// publish or put their app in the store; a wrong code is refused, the right one (any letter
/// case, spaces around it) lets them, and what's left says so until then.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn going_public_needs_the_invite_code_once_staff_switch_it_on(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    assert_eq!(
        mine(&app, &maker).await,
        json!({ "required": false, "invited": false })
    );
    let first = ready_release(&app, &maker, "1.0.0").await;
    assert_eq!(
        status_of(&app, publish(&first, &maker)).await,
        StatusCode::OK
    );

    let view = set_invite(&app, &admin, true, "  Beta-Fox ").await;
    assert_eq!(view["required"], true);
    assert_eq!(view["code"], "Beta-Fox", "trimmed");
    assert_eq!(view["invited"], 0);
    assert_eq!(
        mine(&app, &maker).await,
        json!({ "required": true, "invited": false })
    );
    let second = ready_release(&app, &maker, "1.1.0").await;
    assert_invite_required(app.send(publish(&second, &maker)).await).await;
    assert_invite_required(
        app.send(req(Method::POST, "/v1/apps/game/list").cookie(&maker))
            .await,
    )
    .await;
    let steps = next_steps(&app, req(Method::GET, "/v1/me/next-steps").cookie(&maker)).await;
    assert!(has_invite_step(&steps), "{steps}");
    // Still a stranger's 404 first: the gate tells nobody else anything.
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    assert_eq!(
        status_of(&app, publish(&second, &stranger)).await,
        StatusCode::NOT_FOUND
    );

    // A wrong code, and an empty one.
    for wrong in ["beta-wolf", "   "] {
        let res = enter(&app, &maker, wrong).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "code");
    }
    assert_invite_required(app.send(publish(&second, &maker)).await).await;

    let res = enter(&app, &maker, " BETA-fox\n").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_json(res).await,
        json!({ "required": true, "invited": true })
    );
    assert_eq!(
        status_of(&app, publish(&second, &maker)).await,
        StatusCode::OK
    );
    assert_eq!(
        status_of(&app, req(Method::POST, "/v1/apps/game/list").cookie(&maker)).await,
        StatusCode::NO_CONTENT
    );
    let steps = next_steps(&app, req(Method::GET, "/v1/me/next-steps").cookie(&maker)).await;
    assert!(!has_invite_step(&steps), "{steps}");
    let invited: Option<OffsetDateTime> =
        sqlx::query_scalar("select publish_invited_at from users where id = $1")
            .bind(maker_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(invited.is_some());

    // Switched off again, nobody is asked.
    set_invite(&app, &admin, false, "").await;
    let (_, newcomer) = app.user("Nina Newcomer", "US").await;
    assert_eq!(
        mine(&app, &newcomer).await,
        json!({ "required": false, "invited": false })
    );
    private_app(&app, &newcomer, "open-tool").await;
    assert_eq!(
        status_of(&app, make_public("open-tool", &newcomer)).await,
        StatusCode::NO_CONTENT
    );
}

/// A new code keeps everyone who entered an earlier one; newcomers need the new one, and the old
/// one stops working. Entering again keeps the first time.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_new_code_keeps_those_who_entered_the_old_one(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (early_id, early) = app.user("Early Maker", "US").await;
    let (_, late) = app.user("Late Maker", "US").await;
    private_app(&app, &early, "early-tool").await;
    private_app(&app, &late, "late-tool").await;
    set_invite(&app, &admin, true, "first-code").await;
    assert_eq!(
        enter(&app, &early, "first-code").await.status(),
        StatusCode::OK
    );
    let first_time: OffsetDateTime =
        sqlx::query_scalar("select publish_invited_at from users where id = $1")
            .bind(early_id)
            .fetch_one(&app.db)
            .await
            .unwrap();

    let view = set_invite(&app, &admin, true, "second-code").await;
    assert_eq!(view["invited"], 1);
    assert_eq!(view["invited_since_update"], 0);
    assert_eq!(
        status_of(&app, make_public("early-tool", &early)).await,
        StatusCode::NO_CONTENT
    );
    assert_invite_required(app.send(make_public("late-tool", &late)).await).await;
    assert_eq!(
        enter(&app, &late, "first-code").await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        enter(&app, &late, "Second-Code").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        status_of(&app, make_public("late-tool", &late)).await,
        StatusCode::NO_CONTENT
    );
    let view = body_json(
        app.send(req(Method::GET, "/v1/admin/publish-invite").cookie(&admin))
            .await,
    )
    .await;
    assert_eq!(
        (
            view["invited"].clone(),
            view["invited_since_update"].clone()
        ),
        (json!(2), json!(1))
    );

    assert_eq!(
        enter(&app, &early, "second-code").await.status(),
        StatusCode::OK
    );
    let again: OffsetDateTime =
        sqlx::query_scalar("select publish_invited_at from users where id = $1")
            .bind(early_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(again, first_time);
}

/// Staff never need the code.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_go_public_without_the_code(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    set_invite(&app, &admin, true, "beta-fox").await;
    assert_eq!(
        mine(&app, &admin).await,
        json!({ "required": true, "invited": true })
    );
    private_app(&app, &admin, "staff-tool").await;
    assert_eq!(
        status_of(&app, make_public("staff-tool", &admin)).await,
        StatusCode::NO_CONTENT
    );
}

/// Only staff see the settings (anyone else: there's nothing there); only reviewers and up
/// change them, with a code of 4 to 64 characters when it's on, in the audit log.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_staff_see_and_change_the_invite_code(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (support_id, support) = make_admin(&app, "Sue Support").await;
    sqlx::query("delete from staff_roles where user_id = $1")
        .bind(support_id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into staff_roles (user_id, role, granted_by) values ($1, 'support', 'test')",
    )
    .bind(support_id)
    .execute(&app.db)
    .await
    .unwrap();
    let (_, player) = app.user("Pat Player", "US").await;
    let put = |cookie: &str, body: Value| {
        req(Method::PUT, "/v1/admin/publish-invite")
            .cookie(cookie)
            .json(body)
    };
    for r in [
        req(Method::GET, "/v1/admin/publish-invite").cookie(&player),
        put(&player, json!({ "required": true, "code": "beta-fox" })),
        req(Method::GET, "/v1/admin/publish-invite"),
    ] {
        let status = status_of(&app, r).await;
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::UNAUTHORIZED,
            "{status}"
        );
    }
    let view = body_json(
        app.send(req(Method::GET, "/v1/admin/publish-invite").cookie(&support))
            .await,
    )
    .await;
    assert_eq!(view["required"], false);
    assert_eq!(view["code"], "");
    assert_eq!(
        status_of(
            &app,
            put(&support, json!({ "required": true, "code": "beta-fox" }))
        )
        .await,
        StatusCode::FORBIDDEN
    );
    for (required, code) in [
        (true, ""),
        (true, "abc"),
        (true, &*"x".repeat(65)),
        (false, "ab"),
    ] {
        let res = app
            .send(put(&admin, json!({ "required": required, "code": code })))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{code}");
        assert_eq!(body_json(res).await["error"]["field"], "code");
    }
    set_invite(&app, &admin, true, "beta-fox").await;
    let (actor, detail): (String, Value) =
        sqlx::query_as("select actor, detail from audit_log where action = 'publish_invite.set'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(actor.contains("Ada Admin"), "{actor}");
    assert_eq!(detail, json!({ "required": true, "code": "beta-fox" }));
}

/// An AI agent can't enter the code (it isn't one of the routes a token may call, and the route
/// refuses agents itself), and its what's-left tells it the person has to.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn agents_cannot_enter_the_invite_code(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    set_invite(&app, &admin, true, "beta-fox").await;
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
            req(Method::POST, "/v1/me/publish-invite")
                .header("authorization", token.clone())
                .json(json!({ "code": "beta-fox" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let invited: Option<OffsetDateTime> =
        sqlx::query_scalar("select publish_invited_at from users where id = $1")
            .bind(maker_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(invited.is_none());
    let steps = next_steps(
        &app,
        req(Method::GET, "/v1/me/next-steps").header("authorization", token),
    )
    .await;
    assert!(has_invite_step(&steps), "{steps}");
}

/// Ten tries an hour: enough for typos, too few to guess it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn invite_code_tries_are_limited(db: PgPool) {
    let app = test_app(db).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    set_invite(&app, &admin, true, "beta-fox").await;
    for i in 0..10 {
        assert_eq!(
            enter(&app, &maker, &format!("guess-{i}")).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let res = enter(&app, &maker, "beta-fox").await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body_json(res).await["error"]["code"], "rate_limited");
    assert_eq!(
        mine(&app, &maker).await,
        json!({ "required": true, "invited": false })
    );
}
