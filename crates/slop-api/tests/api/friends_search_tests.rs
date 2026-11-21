// SPDX-License-Identifier: AGPL-3.0-or-later
//! Finding people by name to add as friends (friends.rs `search`).

use crate::*;

async fn search(app: &TestApp, who: &str, q: &str) -> Vec<Value> {
    let res = app
        .send(
            req(
                Method::GET,
                format!("/v1/me/friends/search?q={}", q.replace('%', "%25")),
            )
            .cookie(who),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "searching {q:?}");
    body_json(res).await.as_array().unwrap().clone()
}

fn names(found: &[Value]) -> Vec<&str> {
    found.iter().map(|p| p["name"].as_str().unwrap()).collect()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn finds_people_by_name_best_match_first(db: PgPool) {
    let app = test_app(db).await;
    let (_, ann) = app.user("Ann", "US").await;
    app.user("Sam Lantern", "US").await;
    app.user("sam", "US").await;
    app.user("Sammy", "DE").await;
    app.user("Pam_Sam", "US").await;
    app.user("Bob", "US").await;

    // Exact first, then names that start with it, then any that hold it; case doesn't matter.
    assert_eq!(
        names(&search(&app, &ann, "SAM").await),
        ["sam", "Sam Lantern", "Sammy", "Pam_Sam"]
    );
    // An underscore or percent sign is only itself.
    assert_eq!(names(&search(&app, &ann, "m_s").await), ["Pam_Sam"]);
    assert_eq!(names(&search(&app, &ann, "%").await), Vec::<&str>::new());
    // One letter finds nobody (too many), and you never find yourself.
    assert!(search(&app, &ann, "s").await.is_empty());
    assert!(search(&app, &ann, "Ann").await.is_empty());
    // Signed out: no searching.
    let res = app
        .send(req(Method::GET, "/v1/me/friends/search?q=sam"))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn search_says_who_is_already_a_friend_and_hides_some_people(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, bob) = app.user("Rob Friend", "US").await;
    let (asked_id, _) = app.user("Rob Asked", "US").await;
    let (_, asking) = app.user("Rob Asking", "US").await;
    let (blocked_id, _) = app.user("Rob Blocked", "US").await;
    let (_, blocker) = app.user("Rob Blocker", "US").await;
    let (closed_id, _) = app.user("Rob Closed", "US").await;
    let (suspended_id, _) = app.user("Rob Suspended", "US").await;
    app.user("Rob Teen", "GB").await;
    app.user("Rob Stranger", "US").await;

    let ask = |from: String, to: Uuid| {
        let app = &app;
        async move {
            let res = app
                .send(
                    req(Method::POST, "/v1/me/friends")
                        .cookie(&from)
                        .json(json!({ "user": to.to_string() })),
                )
                .await;
            assert_eq!(res.status(), StatusCode::OK);
        }
    };
    ask(ann.clone(), bob_id).await;
    app.send(req(Method::POST, format!("/v1/me/friends/{ann_id}/accept")).cookie(&bob))
        .await;
    ask(ann.clone(), asked_id).await;
    ask(asking.clone(), ann_id).await;
    app.send(req(Method::PUT, format!("/v1/me/blocks/{blocked_id}")).cookie(&ann))
        .await;
    app.send(req(Method::PUT, format!("/v1/me/blocks/{ann_id}")).cookie(&blocker))
        .await;
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

    let found = search(&app, &ann, "rob").await;
    let by_name: Vec<(&str, &str)> = found
        .iter()
        .map(|p| (p["name"].as_str().unwrap(), p["friend"].as_str().unwrap()))
        .collect();
    // Blocked either way, closed, suspended and under-18 accounts aren't found.
    assert_eq!(
        by_name,
        [
            ("Rob Asked", "requested"),
            ("Rob Asking", "incoming"),
            ("Rob Friend", "friends"),
            ("Rob Stranger", "none")
        ]
    );
    assert!(found[0]["joined"].is_string() && found[0]["id"] == asked_id.to_string());
}
