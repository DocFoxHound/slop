// SPDX-License-Identifier: AGPL-3.0-or-later
//! Publishing, repository ownership and identity, releases, provenance, Slop Build, the GitHub App, scanning, branches, fixes, desktop updates, app management.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers

/// Uses up an account's whole allowance of `action` (limits.rs), so the next one is refused.
fn exhaust(app: &TestApp, user: Uuid, action: slop_api::limits::Action) {
    let now = std::time::Instant::now();
    while app.state.account_limiter.take(user, action, now).is_ok() {}
}

/// The `error.message` of a refusal.
async fn error_message(res: Response) -> String {
    body_json(res).await["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The `error.field` of a 422.
async fn error_field(res: Response) -> String {
    let status = res.status();
    let body = body_json(res).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    body["error"]["field"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn status_of(app: &TestApp, r: Req) -> StatusCode {
    app.send(r).await.status()
}

/// Starts a small local server (a stand-in for some outside service) and returns its base URL.
async fn serve_fake(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}/")
}

/// Private Slop on octo/<slug>, made through the API, with an approved release when `approved`.
async fn private_app(app: &TestApp, owner: Uuid, cookie: &str, slug: &str, approved: bool) {
    let mut body = new_app(slug);
    body["visibility"] = json!("private");
    body["repo_url"] = json!(format!("https://github.com/octo/{slug}"));
    let res = app
        .send(req(Method::POST, "/v1/apps").cookie(cookie).json(body))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    if approved {
        approved_release(app, slug, owner).await;
    }
}

/// A team named "Team <slug>" with these members (the first is its owner).
async fn team(app: &TestApp, slug: &str, kind: &str, join_policy: &str, members: &[Uuid]) {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into teams (id, slug, name, kind, join_policy, created_by, github_org)
         values ($1, $2, $3, $4, $5, $6, case when $4 = 'official' then 'acme' end)",
    )
    .bind(id)
    .bind(slug)
    .bind(format!("Team {slug}"))
    .bind(kind)
    .bind(join_policy)
    .bind(members[0])
    .execute(&app.db)
    .await
    .unwrap();
    for (i, m) in members.iter().enumerate() {
        sqlx::query("insert into team_members (team_id, user_id, role) values ($1, $2, $3)")
            .bind(id)
            .bind(m)
            .bind(if i == 0 { "owner" } else { "member" })
            .execute(&app.db)
            .await
            .unwrap();
    }
}

// ---------------------------------------------------------------------------------------------
// Private Slop sharing (private.rs, frozen.rs)

/// The share list shows people and teams by name, to the owner only, and only for Private Slop.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_owner_sees_who_private_slop_is_shared_with_by_name(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = app.user("Olive Owner", "US").await;
    let (zed_id, zed) = app.user("Zed Zebra", "US").await;
    let (amy_id, _) = app.user("Amy Apple", "US").await;
    private_app(&app, owner_id, &owner, "secret-tool", true).await;
    team(&app, "mid-team", "player", "invite", &[owner_id]).await;
    for body in [
        json!({ "user": zed_id.to_string() }),
        json!({ "team": "mid-team" }),
        json!({ "user": amy_id.to_string() }),
    ] {
        let res = app
            .send(
                req(Method::POST, "/v1/apps/secret-tool/shares")
                    .cookie(&owner)
                    .json(body),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
    }
    let res = app
        .send(req(Method::GET, "/v1/apps/secret-tool/shares").cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(res).await;
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Amy Apple", "Team mid-team", "Zed Zebra"]);
    assert_eq!(list[1]["team"], "mid-team");
    assert!(list[1]["user_id"].is_null());
    assert_eq!(list[0]["user_id"], amy_id.to_string());
    assert!(list[0]["team"].is_null());

    // Someone it's shared with, or the signed out, can't see the list.
    assert_eq!(
        status_of(
            &app,
            req(Method::GET, "/v1/apps/secret-tool/shares").cookie(&zed)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(&app, req(Method::GET, "/v1/apps/secret-tool/shares")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::GET, "/v1/apps/nothing-here/shares").cookie(&owner)
        )
        .await,
        StatusCode::NOT_FOUND
    );

    // A public app isn't shared this way.
    let res = app
        .send(
            req(Method::POST, "/v1/apps")
                .cookie(&owner)
                .json(new_app("open-game")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let res = app
        .send(req(Method::GET, "/v1/apps/open-game/shares").cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        error_message(res)
            .await
            .contains("public apps are in the store")
    );
    let res = app
        .send(
            req(Method::POST, "/v1/apps/open-game/shares")
                .cookie(&owner)
                .json(json!({ "user": zed_id.to_string() })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        count(
            &app,
            "select count(*) from app_shares s join apps a on a.id = s.app_id where a.slug = 'open-game'"
        )
        .await,
        0
    );
}

/// Sharing refuses yourself, accounts that can't use Slop, links that aren't profiles, official
/// teams, open teams and teams the owner isn't in; a repeat share doesn't tell them twice.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sharing_private_slop_refuses_what_it_cannot_share_with(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = app.user("Olive Owner", "US").await;
    let (friend_id, _) = app.user("Finn Friend", "US").await;
    let (closed_id, _) = app.user("Cleo Closed", "US").await;
    let (suspended_id, _) = app.user("Sid Suspended", "US").await;
    let (pal_id, _) = app.user("Pat Pal", "US").await;
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(closed_id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(suspended_id)
        .execute(&app.db)
        .await
        .unwrap();
    private_app(&app, owner_id, &owner, "secret-tool", true).await;
    let share = |body: Value| {
        req(Method::POST, "/v1/apps/secret-tool/shares")
            .cookie(&owner)
            .json(body)
    };

    for (body, says) in [
        (json!({ "user": owner_id.to_string() }), "already yours"),
        (
            json!({ "user": Uuid::now_v7().to_string() }),
            "no Slop account",
        ),
        (json!({ "user": closed_id.to_string() }), "no Slop account"),
        (
            json!({ "user": suspended_id.to_string() }),
            "no Slop account",
        ),
        (
            json!({ "user": "https://example.com/not-a-profile" }),
            "profile link",
        ),
        (json!({}), "profile link"),
    ] {
        let res = app.send(share(body.clone())).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let err = body_json(res).await;
        assert_eq!(err["error"]["field"], "user");
        assert!(
            err["error"]["message"].as_str().unwrap().contains(says),
            "{body}: {err}"
        );
    }
    assert_eq!(
        status_of(&app, share(json!({ "user": friend_id, "colour": "red" }))).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown fields are refused"
    );

    // Teams: only invite-only player teams the owner is in.
    team(
        &app,
        "official-team",
        "official",
        "invite",
        &[owner_id, pal_id],
    )
    .await;
    team(&app, "open-team", "player", "open", &[owner_id, pal_id]).await;
    team(&app, "their-team", "player", "invite", &[pal_id, friend_id]).await;
    for (slug, says) in [
        ("official-team", "official teams"),
        ("open-team", "invite-only"),
        ("their-team", "you're in"),
        ("no-such-team", "you're in"),
    ] {
        let res = app.send(share(json!({ "team": slug }))).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{slug}");
        let err = body_json(res).await;
        assert_eq!(err["error"]["field"], "team");
        assert!(
            err["error"]["message"].as_str().unwrap().contains(says),
            "{slug}: {err}"
        );
    }
    assert_eq!(count(&app, "select count(*) from app_shares").await, 0);
    assert_eq!(count(&app, "select count(*) from app_team_shares").await, 0);
    assert_eq!(
        count(
            &app,
            "select count(*) from notifications where category = 'shared'"
        )
        .await,
        0
    );

    // A share notifies once; sharing again changes nothing.
    for _ in 0..2 {
        let res = app
            .send(share(json!({ "user": friend_id.to_string() })))
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        assert_eq!(body_json(res).await.as_array().unwrap().len(), 1);
    }
    let told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and category = 'shared'",
    )
    .bind(friend_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 1);

    // A team share tells the other members once, not the owner.
    team(&app, "our-team", "player", "invite", &[owner_id, pal_id]).await;
    for _ in 0..2 {
        assert_eq!(
            status_of(&app, share(json!({ "team": " our-team " }))).await,
            StatusCode::CREATED
        );
    }
    let told: Vec<(Uuid, String)> = sqlx::query_as(
        "select user_id, title from notifications where category = 'shared' and user_id <> $1",
    )
    .bind(friend_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        told,
        [(
            pal_id,
            "Olive Owner shared Pixel Dungeon Delver with Team our-team".to_string()
        )]
    );

    // Sharing too fast is refused.
    exhaust(&app, owner_id, slop_api::limits::Action::Share);
    let res = app.send(share(json!({ "user": pal_id.to_string() }))).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers().contains_key("retry-after"));
    assert_eq!(count(&app, "select count(*) from app_shares").await, 1);
}

/// Only the owner stops a share; a stopped team share takes the app away from its members.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_the_owner_stops_sharing_with_people_and_teams(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = app.user("Olive Owner", "US").await;
    let (friend_id, friend) = app.user("Finn Friend", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    private_app(&app, owner_id, &owner, "secret-tool", true).await;
    team(&app, "our-team", "player", "invite", &[owner_id, pal_id]).await;
    for body in [
        json!({ "user": friend_id.to_string() }),
        json!({ "team": "our-team" }),
    ] {
        app.send(
            req(Method::POST, "/v1/apps/secret-tool/shares")
                .cookie(&owner)
                .json(body),
        )
        .await;
    }
    let sees = |cookie: &str| req(Method::GET, "/v1/apps/secret-tool").cookie(cookie);
    assert_eq!(status_of(&app, sees(&friend)).await, StatusCode::OK);
    assert_eq!(status_of(&app, sees(&pal)).await, StatusCode::OK);

    // Someone else's attempts are 404 and change nothing.
    assert_eq!(
        status_of(
            &app,
            req(
                Method::DELETE,
                format!("/v1/apps/secret-tool/shares/{friend_id}")
            )
            .cookie(&friend)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, "/v1/apps/secret-tool/team-shares/our-team").cookie(&pal)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(count(&app, "select count(*) from app_shares").await, 1);
    assert_eq!(count(&app, "select count(*) from app_team_shares").await, 1);

    // An unknown team is nothing to stop.
    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, "/v1/apps/secret-tool/team-shares/nobody").cookie(&owner)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(count(&app, "select count(*) from app_team_shares").await, 1);

    // The owner stops the team share: its members lose the app, the person keeps it.
    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, "/v1/apps/secret-tool/team-shares/our-team").cookie(&owner)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(count(&app, "select count(*) from app_team_shares").await, 0);
    assert_eq!(status_of(&app, sees(&pal)).await, StatusCode::NOT_FOUND);
    assert_eq!(status_of(&app, sees(&friend)).await, StatusCode::OK);
    let shared = body_json(
        app.send(req(Method::GET, "/v1/me/private").cookie(&pal))
            .await,
    )
    .await;
    assert!(shared["shared"].as_array().unwrap().is_empty());
}

/// "My Private Slop": the user's own private apps (even without a release), and apps shared
/// with them that have an approved release and aren't frozen past the grace period.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn my_private_slop_lists_mine_and_what_is_ready_for_me(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = app.user("Olive Owner", "US").await;
    let (friend_id, friend) = app.user("Finn Friend", "US").await;
    private_app(&app, owner_id, &owner, "ready-tool", true).await;
    private_app(&app, owner_id, &owner, "unready-tool", false).await;
    private_app(&app, owner_id, &owner, "frozen-tool", true).await;
    app.send(
        req(Method::POST, "/v1/apps")
            .cookie(&owner)
            .json(new_app("open-game")),
    )
    .await;
    for slug in ["ready-tool", "unready-tool", "frozen-tool"] {
        let res = app
            .send(
                req(Method::POST, format!("/v1/apps/{slug}/shares"))
                    .cookie(&owner)
                    .json(json!({ "user": friend_id.to_string() })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
    }
    sqlx::query(
        "update apps set frozen_at = now() - interval '15 days', frozen_reason = 'gone' where slug = 'frozen-tool'",
    )
    .execute(&app.db)
    .await
    .unwrap();

    let slugs = |v: &Value| -> Vec<String> {
        let mut s: Vec<String> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["slug"].as_str().unwrap().to_string())
            .collect();
        s.sort();
        s
    };
    let res = app
        .send(req(Method::GET, "/v1/me/private").cookie(&owner))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let mine = body_json(res).await;
    assert_eq!(
        slugs(&mine["mine"]),
        ["frozen-tool", "ready-tool", "unready-tool"]
    );
    assert!(mine["shared"].as_array().unwrap().is_empty());

    let theirs = body_json(
        app.send(req(Method::GET, "/v1/me/private").cookie(&friend))
            .await,
    )
    .await;
    assert!(theirs["mine"].as_array().unwrap().is_empty());
    assert_eq!(slugs(&theirs["shared"]), ["ready-tool"]);
    assert_eq!(theirs["shared"][0]["owner"], "Olive Owner");

    // Still listed within the 14 days.
    sqlx::query(
        "update apps set frozen_at = now() - interval '13 days' where slug = 'frozen-tool'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let theirs = body_json(
        app.send(req(Method::GET, "/v1/me/private").cookie(&friend))
            .await,
    )
    .await;
    assert_eq!(slugs(&theirs["shared"]), ["frozen-tool", "ready-tool"]);

    assert_eq!(
        status_of(&app, req(Method::GET, "/v1/me/private")).await,
        StatusCode::UNAUTHORIZED
    );
}

/// Taking back a request to go public takes the app out of the reviewers' queue.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_owner_can_take_back_a_request_to_go_public(db: PgPool) {
    let app = test_app(db).await;
    let (owner_id, owner) = app.user("Olive Owner", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    let (_, admin_cookie) = make_admin(&app, "Ada Admin").await;
    let mut body = new_app("open-tool");
    body["visibility"] = json!("private");
    let res = app
        .send(req(Method::POST, "/v1/apps").cookie(&owner).json(body))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    approved_release(&app, "open-tool", owner_id).await;
    assert_eq!(
        status_of(
            &app,
            req(Method::POST, "/v1/apps/open-tool/make-public").cookie(&owner)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let going_public = || async {
        let queue = body_json(
            app.send(req(Method::GET, "/v1/admin/queue").cookie(&admin_cookie))
                .await,
        )
        .await;
        queue["ready_to_list"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["going_public"] == true)
    };
    assert!(going_public().await);

    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, "/v1/apps/open-tool/make-public").cookie(&stranger)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert!(going_public().await, "a stranger changes nothing");
    assert_eq!(
        status_of(&app, req(Method::DELETE, "/v1/apps/open-tool/make-public")).await,
        StatusCode::UNAUTHORIZED
    );

    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, "/v1/apps/open-tool/make-public").cookie(&owner)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let requested: Option<OffsetDateTime> =
        sqlx::query_scalar("select public_requested_at from apps where slug = 'open-tool'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(requested.is_none());
    assert!(!going_public().await);
    let overview = body_json(
        app.send(req(Method::GET, "/v1/apps/open-tool/manage").cookie(&owner))
            .await,
    )
    .await;
    assert_eq!(overview["public_requested"], false);
}

// ---------------------------------------------------------------------------------------------
// Makers' signing keys (maker_keys.rs)

fn ssh_key(seed: u8) -> String {
    let key = slop_manifest::ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
    ssh_sign(&key, b"").0
}

fn add_key(cookie: &str, name: &str, public_key: &str) -> Req {
    req(Method::POST, "/v1/me/signing-keys")
        .cookie(cookie)
        .json(json!({ "name": name, "public_key": public_key }))
}

/// Keys are checked, named, limited to five, audited, and listed only to their owner.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn signing_keys_are_checked_limited_and_listed_to_their_owner(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (_, bob) = app.user("Bob", "US").await;

    // Bad keys and names.
    let long_name = "n".repeat(61);
    for (name, key, field) in [
        (
            "Laptop",
            "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ ann@laptop".to_string(),
            "public_key",
        ),
        ("Laptop", "not a key".to_string(), "public_key"),
        ("Laptop", String::new(), "public_key"),
        ("   ", ssh_key(1), "name"),
        (long_name.as_str(), ssh_key(1), "name"),
    ] {
        assert_eq!(
            error_field(app.send(add_key(&ann, name, &key)).await).await,
            field,
            "{name} {key}"
        );
    }
    assert_eq!(count(&app, "select count(*) from maker_keys").await, 0);
    assert_eq!(
        status_of(
            &app,
            req(Method::POST, "/v1/me/signing-keys")
                .json(json!({ "name": "x", "public_key": ssh_key(1) }))
        )
        .await,
        StatusCode::UNAUTHORIZED
    );

    // Five keys (a 60-character name is fine); the sixth is refused.
    let mut ids = vec![];
    for seed in 1..=5u8 {
        let name = if seed == 1 {
            "k".repeat(60)
        } else {
            format!("Key {seed}")
        };
        let res = app.send(add_key(&ann, &name, &ssh_key(seed))).await;
        assert_eq!(res.status(), StatusCode::CREATED);
        let key = body_json(res).await;
        assert_eq!(key["active"], false);
        ids.push(key["id"].as_str().unwrap().to_string());
    }
    let res = app.send(add_key(&ann, "Sixth", &ssh_key(6))).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("up to 5"));
    let audited: i64 = sqlx::query_scalar(
        "select count(*) from audit_log where action = 'maker_key.add' and actor = $1",
    )
    .bind(ann_id.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(audited, 5);

    // Listed to Ann only, with whether each works yet.
    sqlx::query(
        "update maker_keys set active_from = now() - interval '1 hour' where id = $1::uuid",
    )
    .bind(&ids[1])
    .execute(&app.db)
    .await
    .unwrap();
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/signing-keys").cookie(&ann))
            .await,
    )
    .await;
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 5);
    assert_eq!(list[1]["id"], ids[1].as_str());
    assert_eq!(list[1]["active"], true);
    assert_eq!(list[0]["active"], false);
    assert!(
        list[0]["fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("SHA256:")
    );
    let theirs = body_json(
        app.send(req(Method::GET, "/v1/me/signing-keys").cookie(&bob))
            .await,
    )
    .await;
    assert_eq!(theirs, json!([]));
    assert_eq!(
        status_of(&app, req(Method::GET, "/v1/me/signing-keys")).await,
        StatusCode::UNAUTHORIZED
    );

    // A removed key isn't listed and frees a place.
    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, format!("/v1/me/signing-keys/{}", ids[4])).cookie(&ann)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let list = body_json(
        app.send(req(Method::GET, "/v1/me/signing-keys").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 4);
    assert_eq!(
        status_of(&app, add_key(&ann, "Sixth", &ssh_key(6))).await,
        StatusCode::CREATED
    );
}

/// Bug b26: re-adding a key you already have (to rename it) isn't a new key, so the five-key
/// limit doesn't apply to it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn renaming_a_signing_key_works_at_the_five_key_limit(db: PgPool) {
    let app = test_app(db).await;
    let (_, ann) = app.user("Ann", "US").await;
    for seed in 1..=5u8 {
        assert_eq!(
            status_of(&app, add_key(&ann, &format!("Key {seed}"), &ssh_key(seed))).await,
            StatusCode::CREATED
        );
    }
    let res = app.send(add_key(&ann, "Renamed", &ssh_key(3))).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(body_json(res).await["name"], "Renamed");
    assert_eq!(
        count(
            &app,
            "select count(*) from maker_keys where revoked_at is null"
        )
        .await,
        5
    );
}

/// A release page of its own that serves the build and a maker's `.sig` beside it (so this
/// test doesn't share MAKER_SIG with others running at the same time).
async fn signed_release_host(sig: String) -> String {
    let router = Router::new().route(
        "/octo/game/releases/download/{tag}/{file}",
        get(
            move |axum::extract::Path((_, file)): axum::extract::Path<(String, String)>| {
                let sig = sig.clone();
                async move {
                    if file.ends_with(".sig") {
                        sig.into_response()
                    } else {
                        BUILD_BYTES.into_response()
                    }
                }
            },
        ),
    );
    serve_fake(router).await
}

/// Removing a key works at once, only on your own keys, and only once; re-adding it starts the
/// 72-hour wait again.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn removed_signing_keys_stop_signing_and_start_over_when_added_again(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (_, bob) = app.user("Bob", "US").await;
    let key = slop_manifest::ed25519_dalek::SigningKey::from_bytes(&[21; 32]);
    let (public_line, signature) = ssh_sign(&key, BUILD_BYTES);
    let host = signed_release_host(signature).await;
    sqlx::query("update apps set repo_url = $1")
        .bind(format!("{host}octo/game"))
        .execute(&app.db)
        .await
        .unwrap();
    let submit = |version: &str| {
        let mut body = release_body(&app, version, "game-windows.zip");
        body["files"][0]["url"] = json!(format!(
            "{host}octo/game/releases/download/v1/game-windows.zip"
        ));
        req(Method::POST, "/v1/apps/game/releases")
            .cookie(&maker)
            .json(body)
    };
    let release = |version: &str| {
        let r = submit(version);
        async {
            let res = app.send(r).await;
            assert_eq!(res.status(), StatusCode::CREATED);
            let id = body_json(res).await["id"].as_str().unwrap().to_string();
            assert!(releases::fetch_next(&app.state).await.unwrap());
            let (key, problem): (Option<String>, Option<String>) = sqlx::query_as(
                "select maker_signature->>'key_name', provenance_problem from release_files where release_id = $1::uuid",
            )
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
            (key, problem)
        }
    };
    let res = app.send(add_key(&maker, "Laptop", &public_line)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    sqlx::query("update maker_keys set active_from = now() - interval '1 hour'")
        .execute(&app.db)
        .await
        .unwrap();

    // Bob can't remove it.
    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, format!("/v1/me/signing-keys/{id}")).cookie(&bob)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from maker_keys where revoked_at is null"
        )
        .await,
        1
    );

    // While it's there, a signed file is the maker's.
    let (signed_by, problem) = release("1.0.0").await;
    assert_eq!(signed_by.as_deref(), Some("Laptop"), "{problem:?}");

    // Removed: at once, and only once.
    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, format!("/v1/me/signing-keys/{id}")).cookie(&maker)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, format!("/v1/me/signing-keys/{id}")).cookie(&maker)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    let (signed_by, problem) = release("1.0.1").await;
    assert!(signed_by.is_none(), "a removed key no longer signs");
    assert!(problem.unwrap().contains("no signing key"));

    // Added again: it waits 72 hours like a new key, and doesn't sign meanwhile.
    let res = app
        .send(add_key(&maker, "Laptop again", &public_line))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let again = body_json(res).await;
    assert_eq!(again["active"], false);
    assert_eq!(again["name"], "Laptop again");
    let wait: f64 = sqlx::query_scalar(
        "select extract(epoch from active_from - now())::float8 from maker_keys where user_id = $1",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(wait > 71.0 * 3600.0, "{wait}");
    let (signed_by, problem) = release("1.0.2").await;
    assert!(signed_by.is_none());
    assert!(problem.unwrap().contains("72-hour"));
}

// ---------------------------------------------------------------------------------------------
// Releases: notes, download links, the revocation list, keys and what the server checks
// (releases.rs)

/// A release of `slug` with one Windows file at `url` (and `parts`, if any), straight into the
/// database with this status. Returns its ID.
async fn release_with_file(
    app: &TestApp,
    slug: &str,
    version: &str,
    status: &str,
    url: &str,
    parts: Option<Value>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, signed_manifest, reviewed_at, revoked_at)
         select $1, a.id, $2, repeat('a', 40), $3::release_status, a.maker_id, '{}'::jsonb, now(),
                case when $3 = 'revoked' then now() end
         from apps a where a.slug = $4",
    )
    .bind(id)
    .bind(version)
    .bind(status)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into release_files (release_id, platform, kind, url, file_name, exec, parts)
         values ($1, 'windows', 'archive', $2, 'game.zip', 'game.exe', $3)",
    )
    .bind(id)
    .bind(url)
    .bind(parts)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

/// Makes the user's publishing passkey overdue (passkeys.rs), so publishing needs one.
async fn passkey_overdue(app: &TestApp, user: Uuid) {
    sqlx::query("update users set passkey_due_at = now() - interval '1 hour' where id = $1")
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
}

/// Notes are the maker's to change at any time; empty clears them; nobody else's.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_edit_their_release_notes(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(release_body(&app, "1.0.0", "game.zip")),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let edit = |cookie: &str, release: &str, body: Value| {
        req(Method::PATCH, format!("/v1/apps/game/releases/{release}"))
            .cookie(cookie)
            .json(body)
    };
    let notes = || async {
        sqlx::query_scalar::<_, Option<String>>("select notes from releases where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap()
    };

    let res = app
        .send(edit(&maker, &id, json!({ "notes": "  Fixed the jump.  " })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let release = body_json(res).await;
    assert_eq!(release["notes"], "Fixed the jump.");
    assert_eq!(release["id"], id.as_str());
    assert_eq!(notes().await.as_deref(), Some("Fixed the jump."));

    // Refusals leave the notes as they were.
    assert_eq!(
        status_of(&app, edit(&stranger, &id, json!({ "notes": "mine now" }))).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::PATCH, format!("/v1/apps/game/releases/{id}"))
                .json(json!({ "notes": "x" }))
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(
            &app,
            edit(&maker, &id, json!({ "notes": "x", "version": "9.9.9" }))
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        error_field(
            app.send(edit(&maker, &id, json!({ "notes": "n".repeat(5001) })))
                .await
        )
        .await,
        "notes"
    );
    assert_eq!(
        status_of(
            &app,
            edit(&maker, &Uuid::now_v7().to_string(), json!({ "notes": "x" }))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(notes().await.as_deref(), Some("Fixed the jump."));

    // Another app's release can't be edited through this one.
    let (other_id, other) = app.user("Olga Other", "US").await;
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms)
         values ($1, 'other', 'Other', 'Another game', 'game', 'beta', $2, 'https://github.com/olga/other', 'MIT', '{windows}')",
    )
    .bind(Uuid::now_v7())
    .bind(other_id)
    .execute(&app.db)
    .await
    .unwrap();
    let theirs = release_with_file(
        &app,
        "other",
        "2.0.0",
        "approved",
        "https://example.com/x.zip",
        None,
    )
    .await;
    assert_eq!(
        status_of(
            &app,
            edit(&maker, &theirs.to_string(), json!({ "notes": "x" }))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::PATCH, format!("/v1/apps/game/releases/{theirs}"))
                .cookie(&other)
                .json(json!({ "notes": "x" }))
        )
        .await,
        StatusCode::NOT_FOUND
    );

    // Empty clears them (exactly 5,000 characters is fine).
    let res = app
        .send(edit(&maker, &id, json!({ "notes": "n".repeat(5000) })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = app.send(edit(&maker, &id, json!({ "notes": "   " }))).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_json(res).await["notes"].is_null());
    assert_eq!(notes().await, None);
}

/// Bug b19: notes over the limit are refused before the release is made, so nothing is left
/// behind (and the version can be sent again with shorter notes).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn release_notes_over_the_limit_leave_no_release_behind(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let mut body = release_body(&app, "1.0.0", "game.zip");
    body["notes"] = json!("n".repeat(5001));
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(body.clone()),
        )
        .await;
    assert_eq!(error_field(res).await, "notes");
    assert_eq!(count(&app, "select count(*) from releases").await, 0);
    body["notes"] = json!("Short.");
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(body),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(body_json(res).await["notes"], "Short.");

    // The same through the GitHub picker.
    sqlx::query("update apps set repo_url = 'https://github.com/octo/game'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases/from-github")
                .cookie(&maker)
                .json(json!({
                    "tag": "v1.1.0",
                    "notes": "n".repeat(5001),
                    "files": [{
                        "platform": "windows", "kind": "archive", "exec": "game.exe",
                        "url": "https://github.com/octo/game/releases/download/v1.1.0/game-1.1.0-windows.zip"
                    }]
                })),
        )
        .await;
    assert_eq!(error_field(res).await, "notes");
    assert_eq!(
        count(
            &app,
            "select count(*) from releases where version = '1.1.0'"
        )
        .await,
        0
    );
}

/// Links are for approved releases of apps the viewer can see, part by part.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn download_links_are_for_approved_releases_the_viewer_can_see(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (friend_id, friend) = app.user("Finn Friend", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    let base = format!("{}octo/game/releases/download/v1", app.fake_base);
    let parts = json!([
        { "url": format!("{base}/game.zip.001") },
        { "url": format!("{base}/game.zip.002") }
    ]);
    let approved = release_with_file(
        &app,
        "game",
        "1.0.0",
        "approved",
        &format!("{base}/game.zip"),
        Some(parts),
    )
    .await;
    let waiting = release_with_file(
        &app,
        "game",
        "1.1.0",
        "awaiting_review",
        &format!("{base}/game.zip"),
        None,
    )
    .await;
    let revoked = release_with_file(
        &app,
        "game",
        "0.9.0",
        "revoked",
        &format!("{base}/game.zip"),
        None,
    )
    .await;
    admin::list_app(&app.db, "game", "rev").await.unwrap();
    let link = |release: Uuid, query: &str| {
        req(
            Method::GET,
            format!("/v1/apps/game/releases/{release}/download/windows{query}"),
        )
        .header("x-country", "US")
    };
    let url_of = |r: Req| async {
        let res = app.send(r).await;
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await["url"].as_str().unwrap().to_string()
    };

    // Anyone, signed in or not: the file's own URL (no GitHub token here), or a part's.
    assert_eq!(url_of(link(approved, "")).await, format!("{base}/game.zip"));
    assert_eq!(
        url_of(link(approved, "?part=2")).await,
        format!("{base}/game.zip.002")
    );
    assert_eq!(
        url_of(link(approved, "?part=0")).await,
        format!("{base}/game.zip.001"),
        "part 0 is taken as the first"
    );
    assert_eq!(
        status_of(&app, link(approved, "?part=3")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            &app,
            req(
                Method::GET,
                format!("/v1/apps/game/releases/{approved}/download/linux")
            )
            .header("x-country", "US")
        )
        .await,
        StatusCode::NOT_FOUND
    );

    // Not approved, or revoked: nothing, even for the maker.
    for id in [waiting, revoked] {
        assert_eq!(status_of(&app, link(id, "")).await, StatusCode::NOT_FOUND);
        assert_eq!(
            status_of(&app, link(id, "").cookie(&maker)).await,
            StatusCode::NOT_FOUND
        );
    }
    sqlx::query("update releases set status = 'revoked', revoked_at = now() where id = $1")
        .bind(approved)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status_of(&app, link(approved, "")).await,
        StatusCode::NOT_FOUND
    );
    sqlx::query("update releases set status = 'approved', revoked_at = null where id = $1")
        .bind(approved)
        .execute(&app.db)
        .await
        .unwrap();

    // Private Slop: only the owner and who it's shared with.
    sqlx::query("update apps set visibility = 'private'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("insert into app_shares (app_id, user_id) select id, $1 from apps")
        .bind(friend_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status_of(&app, link(approved, "")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(&app, link(approved, "").cookie(&stranger)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        url_of(link(approved, "").cookie(&friend)).await,
        format!("{base}/game.zip")
    );
    assert_eq!(
        url_of(link(approved, "").cookie(&maker)).await,
        format!("{base}/game.zip")
    );
    let _ = maker_id;
}

/// A private GitHub release, through the route: with a GitHub token, the player gets GitHub's
/// short-lived signed link, never the token.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn private_github_release_files_are_linked_through_the_route(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.releases.github_token = Some("gh-test-token".into())
    })
    .await;
    let (_, _) = github_app(&app).await;
    let id = release_with_file(
        &app,
        "game",
        "1.0.0",
        "approved",
        "https://github.com/octo/game/releases/download/v1/game.zip",
        None,
    )
    .await;
    admin::list_app(&app.db, "game", "rev").await.unwrap();
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/apps/game/releases/{id}/download/windows"),
            )
            .header("x-country", "US"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let link = body_json(res).await;
    assert_eq!(
        link["url"],
        "https://objects.example/signed?token=short-lived"
    );
    assert!(!link.to_string().contains("gh-test-token"));
}

/// The kill switch: signed, oldest revocation first, valid for six hours, and signed even when
/// empty; a server without a signing key says it can't.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_revocation_list_is_signed_in_order_and_expires_in_six_hours(db: PgPool) {
    let app = test_app(db).await;
    let (_, _) = maker_with_app(&app, "released").await;
    let read = || async {
        let res = app.send(req(Method::GET, "/v1/revocations")).await;
        assert_eq!(res.status(), StatusCode::OK);
        let signed: slop_manifest::Signed = serde_json::from_value(body_json(res).await).unwrap();
        let list: slop_manifest::RevocationList = slop_manifest::verify(
            &signed,
            slop_manifest::Purpose::RevocationList,
            &[app.signing_key.verifying_key()],
        )
        .unwrap();
        list
    };
    let empty = read().await;
    assert!(empty.revoked.is_empty());
    let issued = OffsetDateTime::parse(&empty.issued_at, &Rfc3339).unwrap();
    let expires = OffsetDateTime::parse(&empty.expires_at, &Rfc3339).unwrap();
    assert_eq!(expires - issued, time::Duration::hours(6));
    assert!((OffsetDateTime::now_utc() - issued).abs() < time::Duration::minutes(1));

    // Two revocations, the older one made second; an approved release isn't listed.
    let newer = release_with_file(
        &app,
        "game",
        "1.0.0",
        "revoked",
        "https://example.com/a.zip",
        None,
    )
    .await;
    let older = release_with_file(
        &app,
        "game",
        "0.9.0",
        "revoked",
        "https://example.com/b.zip",
        None,
    )
    .await;
    release_with_file(
        &app,
        "game",
        "1.1.0",
        "approved",
        "https://example.com/c.zip",
        None,
    )
    .await;
    sqlx::query("update releases set revoked_at = now() - interval '2 days', status_reason = 'Malware' where id = $1")
        .bind(older)
        .execute(&app.db)
        .await
        .unwrap();
    let list = read().await;
    let ids: Vec<&str> = list.revoked.iter().map(|r| r.release_id.as_str()).collect();
    assert_eq!(ids, [older.to_string(), newer.to_string()]);
    assert_eq!(list.revoked[0].app_slug, "game");
    assert_eq!(list.revoked[0].reason, "Malware");
    assert_eq!(list.revoked[1].reason, "");

    // No signing key: unavailable, not an unsigned list.
    let unsigned = test_app_with(app.db.clone(), |c| c.releases.signing_key = None).await;
    let res = unsigned.send(req(Method::GET, "/v1/revocations")).await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body_json(res).await["error"]["code"], "unavailable");
}

/// Slop's public key, for transparency, and what the server checks, so the UI claims only that.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_server_publishes_its_key_and_what_it_checks(db: PgPool) {
    let app = test_app(db.clone()).await;
    let keys = body_json(app.send(req(Method::GET, "/v1/keys")).await).await;
    let key = app.signing_key.verifying_key();
    assert_eq!(
        keys,
        json!([{
            "key_id": slop_manifest::key_id(&key),
            "public_key": slop_manifest::verifying_key_to_base64(&key),
        }])
    );
    assert_eq!(
        slop_manifest::verifying_key_from_base64(keys[0]["public_key"].as_str().unwrap()),
        Some(key)
    );
    let info = body_json(app.send(req(Method::GET, "/v1/server")).await).await;
    assert_eq!(
        info,
        json!({ "scanning": false, "maker_signatures_required": false, "simulation": false })
    );

    let other = test_app_with(db, |c| {
        c.releases.signing_key = None;
        c.releases.scanner = slop_api::scan::Scanner::Local("/bin/true");
        c.releases.require_maker_signature = true;
        c.simulation = true;
    })
    .await;
    let res = other.send(req(Method::GET, "/v1/keys")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await, json!([]));
    let info = body_json(other.send(req(Method::GET, "/v1/server")).await).await;
    assert_eq!(
        info,
        json!({ "scanning": true, "maker_signatures_required": true, "simulation": true })
    );
}

// ---------------------------------------------------------------------------------------------
// Repository tokens for private repositories outside GitHub (repo_tokens.rs)

/// What a fake forge was asked: (path and query, Authorization header).
type Seen = std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>;

/// A self-hosted forge holding `ann/<repo>`, answering as Gitea ("token good" or "token
/// better"), GitLab or Bitbucket ("Bearer good"). Its release files: `ann/game` redirects to file
/// storage on another host name, `ann/same` to itself, and `ann/direct` serves the file.
async fn fake_forge() -> (String, Seen) {
    let seen: Seen = Default::default();
    let log = seen.clone();
    let router = Router::new().fallback(
        move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
            let log = log.clone();
            async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                log.lock().unwrap().push((uri.to_string(), auth.clone()));
                let port = headers["host"]
                    .to_str()
                    .unwrap()
                    .rsplit(':')
                    .next()
                    .unwrap()
                    .to_string();
                let path = uri.path();
                let gitea = auth == "token good" || auth == "token better";
                let bearer = auth == "Bearer good";
                let json = |v: Value| axum::Json(v).into_response();
                let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
                match parts.as_slice() {
                    ["storage", ..] => BUILD_BYTES.into_response(),
                    _ if !gitea && !bearer => StatusCode::UNAUTHORIZED.into_response(),
                    ["api", "v1", "repos", "ann", _] => json(json!({ "private": true })),
                    ["api", "v1", "repos", "ann", _, "contents"] => json(json!([
                        { "name": "slop-verify.txt", "type": "file" },
                        { "name": "LICENSE", "type": "file" },
                        { "name": "src", "type": "dir" }
                    ])),
                    ["api", "v1", "repos", "ann", _, "raw", "slop-verify.txt"] => {
                        "gitea-proof".into_response()
                    }
                    ["api", "v4", "projects", "ann%2Fgame"] => {
                        json(json!({ "default_branch": "trunk" }))
                    }
                    ["api", "v4", "projects", "ann%2Fgame", "repository", "tree"] => json(json!([
                        { "name": "README.md", "type": "blob" },
                        { "name": "docs", "type": "tree" }
                    ])),
                    [
                        "api",
                        "v4",
                        "projects",
                        "ann%2Fgame",
                        "repository",
                        "files",
                        "README.md",
                        "raw",
                    ] => "# Game\n\ngitlab-proof".into_response(),
                    ["bitbucket-api", "2.0", "repositories", "ann", "game"] => {
                        json(json!({ "mainbranch": { "name": "main" } }))
                    }
                    [
                        "bitbucket-api",
                        "2.0",
                        "repositories",
                        "ann",
                        "game",
                        "src",
                        "main",
                        "",
                    ] => json(json!({ "values": [
                            { "path": "README.md", "type": "commit_file" },
                            { "path": "src", "type": "commit_directory" }
                        ] })),
                    [
                        "bitbucket-api",
                        "2.0",
                        "repositories",
                        "ann",
                        "game",
                        "src",
                        "main",
                        "README.md",
                    ] => "bitbucket-proof".into_response(),
                    ["ann", "game", "releases", "download", ..] => axum::response::Redirect::to(
                        &format!("http://localhost:{port}/storage/game.zip?expires=soon"),
                    )
                    .into_response(),
                    ["ann", "same", "releases", "download", ..] => axum::response::Redirect::to(
                        &format!("http://127.0.0.1:{port}/storage/game.zip"),
                    )
                    .into_response(),
                    ["ann", "direct", "releases", "download", ..] => BUILD_BYTES.into_response(),
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }
        },
    );
    (serve_fake(router).await, seen)
}

/// An app on the forge's `ann/<repo>`, made by `maker`.
async fn forge_app(app: &TestApp, maker: Uuid, forge: &str, repo: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms)
         values ($1, 'game', 'Game', 'A game', 'game', 'beta', $2, $3, 'MIT', '{windows}')",
    )
    .bind(id)
    .bind(maker)
    .bind(format!("{forge}ann/{repo}"))
    .execute(&app.db)
    .await
    .unwrap();
    id
}

fn put_token(cookie: &str, kind: &str, token: &str) -> Req {
    req(Method::PUT, "/v1/apps/game/repo-token")
        .cookie(cookie)
        .json(json!({ "kind": kind, "token": token }))
}

/// Saving checks the token reads the repository, keeps it sealed, audits it, and replaces the
/// last one; only the maker saves, reads or forgets it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_save_check_and_forget_a_read_only_repository_token(db: PgPool) {
    let app = test_app(db).await;
    let (forge, seen) = fake_forge().await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    let app_id = forge_app(&app, ann_id, &forge, "game").await;
    let status = |cookie: &str| {
        let r = req(Method::GET, "/v1/apps/game/repo-token").cookie(cookie);
        async { body_json(app.send(r).await).await }
    };

    let before = status(&ann).await;
    assert_eq!(before["saved"], false);
    assert_eq!(before["available"], true);
    assert_eq!(before["github"], false);
    assert!(before["kind"].is_null());

    // Bad tokens: refused before anything is kept.
    for (kind, token, says) in [
        ("gitea", "bad", "refused the token"),
        ("gitea", "", "on its own"),
        ("gitea", "to ken", "on its own"),
        ("gitea", &"t".repeat(501), "on its own"),
    ] {
        let res = app.send(put_token(&ann, kind, token)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{token}");
        let err = body_json(res).await;
        assert_eq!(err["error"]["field"], "repo_token");
        assert!(
            err["error"]["message"].as_str().unwrap().contains(says),
            "{err}"
        );
    }
    assert_eq!(
        status_of(&app, put_token(&ann, "svn", "good")).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(count(&app, "select count(*) from repo_tokens").await, 0);

    // Someone else: as if there were no app.
    assert_eq!(
        status_of(&app, put_token(&stranger, "gitea", "good")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::GET, "/v1/apps/game/repo-token").cookie(&stranger)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::PUT, "/v1/apps/game/repo-token")
                .json(json!({ "kind": "gitea", "token": "good" }))
        )
        .await,
        StatusCode::UNAUTHORIZED
    );

    // A good one (with spaces around it) is kept sealed, audited, and never shown.
    let res = app.send(put_token(&ann, "gitea", "  good  ")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let saved = body_json(res).await;
    assert_eq!(saved["saved"], true);
    assert_eq!(saved["kind"], "gitea");
    assert!(saved["saved_at"].is_string());
    assert!(!saved.to_string().contains("good"));
    let (sealed, created_by): (Vec<u8>, Uuid) =
        sqlx::query_as("select sealed, created_by from repo_tokens where app_id = $1")
            .bind(app_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(!sealed.windows(4).any(|w| w == b"good"));
    assert_eq!(created_by, ann_id);
    let audited: Vec<(String, String, Value)> = sqlx::query_as(
        "select actor, subject, detail from audit_log where action = 'repo_token.save'",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        audited,
        [(
            ann_id.to_string(),
            "game".to_string(),
            json!({ "kind": "gitea" })
        )]
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(path, auth)| path == "/api/v1/repos/ann/game" && auth == "token good"),
        "checked against the host's API"
    );

    // Used once, then replaced: the new token, and no last use yet.
    let file = format!("{forge}ann/game/releases/download/v1/game.zip");
    assert_eq!(
        slop_api::repo_tokens::auth_for(&app.state, app_id, &file)
            .await
            .unwrap()
            .as_deref(),
        Some("token good")
    );
    assert!(status(&ann).await["last_used_at"].is_string());
    let res = app.send(put_token(&ann, "gitea", "better")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_json(res).await["last_used_at"].is_null());
    assert_eq!(count(&app, "select count(*) from repo_tokens").await, 1);
    assert_eq!(
        slop_api::repo_tokens::auth_for(&app.state, app_id, &file)
            .await
            .unwrap()
            .as_deref(),
        Some("token better")
    );

    // A stranger can't forget it; the maker can.
    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, "/v1/apps/game/repo-token").cookie(&stranger)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(count(&app, "select count(*) from repo_tokens").await, 1);
    assert_eq!(
        status_of(
            &app,
            req(Method::DELETE, "/v1/apps/game/repo-token").cookie(&ann)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(count(&app, "select count(*) from repo_tokens").await, 0);
    assert_eq!(status(&ann).await["saved"], false);
}

/// GitHub repositories use the App instead; a server without a key can't keep tokens; a token
/// for the app's old repository doesn't count.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn repository_tokens_are_only_for_the_current_repository_outside_github(db: PgPool) {
    let app = test_app(db.clone()).await;
    let (forge, _) = fake_forge().await;
    let (ann_id, ann) = github_app(&app).await;
    let status = body_json(
        app.send(req(Method::GET, "/v1/apps/game/repo-token").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(status["github"], true);
    assert_eq!(status["saved"], false);
    let res = app.send(put_token(&ann, "gitea", "good")).await;
    assert_eq!(error_field(res).await, "repo_token");

    // Moved to the forge, with a token; then moved again: the old token isn't used or shown.
    sqlx::query("update apps set repo_url = $1")
        .bind(format!("{forge}ann/game"))
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status_of(&app, put_token(&ann, "gitea", "good")).await,
        StatusCode::OK
    );
    let app_id: Uuid = sqlx::query_scalar("select id from apps")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query("update apps set repo_url = $1")
        .bind(format!("{forge}ann/direct"))
        .execute(&app.db)
        .await
        .unwrap();
    let status = body_json(
        app.send(req(Method::GET, "/v1/apps/game/repo-token").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(status["saved"], false);
    assert_eq!(status["github"], false);
    assert_eq!(
        slop_api::repo_tokens::auth_for(&app.state, app_id, &format!("{forge}ann/direct/x.zip"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        slop_api::repo_tokens::still_works(&app.state, app_id).await,
        None
    );
    let _ = ann_id;

    // No key on this server.
    let keyless = test_app_with(db, |c| c.releases.repo_token_key = None).await;
    let status = body_json(
        keyless
            .send(req(Method::GET, "/v1/apps/game/repo-token").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(status["available"], false);
    let res = keyless.send(put_token(&ann, "gitea", "good")).await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        slop_api::repo_tokens::auth_for(
            &keyless.state,
            app_id,
            &format!("{forge}ann/direct/x.zip")
        )
        .await
        .unwrap(),
        None
    );
}

/// The token goes only to the repository's own scheme, host and port; players get the host's
/// redirect to somewhere else, never the file relayed or a redirect back to the same host.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn repository_tokens_are_only_sent_to_the_repositorys_own_host(db: PgPool) {
    let app = test_app(db).await;
    let (forge, seen) = fake_forge().await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let app_id = forge_app(&app, ann_id, &forge, "game").await;
    assert_eq!(
        status_of(&app, put_token(&ann, "gitea", "good")).await,
        StatusCode::OK
    );
    let port = forge
        .trim_end_matches('/')
        .rsplit(':')
        .next()
        .unwrap()
        .to_string();
    let auth = |url: String| {
        let state = app.state.clone();
        async move { slop_api::repo_tokens::auth_for(&state, app_id, &url).await }
    };
    assert_eq!(
        auth(format!("{forge}ann/game/releases/download/v1/game.zip"))
            .await
            .unwrap()
            .as_deref(),
        Some("token good")
    );
    for elsewhere in [
        format!("http://localhost:{port}/ann/game/releases/download/v1/game.zip"),
        format!("http://127.0.0.1:1/ann/game/releases/download/v1/game.zip"),
        format!("https://127.0.0.1:{port}/ann/game/releases/download/v1/game.zip"),
        "https://evil.example/ann/game".to_string(),
        "not a url".to_string(),
    ] {
        assert_eq!(auth(elsewhere.clone()).await.unwrap(), None, "{elsewhere}");
    }

    // The host's redirect to other storage is a fine link, and the token isn't sent there.
    let header = "token good";
    let link = slop_api::repo_tokens::short_lived_link(
        &app.state,
        &format!("{forge}ann/game/releases/download/v1/game.zip"),
        header,
    )
    .await
    .unwrap();
    assert_eq!(
        link,
        format!("http://localhost:{port}/storage/game.zip?expires=soon")
    );
    // A redirect to the same host, or the file itself, isn't.
    for repo in ["same", "direct"] {
        let err = slop_api::repo_tokens::short_lived_link(
            &app.state,
            &format!("{forge}ann/{repo}/releases/download/v1/game.zip"),
            header,
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("serves private files directly"),
            "{repo}: {err}"
        );
    }
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(path, auth)| !path.starts_with("/storage") || auth.is_empty()),
        "the token never reaches file storage"
    );

    // Through the route: a host that serves the file directly can't be shared (409).
    sqlx::query("update apps set repo_url = $1, visibility = 'private'")
        .bind(format!("{forge}ann/direct"))
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status_of(&app, put_token(&ann, "gitea", "good")).await,
        StatusCode::OK
    );
    let id = release_with_file(
        &app,
        "game",
        "1.0.0",
        "approved",
        &format!("{forge}ann/direct/releases/download/v1/game.zip"),
        None,
    )
    .await;
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/apps/game/releases/{id}/download/windows"),
            )
            .cookie(&ann),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("never relays files"));
}

/// The README or slop-verify.txt is read through each host's API with the token, and a token
/// that stops working is noticed.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn ownership_codes_are_read_through_gitea_gitlab_and_bitbucket_apis(db: PgPool) {
    use slop_api::repo_tokens::{Given, Kind, read_proof};
    let app = test_app(db).await;
    let (forge, seen) = fake_forge().await;
    let repo = format!("{forge}ann/game");
    let given = |kind: Kind, token: &str| Given {
        kind,
        token: token.into(),
    };

    let text = read_proof(&app.state, &repo, &given(Kind::Gitea, "good"))
        .await
        .unwrap()
        .unwrap();
    assert!(text.contains("gitea-proof"), "{text}");
    let text = read_proof(&app.state, &repo, &given(Kind::Gitlab, "good"))
        .await
        .unwrap()
        .unwrap();
    assert!(text.contains("gitlab-proof"), "{text}");
    let text = read_proof(&app.state, &repo, &given(Kind::Bitbucket, "good"))
        .await
        .unwrap()
        .unwrap();
    assert!(text.contains("bitbucket-proof"), "{text}");
    {
        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|(p, a)| p
            == "/api/v4/projects/ann%2Fgame/repository/tree?per_page=100&ref=trunk"
            && a == "Bearer good"));
        assert!(seen.iter().any(|(p, a)| p
            == "/bitbucket-api/2.0/repositories/ann/game/src/main/?pagelen=100"
            && a == "Bearer good"));
        assert!(
            !seen.iter().any(|(p, _)| p.contains("LICENSE")),
            "only the proof files are read"
        );
    }
    let err = read_proof(&app.state, &repo, &given(Kind::Gitea, "bad"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("401"), "{err}");

    // still_works: yes with the saved token, no once it stops reading the repository.
    let (ann_id, ann) = app.user("Ann", "US").await;
    let app_id = forge_app(&app, ann_id, &forge, "game").await;
    assert_eq!(
        slop_api::repo_tokens::still_works(&app.state, app_id).await,
        None
    );
    assert_eq!(
        status_of(&app, put_token(&ann, "gitlab", "good")).await,
        StatusCode::OK
    );
    assert_eq!(
        slop_api::repo_tokens::still_works(&app.state, app_id).await,
        Some(true)
    );
    let mut conn = app.db.acquire().await.unwrap();
    slop_api::repo_tokens::store(
        &mut conn,
        &[9; 32],
        app_id,
        &repo,
        &given(Kind::Gitlab, "revoked"),
        ann_id,
    )
    .await
    .unwrap();
    assert_eq!(
        slop_api::repo_tokens::still_works(&app.state, app_id).await,
        Some(false)
    );
}

// ---------------------------------------------------------------------------------------------
// Passkeys for what decides where players' downloads come from (bug b23)

/// The user has a passkey and confirmed with it in this session.
async fn passkey_proved(app: &TestApp, user: Uuid) {
    sqlx::query(
        "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, $3, '{}', 'Laptop')",
    )
    .bind(Uuid::now_v7())
    .bind(user)
    .bind(Uuid::now_v7().as_bytes().to_vec())
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update sessions set passkey_at = now() where user_id = $1")
        .bind(user)
        .execute(&app.db)
        .await
        .unwrap();
}

/// Bug b23: like publishing, moving an app to another repository, saving the token Slop reads
/// it with, and turning on release watching or automatic updates need a passkey once the
/// maker's grace period is over.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn moving_the_repository_and_its_token_and_update_settings_need_a_passkey(db: PgPool) {
    let app = test_app(db).await;
    let (forge, _) = fake_forge().await;
    let (maker_id, maker) = app.user("Ann", "US").await;
    let app_id = forge_app(&app, maker_id, &forge, "game").await;
    passkey_overdue(&app, maker_id).await;
    let elsewhere = "https://github.com/octo/other-game";
    let refusals = [
        request_change(
            &app,
            &maker,
            json!({ "kind": "repo", "repo_url": elsewhere }),
        )
        .await,
        app.send(put_token(&maker, "gitea", "good")).await,
        app.send(
            req(Method::PUT, "/v1/apps/game/update-settings")
                .cookie(&maker)
                .json(json!({ "watch_releases": true, "auto_update": true })),
        )
        .await,
    ];
    for res in refusals {
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(body_json(res).await["error"]["code"], "passkey_required");
    }
    let (repo, watch): (String, bool) =
        sqlx::query_as("select repo_url, watch_releases from apps where id = $1")
            .bind(app_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(repo, format!("{forge}ann/game"));
    assert!(watch, "watching stays as it was (on by default)");
    assert_eq!(count(&app, "select count(*) from app_changes").await, 0);
    assert_eq!(count(&app, "select count(*) from repo_tokens").await, 0);

    // Confirmed with a passkey: all three go through.
    passkey_proved(&app, maker_id).await;
    assert_eq!(
        status_of(&app, put_token(&maker, "gitea", "good")).await,
        StatusCode::OK
    );
    let res = app
        .send(
            req(Method::PUT, "/v1/apps/game/update-settings")
                .cookie(&maker)
                .json(json!({ "watch_releases": false })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = request_change(
        &app,
        &maker,
        json!({ "kind": "repo", "repo_url": elsewhere }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(body_json(res).await["status"], "approved");
}

// ---------------------------------------------------------------------------------------------
// Update settings and change requests (updates.rs, manage.rs)

/// Agent tokens (maker_agents.rs) for the maker signed in with `cookie`. Returns the
/// Authorization header value.
async fn agent_token(app: &TestApp, cookie: &str, scope: &str, apps: Option<&[&str]>) -> String {
    let mut body = json!({ "name": "Agent", "scope": scope });
    if let Some(apps) = apps {
        body["apps"] = json!(apps);
    }
    let res = app
        .send(
            req(Method::POST, "/v1/me/maker-tokens")
                .cookie(cookie)
                .json(body),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    format!(
        "Bearer {}",
        body_json(res).await["secret"].as_str().unwrap()
    )
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_switch_release_watching_and_automatic_updates(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = github_app(&app).await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    let put = |cookie: &str, body: Value| {
        req(Method::PUT, "/v1/apps/game/update-settings")
            .cookie(cookie)
            .json(body)
    };
    let stored = || async {
        sqlx::query_as::<_, (bool, bool)>("select watch_releases, auto_update from apps")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    let before = stored().await;

    let res = app
        .send(put(&maker, json!({ "watch_releases": !before.0 })))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let settings = body_json(res).await;
    assert_eq!(settings["watch_releases"], !before.0);
    assert_eq!(settings["auto_update"], before.1, "left out: unchanged");
    assert_eq!(stored().await, (!before.0, before.1));
    let res = app
        .send(put(&maker, json!({ "auto_update": !before.1 })))
        .await;
    assert_eq!(body_json(res).await["auto_update"], !before.1);
    assert_eq!(stored().await, (!before.0, !before.1));

    // Refusals change nothing.
    assert_eq!(
        status_of(&app, put(&stranger, json!({ "watch_releases": before.0 }))).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            &app,
            put(&maker, json!({ "watch_releases": before.0, "colour": 1 }))
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let agent = agent_token(&app, &maker, "publish", None).await;
    let res = app
        .send(
            req(Method::PUT, "/v1/apps/game/update-settings")
                .header("authorization", agent.clone())
                .json(json!({ "watch_releases": before.0 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "agents only read them");
    assert_eq!(
        status_of(
            &app,
            req(Method::GET, "/v1/apps/game/update-settings").header("authorization", agent)
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(stored().await, (!before.0, !before.1));
}

/// A maker cancels their own pending request; a decided one, someone else's, or another app's
/// can't be.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_cancel_only_their_own_pending_changes(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    approved_release(&app, "game", maker_id).await;
    let res = request_change(
        &app,
        &maker,
        json!({ "kind": "repo", "repo_url": "https://github.com/octo/game-two" }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let pending = body_json(res).await;
    assert_eq!(pending["status"], "pending");
    let id = pending["id"].as_str().unwrap().to_string();
    let cancel = |cookie: &str, slug: &str, id: &str| {
        req(Method::POST, format!("/v1/apps/{slug}/changes/{id}/cancel")).cookie(cookie)
    };

    // Another maker's app: the id isn't one of its changes.
    let (other_id, other) = app.user("Olga Other", "US").await;
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms)
         values ($1, 'other', 'Other', 'Another game', 'game', 'beta', $2, 'https://github.com/olga/other', 'MIT', '{windows}')",
    )
    .bind(Uuid::now_v7())
    .bind(other_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        status_of(&app, cancel(&stranger, "game", &id)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(&app, cancel(&other, "other", &id)).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        status_of(&app, cancel(&maker, "game", &Uuid::now_v7().to_string())).await,
        StatusCode::CONFLICT
    );
    let status: String = sqlx::query_scalar("select status from app_changes")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "pending");

    let res = app.send(cancel(&maker, "game", &id)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let cancelled = body_json(res).await;
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["status_reason"], "Cancelled by the maker.");
    let res = app.send(cancel(&maker, "game", &id)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("isn't waiting"));
}

// ---------------------------------------------------------------------------------------------
// Admin review of releases and changes (admin_web.rs, admin.rs, manage.rs)

/// Staff with only these roles.
async fn staff_with(app: &TestApp, name: &str, roles: &[&str]) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    for role in roles {
        sqlx::query("insert into staff_roles (user_id, role, granted_by) values ($1, $2, 'test')")
            .bind(id)
            .bind(role)
            .execute(&app.db)
            .await
            .unwrap();
    }
    (id, cookie)
}

/// Rejecting a release: reviewers only, with a reason the maker hears, never an approved one;
/// and the next release waits for a person again.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reviewers_reject_releases_with_a_reason_the_maker_hears(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (_, reviewer) = staff_with(&app, "Rita Reviewer", &["reviewer"]).await;
    let (_, support) = staff_with(&app, "Sue Support", &["support"]).await;
    let first = submit_and_fetch(&app, &maker, "1.0.0").await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        first.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let second = submit_and_fetch(&app, &maker, "1.1.0").await;
    // With a person's approval behind it, the second was approved on its own; the third waits.
    assert_eq!(release_status(&app, &second).await.0, "approved");
    sqlx::query("update apps set auto_update = false")
        .execute(&app.db)
        .await
        .unwrap();
    let third = submit_and_fetch(&app, &maker, "1.2.0").await;
    assert_eq!(release_status(&app, &third).await.0, "awaiting_review");
    sqlx::query("update apps set auto_update = true")
        .execute(&app.db)
        .await
        .unwrap();
    let reject = |cookie: &str, id: &str, reason: &str| {
        req(Method::POST, format!("/v1/admin/releases/{id}/reject"))
            .cookie(cookie)
            .json(json!({ "reason": reason }))
    };

    assert_eq!(
        status_of(&app, reject(&maker, &third, "No.")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::POST, format!("/v1/admin/releases/{third}/reject"))
                .json(json!({ "reason": "No." }))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(&app, reject(&support, &third, "No.")).await,
        StatusCode::FORBIDDEN
    );
    let res = app.send(reject(&reviewer, &third, "   ")).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("reason"));
    let res = app.send(reject(&reviewer, &second, "Too late.")).await;
    assert_eq!(
        res.status(),
        StatusCode::CONFLICT,
        "an approved release isn't rejected"
    );
    assert_eq!(release_status(&app, &second).await.0, "approved");
    assert_eq!(release_status(&app, &third).await.0, "awaiting_review");

    let res = app
        .send(reject(
            &reviewer,
            &third,
            "  The Windows build crashes on start.  ",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (status, reason, by) = release_status(&app, &third).await;
    assert_eq!(status, "rejected");
    assert_eq!(
        reason.as_deref(),
        Some("The Windows build crashes on start.")
    );
    assert!(by.unwrap().starts_with("Rita Reviewer ("));
    let audited: Vec<(String, String)> =
        sqlx::query_as("select actor, subject from audit_log where action = 'release.reject'")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].1, third);
    assert!(audited[0].0.starts_with("Rita Reviewer"));
    let told: Vec<(String, String)> = sqlx::query_as(
        "select title, body from notifications where user_id = $1 and title like '%wasn''t approved'",
    )
    .bind(maker_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        told,
        [(
            "Game 1.2.0 wasn't approved".to_string(),
            "The Windows build crashes on start.".to_string()
        )]
    );
    assert_eq!(
        status_of(&app, reject(&reviewer, &third, "Again.")).await,
        StatusCode::CONFLICT,
        "already decided"
    );

    // The next release waits for a person, saying why.
    let fourth = submit_and_fetch(&app, &maker, "1.3.0").await;
    let (status, reason, _) = release_status(&app, &fourth).await;
    assert_eq!(status, "awaiting_review");
    assert!(reason.unwrap().contains("withdrawn or rejected"));
}

/// Clearing a scan finding as a false positive: reviewers only, with a reason, on a release
/// still waiting for review.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reviewers_clear_false_positive_scan_findings(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let (_, reviewer) = staff_with(&app, "Rita Reviewer", &["reviewer"]).await;
    let (_, support) = staff_with(&app, "Sue Support", &["support"]).await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;
    sqlx::query(
        "update release_files set scan_result = 'on_hold', scan_detail = 'Heuristic.Packer'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let clear = |cookie: &str, platform: &str, reason: &str| {
        req(
            Method::POST,
            format!("/v1/admin/releases/{id}/files/{platform}/clear"),
        )
        .cookie(cookie)
        .json(json!({ "reason": reason }))
    };
    let reason = "UPX-packed Godot export; checked the unpacked binary.";
    assert_eq!(
        status_of(&app, clear(&maker, "windows", reason)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(&app, clear(&support, "windows", reason)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status_of(&app, clear(&reviewer, "windows", "fine")).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        status_of(&app, clear(&reviewer, "linux", reason)).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from release_files where cleared_by is not null"
        )
        .await,
        0
    );

    assert_eq!(
        status_of(&app, clear(&reviewer, "windows", reason)).await,
        StatusCode::NO_CONTENT
    );
    let (by, why): (String, String) =
        sqlx::query_as("select cleared_by, cleared_reason from release_files")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(by.starts_with("Rita Reviewer"));
    assert_eq!(why, reason);
    assert_eq!(
        count(
            &app,
            "select count(*) from audit_log where action = 'release.clear_scan'"
        )
        .await,
        1
    );

    // Once decided, there's nothing to clear.
    sqlx::query("update releases set status = 'rejected'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(clear(&reviewer, "windows", reason)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("waiting for review"));
}

/// Approving a maker's change: reviewers only, once; a hold at a release withdrawn since the
/// request is refused; a repository move starts the app afresh at the new repository.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reviewers_approve_changes_that_still_make_sense(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (_, reviewer) = staff_with(&app, "Rita Reviewer", &["reviewer"]).await;
    let first = submit_and_fetch(&app, &maker, "1.0.0").await;
    admin::approve(
        &app.db,
        Some(&app.signing_key),
        lax(),
        first.parse().unwrap(),
        "rev",
    )
    .await
    .unwrap();
    let second = submit_and_fetch(&app, &maker, "1.1.0").await;
    assert_eq!(release_status(&app, &second).await.0, "approved");
    sqlx::query("update apps set auto_update = false")
        .execute(&app.db)
        .await
        .unwrap();
    let approve = |cookie: &str, id: &str| {
        req(Method::POST, format!("/v1/admin/changes/{id}/approve")).cookie(cookie)
    };

    // Hold at 1.0.0: waits; 1.0.0 is withdrawn meanwhile, so it can't be approved.
    let res = request_change(
        &app,
        &maker,
        json!({ "kind": "release", "release_id": first }),
    )
    .await;
    let hold = body_json(res).await;
    assert_eq!(hold["status"], "pending");
    let hold_id = hold["id"].as_str().unwrap().to_string();
    assert_eq!(
        status_of(&app, approve(&maker, &hold_id)).await,
        StatusCode::NOT_FOUND
    );
    admin::revoke(&app.db, first.parse().unwrap(), "rev", "Malware found")
        .await
        .unwrap();
    let res = app.send(approve(&reviewer, &hold_id)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("withdrawn"));
    let pinned: Option<Uuid> = sqlx::query_scalar("select pinned_release_id from apps")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(pinned.is_none());

    // A move: watch state and the old repository's IDs are dropped.
    sqlx::query(
        "update apps set watch_last_tag = 'v1.1.0', watch_problem = 'odd', watch_checked_at = now(),
                github_repo_id = 1001, github_owner_id = 42",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let res = request_change(
        &app,
        &maker,
        json!({ "kind": "repo", "repo_url": "https://github.com/octo/game-two", "note": "renamed" }),
    )
    .await;
    let mv = body_json(res).await;
    assert_eq!(mv["status"], "pending");
    let move_id = mv["id"].as_str().unwrap().to_string();
    assert_eq!(
        status_of(&app, approve(&reviewer, &move_id)).await,
        StatusCode::NO_CONTENT
    );
    type AppRow = (
        String,
        Option<String>,
        Option<String>,
        Option<OffsetDateTime>,
        Option<i64>,
        Option<i64>,
        Option<OffsetDateTime>,
    );
    let row: AppRow = sqlx::query_as(
        "select repo_url, watch_last_tag, watch_problem, watch_checked_at, github_repo_id, github_owner_id, repo_changed_at from apps",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(row.0, "https://github.com/octo/game-two");
    assert!(row.1.is_none() && row.2.is_none() && row.3.is_none());
    assert!(
        row.4.is_none() && row.5.is_none(),
        "the old repository's IDs are forgotten"
    );
    assert!(row.6.is_some());
    let (status, by): (String, String) =
        sqlx::query_as("select status, reviewed_by from app_changes where id = $1::uuid")
            .bind(&move_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "approved");
    assert!(by.starts_with("Rita Reviewer"));
    assert_eq!(
        count(
            &app,
            "select count(*) from audit_log where action = 'app.repo_change'"
        )
        .await,
        1
    );
    let res = app.send(approve(&reviewer, &move_id)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "already decided");
    let _ = maker_id;
}

/// Turning a change down: reviewers only, with a reason, once.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reviewers_turn_down_changes_with_a_reason(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let (_, reviewer) = staff_with(&app, "Rita Reviewer", &["reviewer"]).await;
    let (_, support) = staff_with(&app, "Sue Support", &["support"]).await;
    approved_release(&app, "game", maker_id).await;
    let res = request_change(
        &app,
        &maker,
        json!({ "kind": "repo", "repo_url": "https://github.com/octo/game-two" }),
    )
    .await;
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    let reject = |cookie: &str, reason: &str| {
        req(Method::POST, format!("/v1/admin/changes/{id}/reject"))
            .cookie(cookie)
            .json(json!({ "reason": reason }))
    };
    assert_eq!(
        status_of(&app, reject(&maker, "No.")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(&app, reject(&support, "No.")).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status_of(&app, reject(&reviewer, " ")).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::POST, format!("/v1/admin/changes/{id}/reject"))
                .cookie(&reviewer)
                .json(json!({ "reason": "x", "extra": 1 }))
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let status: String = sqlx::query_scalar("select status from app_changes")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "pending");

    assert_eq!(
        status_of(&app, reject(&reviewer, "Which repository is really yours?")).await,
        StatusCode::NO_CONTENT
    );
    let (status, reason, repo): (String, String, String) = sqlx::query_as(
        "select c.status, c.status_reason, a.repo_url from app_changes c join apps a on a.id = c.app_id",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(status, "rejected");
    assert_eq!(reason, "Which repository is really yours?");
    assert!(repo.ends_with("octo/game"), "the app stays where it was");
    assert_eq!(
        count(
            &app,
            "select count(*) from audit_log where action = 'app.change_reject'"
        )
        .await,
        1
    );
    assert_eq!(
        status_of(&app, reject(&reviewer, "Again.")).await,
        StatusCode::CONFLICT
    );
    // The maker sees why on the Manage page.
    let manage = body_json(
        app.send(req(Method::GET, "/v1/apps/game/manage").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(
        manage["changes"][0]["status_reason"],
        "Which repository is really yours?"
    );
}

/// Bug b21: the "was submitted" notice tells the maker to withdraw an unexpected release from
/// Manage, but makers have no way to withdraw a release.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
#[ignore = "b21: the submitted-release notice says to withdraw it from Manage, which makers can't do"]
async fn the_submitted_notice_only_suggests_what_makers_can_do(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "released").await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;
    let body: String = sqlx::query_scalar(
        "select body from notifications where user_id = $1 and title = 'Game 1.0.0 was submitted'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    if body.contains("withdraw it from Manage") {
        // Then the maker must be able to withdraw it.
        let res = app
            .send(
                req(
                    Method::POST,
                    format!("/v1/apps/game/releases/{id}/withdraw"),
                )
                .cookie(&maker),
            )
            .await;
        assert!(res.status().is_success(), "{body}");
    }
}

// ---------------------------------------------------------------------------------------------
// A GitHub whose answers each test chooses

/// One answer: status, extra headers, body.
#[derive(Clone)]
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

/// A stand-in for GitHub's API (and its OIDC keys) that answers each path the way the test set
/// it, 404 otherwise, and remembers what it was asked.
#[derive(Clone, Default)]
struct FakeGh {
    seen: Seen,
    answers: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Answer>>>,
}

impl FakeGh {
    fn set(&self, path: &str, status: u16, headers: &[(&str, &str)], body: impl Into<String>) {
        self.answers.lock().unwrap().insert(
            path.to_string(),
            Answer {
                status,
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body: body.into(),
            },
        );
    }

    fn json(&self, path: &str, status: u16, body: Value) {
        self.set(path, status, &[], body.to_string());
    }

    /// How many requests were for this path (ignoring the query).
    fn hits(&self, path: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.split('?').next() == Some(path))
            .count()
    }

    /// The Authorization headers sent for this path.
    fn auths(&self, path: &str) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.split('?').next() == Some(path))
            .map(|(_, a)| a.clone())
            .collect()
    }
}

async fn fake_gh() -> (String, FakeGh) {
    let gh = FakeGh::default();
    let state = gh.clone();
    let router = Router::new().fallback(
        move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
            let gh = state.clone();
            async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                gh.seen.lock().unwrap().push((uri.to_string(), auth));
                let answer = gh.answers.lock().unwrap().get(uri.path()).cloned();
                match answer {
                    Some(a) => {
                        let mut res = axum::response::Response::new(Body::from(a.body));
                        *res.status_mut() = StatusCode::from_u16(a.status).unwrap();
                        res.headers_mut()
                            .insert("content-type", "application/json".parse().unwrap());
                        for (k, v) in a.headers {
                            res.headers_mut().insert(
                                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                                v.parse().unwrap(),
                            );
                        }
                        res
                    }
                    None => (
                        StatusCode::NOT_FOUND,
                        axum::Json(json!({ "message": "Not Found" })),
                    )
                        .into_response(),
                }
            }
        },
    );
    let base = serve_fake(router).await;
    // GitHub Actions' OIDC keys (the test key), so Slop Build tokens work against it.
    gh.set(
        "/.well-known/jwks",
        200,
        &[],
        include_str!("../fixtures/oidc-test-jwks.json"),
    );
    (base, gh)
}

/// A test app whose GitHub API and OIDC issuer are this fake.
async fn app_on(db: PgPool, gh_base: &str, adjust: impl FnOnce(&mut Config)) -> TestApp {
    let base = gh_base.to_string();
    test_app_with(db, move |c| {
        c.releases.github_api_base = base.parse().unwrap();
        c.releases.github_oidc_issuer = base.trim_end_matches('/').to_string();
        adjust(c);
    })
    .await
}

fn github_app_config() -> slop_api::config::GithubAppConfig {
    slop_api::config::GithubAppConfig {
        app_id: "123".into(),
        slug: "slop-test".into(),
        private_key: jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
            "../fixtures/github-app-test-key.pem"
        ))
        .unwrap(),
        webhook_secret: "hook-secret".into(),
    }
}

// ---------------------------------------------------------------------------------------------
// Picking and submitting GitHub releases (updates.rs)

/// The picker lists the repository's releases for its maker (or their agent), marking ones
/// already sent; a mod's .zip is for any platform.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_pick_from_their_repositorys_github_releases(db: PgPool) {
    let (gh_base, gh) = fake_gh().await;
    let app = app_on(db, &gh_base, |_| {}).await;
    let (maker_id, maker) = github_app(&app).await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    gh.json(
        "/repos/octo/game/releases",
        200,
        json!([
            { "tag_name": "v1.1.0", "name": "  ", "draft": false, "prerelease": true,
              "published_at": "2026-10-01T00:00:00Z", "html_url": "https://github.com/octo/game/releases/tag/v1.1.0",
              "assets": [{ "name": "game 1.1.0.zip", "size": 10 }] },
            { "tag_name": "v1.0.0", "name": "First", "draft": false, "prerelease": false,
              "html_url": "https://github.com/octo/game/releases/tag/v1.0.0", "assets": [] }
        ]),
    );
    release_with_file(
        &app,
        "game",
        "1.0.0",
        "rejected",
        "https://github.com/octo/game/releases/download/v1.0.0/a.zip",
        None,
    )
    .await;
    let picks = |r: Req| async { app.send(r).await };

    let res = picks(req(Method::GET, "/v1/apps/game/github-releases").cookie(&maker)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(res).await;
    assert_eq!(list[0]["tag"], "v1.1.0");
    assert_eq!(list[0]["submitted"], false);
    assert_eq!(list[0]["prerelease"], true);
    assert!(list[0]["name"].is_null(), "a blank name is left out");
    assert_eq!(
        list[0]["assets"][0]["url"],
        "https://github.com/octo/game/releases/download/v1.1.0/game%201.1.0.zip"
    );
    assert_eq!(list[0]["assets"][0]["platform"], "windows");
    assert_eq!(
        list[1]["submitted"], true,
        "1.0.0 was sent before (any status)"
    );
    assert_eq!(list[1]["name"], "First");

    // The maker's agent may read them too; nobody else.
    let agent = agent_token(&app, &maker, "read", Some(&["game"])).await;
    assert_eq!(
        status_of(
            &app,
            req(Method::GET, "/v1/apps/game/github-releases").header("authorization", agent)
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::GET, "/v1/apps/game/github-releases").cookie(&stranger)
        )
        .await,
        StatusCode::NOT_FOUND
    );

    // GitHub failing: said, as a conflict.
    gh.json(
        "/repos/octo/game/releases",
        500,
        json!({ "message": "oops" }),
    );
    let res = picks(req(Method::GET, "/v1/apps/game/github-releases").cookie(&maker)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        error_message(res)
            .await
            .contains("couldn't read the releases")
    );

    // A mod's archive is for every platform.
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, maker_id, repo_url, license_spdx, platforms, parent_app_id, branch_kind)
         select $1, 'game-mod', 'Game Mod', 'A mod', 'game', 'beta', $2, 'https://github.com/octo/game-mod', 'MIT', '{any}', id, 'addon'
         from apps where slug = 'game'",
    )
    .bind(Uuid::now_v7())
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    gh.json(
        "/repos/octo/game-mod/releases",
        200,
        json!([{ "tag_name": "v1", "draft": false, "html_url": "https://github.com/octo/game-mod/releases/tag/v1",
                 "assets": [{ "name": "mod.zip", "size": 3 }, { "name": "mod.tar.gz", "size": 3 }] }]),
    );
    let list = body_json(
        picks(req(Method::GET, "/v1/apps/game-mod/github-releases").cookie(&maker)).await,
    )
    .await;
    assert_eq!(list[0]["assets"][0]["platform"], "any");
    assert_eq!(list[0]["assets"][1]["platform"], "linux");

    // Not on GitHub: nothing to pick from.
    sqlx::query("update apps set repo_url = 'https://gitlab.com/octo/game' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = picks(req(Method::GET, "/v1/apps/game/github-releases").cookie(&maker)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("only works for GitHub"));
}

fn from_github(cookie: &str, tag: &str, notes: Option<&str>) -> Req {
    let mut body = json!({
        "tag": tag,
        "files": [{
            "platform": "windows", "kind": "archive", "exec": "game.exe",
            "url": "https://github.com/octo/game/releases/download/v1.1.0/game.zip"
        }]
    });
    if let Some(n) = notes {
        body["notes"] = json!(n);
    }
    req(Method::POST, "/v1/apps/game/releases/from-github")
        .cookie(cookie)
        .json(body)
}

/// Submitting a picked release: the tag must be in the repository; the notes are the GitHub
/// release's text unless given; it's the maker's, with their passkey, at the usual pace.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn submitting_a_github_release_checks_the_tag_and_takes_its_notes(db: PgPool) {
    let (gh_base, gh) = fake_gh().await;
    let app = app_on(db, &gh_base, |_| {}).await;
    let (maker_id, maker) = github_app(&app).await;
    sqlx::query("update apps set platforms = '{windows}'")
        .execute(&app.db)
        .await
        .unwrap();
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    gh.json(
        "/repos/octo/game/commits/v1.1.0",
        200,
        json!({ "sha": "B".repeat(40) }),
    );
    gh.json(
        "/repos/octo/game/releases/tags/v1.1.0",
        200,
        json!({ "body": "é".repeat(6000) }),
    );
    gh.json(
        "/repos/octo/game/commits/v1.0.0",
        200,
        json!({ "sha": "1".repeat(40) }),
    );
    gh.json(
        "/repos/octo/game/releases/tags/v1.0.0",
        200,
        json!({ "body": "From GitHub." }),
    );

    // Bad tags.
    let long = "v".repeat(101);
    for tag in ["", long.as_str(), "v1\n"] {
        assert_eq!(
            error_field(app.send(from_github(&maker, tag, None)).await).await,
            "tag",
            "{tag:?}"
        );
    }
    let res = app.send(from_github(&maker, "v9.9.9", None)).await;
    let err = body_json(res).await;
    assert_eq!(err["error"]["field"], "tag");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("isn't in the repository"),
        "{err}"
    );
    gh.json("/repos/octo/game/commits/v5", 502, json!({}));
    assert_eq!(
        error_field(app.send(from_github(&maker, "v5", None)).await).await,
        "tag"
    );

    // Someone else, or nobody.
    assert_eq!(
        status_of(&app, from_github(&stranger, "v1.1.0", None)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            &app,
            req(Method::POST, "/v1/apps/game/releases/from-github")
                .json(json!({ "tag": "v1.1.0", "files": [] }))
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    // Past the passkey deadline without one.
    passkey_overdue(&app, maker_id).await;
    let res = app.send(from_github(&maker, "v1.1.0", None)).await;
    assert_eq!(body_json(res).await["error"]["code"], "passkey_required");
    sqlx::query("update users set passkey_due_at = null where id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(count(&app, "select count(*) from releases").await, 0);

    // The GitHub release's text, cut at 5,000 characters, and its commit.
    let res = app.send(from_github(&maker, "v1.1.0", None)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release = body_json(res).await;
    assert_eq!(release["version"], "1.1.0");
    assert_eq!(release["commit_sha"], "b".repeat(40));
    assert_eq!(release["notes"].as_str().unwrap().chars().count(), 5000);
    // Notes given win; the version comes from the tag.
    let res = app.send(from_github(&maker, "v1.0.0", Some("Mine."))).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(body_json(res).await["notes"], "Mine.");

    // Not on GitHub.
    sqlx::query("update apps set repo_url = 'https://gitlab.com/octo/game'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        status_of(&app, from_github(&maker, "v1.1.0", None)).await,
        StatusCode::CONFLICT
    );
}

/// Bug b20: releases from the GitHub picker count against the same pace as any other release.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn releases_from_the_github_picker_are_rate_limited(db: PgPool) {
    let (gh_base, gh) = fake_gh().await;
    let app = app_on(db, &gh_base, |_| {}).await;
    let (maker_id, maker) = github_app(&app).await;
    sqlx::query("update apps set platforms = '{windows}'")
        .execute(&app.db)
        .await
        .unwrap();
    gh.json(
        "/repos/octo/game/commits/v1.1.0",
        200,
        json!({ "sha": "b".repeat(40) }),
    );
    exhaust(&app, maker_id, slop_api::limits::Action::Release);
    let res = app.send(from_github(&maker, "v1.1.0", Some("x"))).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(count(&app, "select count(*) from releases").await, 0);
    assert_eq!(
        gh.hits("/repos/octo/game/commits/v1.1.0"),
        0,
        "refused before asking GitHub"
    );
}

/// The fetcher asks GitHub whether the release's commit is in the repository: a 404 or 422
/// fails the release, as does GitHub failing; other hosts are simply not verified.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn releases_fail_when_github_doesnt_have_their_commit(db: PgPool) {
    let (gh_base, gh) = fake_gh().await;
    let app = app_on(db, &gh_base, |_| {}).await;
    let (maker_id, _) = github_app(&app).await;
    for (version, commit) in [("1.0.4", "4"), ("1.0.5", "5"), ("1.0.6", "6")] {
        let id = release_with_file(
            &app,
            "game",
            version,
            "fetching",
            "https://github.com/octo/game/releases/download/v1/game.zip",
            None,
        )
        .await;
        sqlx::query(
            "update releases set commit_sha = repeat($2, 40), signed_manifest = null where id = $1",
        )
        .bind(id)
        .bind(commit)
        .execute(&app.db)
        .await
        .unwrap();
    }
    gh.json(
        &format!("/repos/octo/game/commits/{}", "5".repeat(40)),
        422,
        json!({}),
    );
    gh.json(
        &format!("/repos/octo/game/commits/{}", "6".repeat(40)),
        500,
        json!({}),
    );
    for _ in 0..3 {
        assert!(releases::fetch_next(&app.state).await.unwrap());
    }
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "select version, status::text, status_reason from releases order by version",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(rows[0].1, "failed");
    assert_eq!(
        rows[0].2,
        format!(
            "commit {} isn't in https://github.com/octo/game",
            "4".repeat(40)
        )
    );
    assert_eq!(rows[1].1, "failed");
    assert!(rows[1].2.contains("isn't in"));
    assert_eq!(rows[2].1, "failed");
    assert_eq!(rows[2].2, "GitHub answered 500 when checking the commit");
    let told: i64 = sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and title like '%couldn''t be checked'",
    )
    .bind(maker_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(told, 3);

    // Another host: fetched, with the commit not verified.
    let other = test_app(app.db.clone()).await;
    sqlx::query("delete from apps")
        .execute(&other.db)
        .await
        .unwrap();
    let (_, maker) = maker_with_app(&other, "released").await;
    let id = submit_and_fetch(&other, &maker, "2.0.0").await;
    let (status, verified): (String, bool) =
        sqlx::query_as("select status::text, commit_verified from releases where id = $1::uuid")
            .bind(&id)
            .fetch_one(&other.db)
            .await
            .unwrap();
    assert_eq!(status, "awaiting_review");
    assert!(!verified);
}

// ---------------------------------------------------------------------------------------------
// Slop Build and Slop Check (slop_build.rs), repository identity (repo_identity.rs)

/// A GitHub Actions token signed with the test key under another key ID.
fn github_token_with_kid(app: &TestApp, kid: &str) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = json!({
        "iss": app.config.releases.github_oidc_issuer,
        "aud": "http://localhost:8080",
        "iat": now, "nbf": now - 5, "exp": now + 300,
        "repository": "octo/game", "sha": BUILD_COMMIT, "job_workflow_ref": RECIPE_REF,
        "repository_id": "1001", "repository_owner_id": "42",
    });
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(kid.into());
    let key =
        jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("../fixtures/oidc-test-key.pem"))
            .unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

/// A Slop Build app (octo/game, GitHub IDs 1001 and 42) on a GitHub the test controls.
async fn slop_build_app(db: PgPool) -> (TestApp, FakeGh, Uuid, String) {
    let (gh_base, gh) = fake_gh().await;
    let app = app_on(db, &gh_base, |_| {}).await;
    gh.json(
        "/repos/octo/game",
        200,
        json!({ "id": 1001, "owner": { "id": 42 } }),
    );
    let (maker_id, maker) = github_app(&app).await;
    (app, gh, maker_id, maker)
}

/// Repojacking: a run in a repository registered again under the app's name (other GitHub IDs)
/// is refused, and Slop doesn't guess when it can't ask GitHub.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn slop_build_refuses_a_repository_registered_again_under_the_same_name(db: PgPool) {
    let (app, gh, _, _) = slop_build_app(db).await;
    for overrides in [
        json!({ "repository_id": "999" }),
        json!({ "repository_owner_id": "41" }),
        json!({ "repository_id": null }),
    ] {
        let res = submit_ci(&app, &github_token(&app, overrides.clone()), ci_release()).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{overrides}");
    }
    // The IDs were recorded on first use, from what GitHub says now.
    let ids: (Option<i64>, Option<i64>) =
        sqlx::query_as("select github_repo_id, github_owner_id from apps")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(ids, (Some(1001), Some(42)));
    // Once recorded, a new repository under the name (GitHub now says 2002) is refused even
    // with a token naming the new IDs.
    gh.json(
        "/repos/octo/game",
        200,
        json!({ "id": 2002, "owner": { "id": 43 } }),
    );
    let res = submit_ci(
        &app,
        &github_token(
            &app,
            json!({ "repository_id": "2002", "repository_owner_id": "43" }),
        ),
        ci_release(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Nothing recorded and GitHub not answering: try again later.
    sqlx::query("update apps set github_repo_id = null, github_owner_id = null")
        .execute(&app.db)
        .await
        .unwrap();
    gh.json("/repos/octo/game", 502, json!({}));
    let res = submit_ci(&app, &github_token(&app, json!({})), ci_release()).await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(count(&app, "select count(*) from releases").await, 0);

    gh.json(
        "/repos/octo/game",
        200,
        json!({ "id": 1001, "owner": { "id": 42 } }),
    );
    let res = submit_ci(&app, &github_token(&app, json!({})), ci_release()).await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

/// What a run sends is checked: the app, the recipe's tag, the release tag, the size of the
/// source checks (bug b18: before anything is made); Defender checks, runtimes and parts are
/// kept.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn slop_build_checks_what_a_run_sends_before_making_the_release(db: PgPool) {
    let (app, _, _, _) = slop_build_app(db).await;
    let token = || github_token(&app, json!({}));
    let mut unknown = ci_release();
    unknown["app"] = json!("no-such-app");
    assert_eq!(
        submit_ci(&app, &token(), unknown).await.status(),
        StatusCode::NOT_FOUND
    );
    let branch = github_token(
        &app,
        json!({ "job_workflow_ref": "slop/slop-build/.github/workflows/build.yml@refs/heads/main" }),
    );
    assert_eq!(
        submit_ci(&app, &branch, ci_release()).await.status(),
        StatusCode::FORBIDDEN
    );
    for tag in ["../evil", "", "has space", &"t".repeat(101)] {
        let mut body = ci_release();
        body["tag"] = json!(tag);
        assert_eq!(
            error_field(submit_ci(&app, &token(), body).await).await,
            "tag",
            "{tag:?}"
        );
    }

    // Oversized source checks: refused (today by the 64 KiB body limit, before the 256 KiB
    // check is reached), and nothing is left to block a retry.
    let mut body = ci_release();
    body["source_checks"] = json!({ "secrets": "x".repeat(300 * 1024) });
    let res = submit_ci(&app, &token(), body).await;
    let status = res.status();
    assert!(
        status == StatusCode::PAYLOAD_TOO_LARGE
            || (status == StatusCode::UNPROCESSABLE_ENTITY
                && body_json(res).await["error"]["field"] == "source_checks"),
        "{status}"
    );
    assert_eq!(count(&app, "select count(*) from releases").await, 0);

    // A bad Defender check is refused too.
    let mut body = ci_release();
    body["files"][3]["defender"] = json!({ "sha256": "not-hex", "engine": "1", "signatures": "1" });
    assert_eq!(
        error_field(submit_ci(&app, &token(), body).await).await,
        "defender"
    );
    assert_eq!(count(&app, "select count(*) from releases").await, 0);

    // The retry: source checks, Defender's check, runtimes and parts all kept.
    let sha = "ab".repeat(32);
    let mut body = ci_release();
    body["source_checks"] = json!({ "secrets": [], "vulnerable": [{ "package": "lodash" }] });
    body["runtimes"] = json!(["dotnet-desktop-8", "dotnet-desktop-8"]);
    body["files"][0]["parts"] = json!([
        "game-build.12-linux.tar.gz.001",
        "game-build.12-linux.tar.gz.002"
    ]);
    body["files"][3]["defender"] = json!({
        "sha256": sha, "engine": "1.1.24000", "signatures": "1.419.1",
        "detections": [], "signed_by": "SignPath Foundation"
    });
    let res = submit_ci(&app, &token(), body).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let release = body_json(res).await;
    let (checks, runtimes): (Value, Vec<String>) =
        sqlx::query_as("select source_checks, runtimes from releases")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(checks["vulnerable"][0]["package"], "lodash");
    assert_eq!(runtimes, ["dotnet-desktop-8"]);
    let parts: Value =
        sqlx::query_scalar("select parts from release_files where platform = 'linux'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        parts[1]["url"],
        "https://github.com/octo/game/releases/download/slop-build-12/game-build.12-linux.tar.gz.002"
    );
    let report: Value = sqlx::query_scalar("select report from defender_checks where sha256 = $1")
        .bind(&sha)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(report["signed_by"], "SignPath Foundation");
    assert_eq!(release["version"], "build.12");

    // Without the recipe configured, Slop Build isn't offered.
    let off = test_app_with(app.db.clone(), |c| c.releases.slop_build_workflow = None).await;
    let res = submit_ci(&off, &github_token(&off, json!({})), ci_release()).await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// GitHub's signing keys are fetched again for an unknown key ID at most once a minute.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn unknown_signing_keys_refetch_githubs_keys_at_most_once_a_minute(db: PgPool) {
    let (app, gh, _, _) = slop_build_app(db).await;
    let rotated = github_token_with_kid(&app, "rotated-key");
    assert_eq!(
        submit_ci(&app, &rotated, ci_release()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(gh.hits("/.well-known/jwks"), 1);
    assert_eq!(
        submit_ci(&app, &rotated, ci_release()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        gh.hits("/.well-known/jwks"),
        1,
        "not again within the minute"
    );
    // The known key is served from the cache.
    let res = submit_ci(&app, &github_token(&app, json!({})), ci_release()).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(gh.hits("/.well-known/jwks"), 1);
}

fn defender(sha: &str) -> Value {
    json!({ "sha256": sha, "engine": "1.1.24000", "signatures": "1.419.1", "detections": [] })
}

async fn submit_checks(app: &TestApp, token: &str, body: Value) -> Response {
    app.send(
        req(Method::POST, "/v1/slop-build/checks")
            .header("authorization", format!("Bearer {token}"))
            .json(body),
    )
    .await
}

/// Slop Check: from check.yml or the build recipe, in the app's very repository, with 1 to 20
/// well-formed checks.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn slop_check_reports_are_checked_like_slop_build_runs(db: PgPool) {
    let (app, _, _, _) = slop_build_app(db).await;
    let check_ref = RECIPE_REF.replace("build.yml", "check.yml");
    let check_token = |overrides: Value| {
        let mut o = json!({ "job_workflow_ref": check_ref });
        for (k, v) in overrides.as_object().unwrap() {
            o[k] = v.clone();
        }
        github_token(&app, o)
    };
    let body = |files: Vec<Value>| json!({ "app": "game", "files": files });

    // Both workflows may report.
    let res = submit_checks(
        &app,
        &check_token(json!({})),
        body(vec![defender(&"a".repeat(64))]),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["matched"], 0);
    let res = submit_checks(
        &app,
        &github_token(&app, json!({})),
        body(vec![defender(&"b".repeat(64))]),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(count(&app, "select count(*) from defender_checks").await, 2);

    // Refusals record nothing.
    let other_workflow = github_token(
        &app,
        json!({ "job_workflow_ref": "octo/game/.github/workflows/mine.yml@refs/tags/v1" }),
    );
    let too_long = {
        let mut d = defender(&"c".repeat(64));
        d["engine"] = json!("e".repeat(101));
        d
    };
    let long_threat = {
        let mut d = defender(&"c".repeat(64));
        d["detections"] = json!([{ "file": "a.exe", "threat": "t".repeat(201) }]);
        d
    };
    let cases = [
        (
            check_token(json!({ "repository_id": "999" })),
            body(vec![defender(&"c".repeat(64))]),
            StatusCode::FORBIDDEN,
        ),
        (
            check_token(json!({ "repository": "mallory/game" })),
            body(vec![defender(&"c".repeat(64))]),
            StatusCode::FORBIDDEN,
        ),
        (
            other_workflow,
            body(vec![defender(&"c".repeat(64))]),
            StatusCode::FORBIDDEN,
        ),
        (
            check_token(json!({})),
            json!({ "app": "nope", "files": [defender(&"c".repeat(64))] }),
            StatusCode::NOT_FOUND,
        ),
        (
            check_token(json!({})),
            body(vec![]),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            check_token(json!({})),
            body(vec![defender(&"c".repeat(64)); 21]),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            check_token(json!({})),
            body(vec![defender("xyz")]),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            check_token(json!({})),
            body(vec![too_long]),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            check_token(json!({})),
            body(vec![long_threat]),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ];
    for (i, (token, body, status)) in cases.into_iter().enumerate() {
        assert_eq!(
            submit_checks(&app, &token, body).await.status(),
            status,
            "case {i}"
        );
    }
    let res = app
        .send(
            req(Method::POST, "/v1/slop-build/checks").json(body(vec![defender(&"c".repeat(64))])),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(count(&app, "select count(*) from defender_checks").await, 2);
    // Twenty is fine.
    let twenty: Vec<Value> = (0..20).map(|i| defender(&format!("{i:064x}"))).collect();
    let res = submit_checks(&app, &check_token(json!({})), body(twenty)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        count(&app, "select count(*) from defender_checks").await,
        22
    );

    let off = test_app_with(app.db.clone(), |c| c.releases.slop_build_workflow = None).await;
    let res = submit_checks(
        &off,
        &github_token(&off, json!({})),
        body(vec![defender(&"d".repeat(64))]),
    )
    .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// "Add Slop Build": the workflow and links for GitHub repositories, read over GitHub's API
/// (an Electron app by its package.json); an explanation for other hosts.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn slop_build_setup_reads_the_repository_and_links_to_github(db: PgPool) {
    use base64::Engine;
    let (app, gh, _, maker) = slop_build_app(db).await;
    gh.json(
        "/repos/octo/game",
        200,
        json!({ "default_branch": "main", "id": 1001, "owner": { "id": 42 } }),
    );
    gh.json(
        "/repos/octo/game/contents/",
        200,
        json!([{ "name": "package.json" }, { "name": "src" }]),
    );
    let package =
        json!({ "name": "game", "devDependencies": { "electron": "^30.0.0" } }).to_string();
    let encoded = base64::engine::general_purpose::STANDARD.encode(package);
    // GitHub wraps the base64 at 60 characters.
    let wrapped = encoded
        .as_bytes()
        .chunks(60)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    gh.json(
        "/repos/octo/game/contents/package.json",
        200,
        json!({ "content": wrapped }),
    );
    let res = app
        .send(req(Method::GET, "/v1/apps/game/slop-build").cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let setup = body_json(res).await;
    assert_eq!(setup["supported"], true);
    assert_eq!(setup["project_type"], "electron");
    assert!(setup["note"].is_null());
    assert_eq!(
        setup["build_now_url"],
        "https://github.com/octo/game/actions/workflows/slop.yml"
    );
    let check = setup["check_create_url"].as_str().unwrap();
    assert!(
        check.starts_with("https://github.com/octo/game/new/main?filename=.github%2Fworkflows%2Fslop-check.yml&value="),
        "{check}"
    );
    assert!(
        setup["check_workflow"]
            .as_str()
            .unwrap()
            .contains("uses: slop/slop-build/.github/workflows/check.yml@v1")
    );
    assert!(gh.hits("/repos/octo/game/contents/package.json") == 1);

    // Plain package.json: a website, with a note.
    gh.json(
        "/repos/octo/game/contents/package.json",
        200,
        json!({ "content": base64::engine::general_purpose::STANDARD.encode("{}") }),
    );
    let setup = body_json(
        app.send(req(Method::GET, "/v1/apps/game/slop-build").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(setup["project_type"], "web");
    assert!(setup["note"].as_str().unwrap().contains("website"));

    // The maker's agent reads it too.
    let agent = agent_token(&app, &maker, "read", None).await;
    assert_eq!(
        status_of(
            &app,
            req(Method::GET, "/v1/apps/game/slop-build").header("authorization", agent)
        )
        .await,
        StatusCode::OK
    );

    // GitHub can't read the repository.
    gh.json("/repos/octo/game", 404, json!({}));
    let res = app
        .send(req(Method::GET, "/v1/apps/game/slop-build").cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("GitHub answered 404"));

    // Not GitHub.
    sqlx::query("update apps set repo_url = 'https://gitlab.com/octo/game'")
        .execute(&app.db)
        .await
        .unwrap();
    let setup = body_json(
        app.send(req(Method::GET, "/v1/apps/game/slop-build").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(setup["supported"], false);
    assert!(
        setup["note"]
            .as_str()
            .unwrap()
            .contains("GitHub repository")
    );
    assert!(
        setup["create_url"].is_null()
            && setup["build_now_url"].is_null()
            && setup["check_create_url"].is_null()
    );

    let off = test_app_with(app.db.clone(), |c| c.releases.slop_build_workflow = None).await;
    assert_eq!(
        status_of(
            &off,
            req(Method::GET, "/v1/apps/game/slop-build").cookie(&maker)
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

/// Repository IDs are filled in for apps linked before they were recorded, and only into an
/// empty record.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn repository_ids_are_recorded_once_and_filled_in_for_older_apps(db: PgPool) {
    let (app, gh, _, _) = slop_build_app(db).await;
    let app_id = app_id(&app, "game").await;
    let ids = || async {
        sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
            "select github_repo_id, github_owner_id from apps where slug = 'game'",
        )
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    // GitHub not answering: nothing recorded.
    gh.json("/repos/octo/game", 500, json!({}));
    assert_eq!(
        slop_api::repo_identity::ensure(&app.state, app_id)
            .await
            .unwrap(),
        None
    );
    assert_eq!(ids().await, (None, None));
    // Recorded once; a later answer doesn't change it.
    gh.json(
        "/repos/octo/game",
        200,
        json!({ "id": 1001, "owner": { "id": 42 } }),
    );
    assert_eq!(
        slop_api::repo_identity::ensure(&app.state, app_id)
            .await
            .unwrap(),
        Some((1001, 42))
    );
    gh.json(
        "/repos/octo/game",
        200,
        json!({ "id": 7, "owner": { "id": 8 } }),
    );
    assert_eq!(
        slop_api::repo_identity::ensure(&app.state, app_id)
            .await
            .unwrap(),
        Some((1001, 42))
    );
    assert_eq!(ids().await, (Some(1001), Some(42)));
    assert_eq!(
        gh.hits("/repos/octo/game"),
        2,
        "not asked again once recorded"
    );
    // Not GitHub: nothing to record.
    sqlx::query("update apps set repo_url = 'https://gitlab.com/octo/game', github_repo_id = null, github_owner_id = null")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        slop_api::repo_identity::ensure(&app.state, app_id)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        slop_api::repo_identity::ensure(&app.state, Uuid::now_v7())
            .await
            .unwrap(),
        None
    );

    // The background fill picks up apps with no record.
    sqlx::query("update apps set repo_url = 'https://github.com/octo/game'")
        .execute(&app.db)
        .await
        .unwrap();
    let fill = tokio::spawn(slop_api::repo_identity::run_fill(app.state.clone()));
    let mut filled = (None, None);
    for _ in 0..100 {
        filled = ids().await;
        if filled.0.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    fill.abort();
    assert_eq!(filled, (Some(7), Some(8)));
}

/// Slop Build's source checks may be up to 256 KiB, but its submissions share the API's 64 KiB
/// body limit, so a release with a long list of findings can't be submitted at all.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
#[ignore = "n1: Slop Build submissions over 64 KiB are refused (413) though source checks may be up to 256 KiB"]
async fn slop_build_takes_source_checks_up_to_their_own_limit(db: PgPool) {
    let (app, _, _, _) = slop_build_app(db).await;
    let findings: Vec<Value> = (0..1500)
        .map(|i| json!({ "package": format!("package-{i}"), "id": format!("GHSA-0000-0000-{i:04}") }))
        .collect();
    let mut body = ci_release();
    body["source_checks"] = json!({ "secrets": [], "vulnerable": findings });
    assert!(body.to_string().len() > 64 * 1024 && body.to_string().len() < 256 * 1024);
    let res = submit_ci(&app, &github_token(&app, json!({})), body).await;
    assert_eq!(res.status(), StatusCode::CREATED);
}
