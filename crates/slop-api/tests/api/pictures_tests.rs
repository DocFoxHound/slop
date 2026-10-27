// SPDX-License-Identifier: AGPL-3.0-or-later
//! Profile pictures from a linked sign-in (avatars.rs): choosing one, SLAPorSLOP serving its own copy,
//! keeping it current, refusing what isn't allowed, and what removes it.

use crate::*;
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

// ---------------------------------------------------------------------------------------------
// Helpers

/// What the fake picture hosts serve at a path (`u/<GitHub ID>` under /avatars, a name under
/// /pics). Tests run at once, so each uses its own paths.
#[derive(Clone)]
enum Pic {
    /// A picture, with an ETag; "304 Not Modified" when the request carries it.
    Image(Vec<u8>),
    /// Bytes as they are, without an ETag.
    Raw(Vec<u8>),
    Status(u16),
    Redirect(String),
}

static PICS: LazyLock<Mutex<HashMap<String, Pic>>> = LazyLock::new(Default::default);
/// The If-None-Match each request for a path carried.
static HITS: LazyLock<Mutex<HashMap<String, Vec<Option<String>>>>> =
    LazyLock::new(Default::default);
/// The picture the fake Google names for an account (by its token), and the one GitLab's public
/// API names for an account ID.
static GOOGLE_PICTURES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);
static GITLAB_PICTURES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

fn etag_of(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("\"{}\"", hex::encode(&sha2::Sha256::digest(bytes)[..6]))
}

/// The fake picture hosts (routed from the fake GitHub in main.rs).
pub(crate) async fn serve_picture(
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let sent = headers
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    HITS.lock()
        .unwrap()
        .entry(path.clone())
        .or_default()
        .push(sent.clone());
    let pic = PICS.lock().unwrap().get(&path).cloned();
    match pic {
        Some(Pic::Image(bytes)) => {
            let etag = etag_of(&bytes);
            if sent.as_deref() == Some(etag.as_str()) {
                return (StatusCode::NOT_MODIFIED, [("etag", etag)]).into_response();
            }
            (
                [("etag", etag), ("content-type", "image/png".into())],
                bytes,
            )
                .into_response()
        }
        Some(Pic::Raw(bytes)) => bytes.into_response(),
        Some(Pic::Status(code)) => StatusCode::from_u16(code).unwrap().into_response(),
        Some(Pic::Redirect(to)) => (StatusCode::FOUND, [("location", to)]).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn put_pic(path: &str, pic: Pic) {
    PICS.lock().unwrap().insert(path.to_string(), pic);
}

fn hits(path: &str) -> Vec<Option<String>> {
    HITS.lock().unwrap().get(path).cloned().unwrap_or_default()
}

fn solid(rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbImage::from_pixel(200, 150, image::Rgb(rgb))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

const RED: [u8; 3] = [200, 30, 30];
const BLUE: [u8; 3] = [30, 30, 200];
const GREEN: [u8; 3] = [30, 200, 30];

/// Google and GitLab sign-in, faked: the code is the token and names the account. Google says
/// where its picture is from GOOGLE_PICTURES; GitLab's public user API from GITLAB_PICTURES.
async fn fake_sign_ins() -> String {
    let app = Router::new()
        .route(
            "/{provider}/token",
            post(|body: String| async move { axum::Json(json!({ "access_token": form_code(&body) })) }),
        )
        .route(
            "/google/userinfo",
            get(|headers: axum::http::HeaderMap| async move {
                let t = bearer(&headers);
                let picture = GOOGLE_PICTURES.lock().unwrap().get(&t).cloned();
                axum::Json(json!({ "sub": t, "name": "Gail", "email": format!("{t}@example.com"), "email_verified": true, "picture": picture }))
            }),
        )
        .route(
            "/gitlab/api/users/{id}",
            get(|axum::extract::Path(id): axum::extract::Path<String>, headers: axum::http::HeaderMap| async move {
                assert!(headers.get("authorization").is_none(), "GitLab's public API needs no token");
                match GITLAB_PICTURES.lock().unwrap().get(&id).cloned() {
                    Some(url) => axum::Json(json!({ "id": id, "avatar_url": url })).into_response(),
                    None => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

/// A test app with Google and GitLab sign-in on.
async fn pictures_app(db: PgPool) -> TestApp {
    let base: url::Url = fake_sign_ins().await.parse().unwrap();
    test_app_with(db, move |c| {
        let oauth = |name: &str, api: &str| slop_api::config::OAuthConfig {
            client_id: format!("{name}-client"),
            client_secret: "secret".into(),
            authorize_url: base.join(&format!("{name}/auth")).unwrap(),
            token_url: base.join(&format!("{name}/token")).unwrap(),
            api_base: base.join(api).unwrap(),
        };
        c.google = Some(oauth("google", "google/"));
        c.gitlab = Some(oauth("gitlab", "gitlab/api/"));
    })
    .await
}

/// Links a provider account to a user directly, with where its picture is.
async fn link(app: &TestApp, user: Uuid, provider: &str, subject: &str, picture: Option<String>) {
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id, picture_url) values ($1, $2, $3, $4)",
    )
    .bind(provider)
    .bind(subject)
    .bind(user)
    .bind(picture)
    .execute(&app.db)
    .await
    .unwrap();
}

async fn choose(app: &TestApp, cookie: &str, body: Value) -> Response {
    app.send(req(Method::PUT, "/v1/me/avatar").cookie(cookie).json(body))
        .await
}

/// Chooses and expects it to work; returns the picture's address.
async fn chosen(app: &TestApp, cookie: &str, body: Value) -> String {
    let res = choose(app, cookie, body.clone()).await;
    let status = res.status();
    let mine = body_json(res).await;
    assert_eq!(status, StatusCode::OK, "{body}: {mine}");
    mine["avatar_url"].as_str().unwrap().to_string()
}

async fn avatar_url(app: &TestApp, cookie: &str) -> Value {
    body_json(app.send(req(Method::GET, "/v1/me").cookie(cookie)).await).await["avatar_url"].clone()
}

/// GETs a picture address the API handed out (on this test server's origin).
async fn fetch_picture(app: &TestApp, url: &str) -> Response {
    let path = url
        .strip_prefix("http://localhost:8080")
        .unwrap_or_else(|| panic!("{url} isn't on SLAPorSLOP's own origin"));
    app.send(req(Method::GET, path)).await
}

async fn bytes(res: Response) -> Vec<u8> {
    res.into_body().collect().await.unwrap().to_bytes().to_vec()
}

/// The colour in the middle of a served picture, which must be SLAPorSLOP's 128×128 PNG.
async fn colour(app: &TestApp, url: &str) -> [u8; 3] {
    let res = fetch_picture(app, url).await;
    assert_eq!(res.status(), StatusCode::OK, "{url}");
    let body = bytes(res).await;
    let img = image::load_from_memory_with_format(&body, image::ImageFormat::Png)
        .unwrap()
        .to_rgb8();
    assert_eq!(img.dimensions(), (128, 128));
    img.get_pixel(64, 64).0
}

async fn make_due(app: &TestApp) {
    sqlx::query("update user_avatars set checked_at = now() - interval '25 hours'")
        .execute(&app.db)
        .await
        .unwrap();
}

async fn scalar<T>(app: &TestApp, sql: &str, user: Uuid) -> T
where
    T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
{
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .bind(user)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------------------------
// Tests

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_github_picture_is_served_from_slops_own_copy_until_the_letter_comes_back(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    link(&app, ann_id, "github", "9001", None).await;
    put_pic("u/9001", Pic::Image(solid(RED)));

    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/avatar").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(mine["avatar_url"], Value::Null);
    assert_eq!(mine["provider"], Value::Null);
    assert_eq!(
        mine["choices"],
        json!([{ "provider": "github", "subject": "9001", "login": null, "available": true, "updates": "daily", "can_refresh": false }])
    );
    assert_eq!(avatar_url(&app, &ann).await, Value::Null);

    let url = chosen(&app, &ann, json!({ "provider": "github" })).await;
    assert!(
        url.starts_with(&format!(
            "http://localhost:8080/v1/users/{ann_id}/avatar?v="
        )),
        "{url}"
    );
    assert_eq!(avatar_url(&app, &ann).await, json!(url));
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/avatar").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(mine["provider"], "github");
    assert_eq!(mine["subject"], "9001");
    // GitHub was asked for its 128-pixel size.
    assert_eq!(hits("u/9001").len(), 1);

    // Anyone can load it, from SLAPorSLOP: cached for good at this address, a picture and nothing else.
    let res = fetch_picture(&app, &url).await;
    assert_eq!(res.status(), StatusCode::OK);
    let h = res.headers().clone();
    assert_eq!(h["content-type"], "image/png");
    assert_eq!(h["cache-control"], "public, max-age=31536000, immutable");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert!(
        h["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("sandbox")
    );
    let etag = h["etag"].to_str().unwrap().to_string();
    assert_eq!(colour(&app, &url).await, RED);
    let res = app
        .send(
            req(
                Method::GET,
                url.strip_prefix("http://localhost:8080").unwrap(),
            )
            .header("if-none-match", etag.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(res.headers()["etag"], etag.as_str());
    // Without the version (or an old one), only for a few minutes.
    let res = app
        .send(req(Method::GET, format!("/v1/users/{ann_id}/avatar")))
        .await;
    assert_eq!(res.headers()["cache-control"], "public, max-age=300");

    // Wherever her name is: her profile.
    let profile = body_json(
        app.send(req(Method::GET, format!("/v1/users/{ann_id}")))
            .await,
    )
    .await;
    assert_eq!(profile["avatar_url"], json!(url));

    // A suspended account shows the letter.
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(ann_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        fetch_picture(&app, &url).await.status(),
        StatusCode::NOT_FOUND
    );
    sqlx::query("update users set suspended_at = null where id = $1")
        .bind(ann_id)
        .execute(&app.db)
        .await
        .unwrap();

    // Back to the letter: the copy goes.
    let res = choose(&app, &ann, json!({ "provider": null })).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["avatar_url"], Value::Null);
    assert_eq!(avatar_url(&app, &ann).await, Value::Null);
    assert_eq!(
        fetch_picture(&app, &url).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        scalar::<i64>(
            &app,
            "select count(*) from user_avatars where user_id = $1",
            ann_id
        )
        .await,
        0
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_a_linked_sign_in_with_a_picture_on_its_own_host_can_be_chosen(db: PgPool) {
    let app = pictures_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    link(&app, bob_id, "github", "9002", None).await;
    put_pic("u/9002", Pic::Image(solid(BLUE)));
    let refused = |res: Response| async move {
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(res).await;
        assert_eq!(body["error"]["field"], "provider", "{body}");
        body["error"]["message"].as_str().unwrap().to_string()
    };

    // Signed out; someone else's sign-in; nonsense.
    assert_eq!(
        app.send(req(Method::PUT, "/v1/me/avatar").json(json!({ "provider": "github" })))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(
        refused(choose(&app, &ann, json!({ "provider": "github" })).await)
            .await
            .contains("isn't linked")
    );
    assert!(
        refused(
            choose(
                &app,
                &ann,
                json!({ "provider": "github", "subject": "9002" })
            )
            .await
        )
        .await
        .contains("isn't linked")
    );
    refused(choose(&app, &ann, json!({ "provider": "myspace" })).await).await;

    // Microsoft shares no picture; Google's is known only after a sign-in with it.
    link(&app, ann_id, "microsoft", "ms-ann", None).await;
    link(&app, ann_id, "google", "g-ann", None).await;
    assert!(
        refused(choose(&app, &ann, json!({ "provider": "microsoft" })).await)
            .await
            .contains("doesn't share a picture")
    );
    assert!(
        refused(choose(&app, &ann, json!({ "provider": "google" })).await)
            .await
            .contains("Refresh")
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/avatar").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(
        mine["choices"],
        json!([
            { "provider": "microsoft", "subject": "ms-ann", "login": null, "available": false, "updates": "never", "can_refresh": false },
            { "provider": "google", "subject": "g-ann", "login": null, "available": false, "updates": "sign_in", "can_refresh": true }
        ])
    );

    // Addresses SLAPorSLOP won't read: another host, a redirect to one, http, a private address.
    let set = |url: String| {
        let db = app.db.clone();
        async move {
            sqlx::query("update identities set picture_url = $1 where provider_subject = 'g-ann'")
                .bind(url)
                .execute(&db)
                .await
                .unwrap();
        }
    };
    for bad in [
        "https://evil.example/me.png",
        "http://lh3.googleusercontent.com/me.png",
        "https://10.0.0.1/me.png",
        "https://user:pass@lh3.googleusercontent.com/me.png",
    ] {
        set(bad.into()).await;
        assert!(
            refused(choose(&app, &ann, json!({ "provider": "google" })).await)
                .await
                .contains("couldn't get your picture"),
            "{bad}"
        );
    }
    put_pic(
        "ann-away",
        Pic::Redirect("https://evil.example/me.png".into()),
    );
    set(format!("{}pics/ann-away", app.fake_base)).await;
    refused(choose(&app, &ann, json!({ "provider": "google" })).await).await;

    // Not a picture, too big, missing.
    put_pic(
        "ann-svg",
        Pic::Raw(
            b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>".to_vec(),
        ),
    );
    let mut huge = b"\x89PNG\r\n\x1a\n".to_vec();
    huge.resize(1024 * 1024 + 10, 0);
    put_pic("ann-huge", Pic::Raw(huge));
    for name in ["ann-svg", "ann-huge"] {
        set(format!("{}pics/{name}", app.fake_base)).await;
        assert!(
            refused(choose(&app, &ann, json!({ "provider": "google" })).await)
                .await
                .contains("couldn't get your picture"),
            "{name}"
        );
    }
    set(format!("{}pics/ann-missing", app.fake_base)).await;
    refused(choose(&app, &ann, json!({ "provider": "google" })).await).await;
    assert_eq!(
        scalar::<i64>(
            &app,
            "select count(*) from user_avatars where user_id = $1",
            ann_id
        )
        .await,
        0
    );

    // A redirect to the provider's own host is followed; whatever hides after the picture's
    // pixels doesn't survive the re-encoding.
    let mut polyglot = solid(GREEN);
    polyglot.extend_from_slice(b"<script>alert(1)</script>");
    put_pic("ann-real", Pic::Raw(polyglot));
    put_pic(
        "ann-hop",
        Pic::Redirect(format!("{}pics/ann-real", app.fake_base)),
    );
    set(format!("{}pics/ann-hop", app.fake_base)).await;
    let url = chosen(&app, &ann, json!({ "provider": "google" })).await;
    assert_eq!(colour(&app, &url).await, GREEN);
    let served = bytes(fetch_picture(&app, &url).await).await;
    assert!(!served.windows(8).any(|w| w == b"<script>"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_google_picture_follows_each_sign_in_and_the_daily_check(db: PgPool) {
    let app = pictures_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    link(&app, ann_id, "google", "g-pic-ann", None).await;
    let first = format!("{}pics/gail-1.png", app.fake_base);
    let second = format!("{}pics/gail-2.png", app.fake_base);
    put_pic("gail-1.png", Pic::Image(solid(RED)));
    put_pic("gail-2.png", Pic::Image(solid(BLUE)));
    GOOGLE_PICTURES
        .lock()
        .unwrap()
        .insert("g-pic-ann".into(), first.clone());

    // "Refresh picture" is the link flow again, back to the account page.
    let res = oauth_flow(
        &app,
        "/v1/auth/google/link?return_to=%2Faccount%3Fpicture%3Drefresh%23picture",
        &[&ann],
        "g-pic-ann",
    )
    .await;
    assert_eq!(
        location(&res),
        "/account?picture=refresh&linked=already#picture"
    );
    let stored: Option<String> = scalar(
        &app,
        "select picture_url from identities where user_id = $1",
        ann_id,
    )
    .await;
    assert_eq!(stored.as_deref(), Some(first.as_str()));
    let url = chosen(&app, &ann, json!({ "provider": "google" })).await;
    assert_eq!(colour(&app, &url).await, RED);

    // A new picture at Google: the next sign-in brings it.
    GOOGLE_PICTURES
        .lock()
        .unwrap()
        .insert("g-pic-ann".into(), second.clone());
    let res = oauth_flow(&app, "/v1/auth/google/start", &[], "g-pic-ann").await;
    assert!(set_cookie(&res, "slop_session").is_some());
    let newer = avatar_url(&app, &ann).await.as_str().unwrap().to_string();
    assert_ne!(newer, url, "a new picture gets a new address");
    assert_eq!(colour(&app, &newer).await, BLUE);
    let source: String = scalar(
        &app,
        "select source_url from user_avatars where user_id = $1",
        ann_id,
    )
    .await;
    assert_eq!(source, second);

    // The daily check reads only what's due, asking with the last ETag.
    assert_eq!(slop_api::avatars::refresh_due(&app.state).await.unwrap(), 0);
    put_pic("gail-2.png", Pic::Image(solid(GREEN)));
    make_due(&app).await;
    assert_eq!(slop_api::avatars::refresh_due(&app.state).await.unwrap(), 1);
    assert_eq!(
        hits("gail-2.png").last().unwrap().as_deref(),
        Some(etag_of(&solid(BLUE)).as_str())
    );
    let newest = avatar_url(&app, &ann).await.as_str().unwrap().to_string();
    assert_eq!(colour(&app, &newest).await, GREEN);

    // Unchanged: "304 Not Modified", nothing to do.
    make_due(&app).await;
    assert_eq!(slop_api::avatars::refresh_due(&app.state).await.unwrap(), 0);
    assert_eq!(
        hits("gail-2.png").last().unwrap().as_deref(),
        Some(etag_of(&solid(GREEN)).as_str())
    );
    let (failures, recent): (i32, bool) = sqlx::query_as(
        "select failures, checked_at > now() - interval '1 minute' from user_avatars where user_id = $1",
    )
    .bind(ann_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((failures, recent), (0, true));

    // Gone at Google: the last copy stays.
    put_pic("gail-2.png", Pic::Status(404));
    make_due(&app).await;
    assert_eq!(slop_api::avatars::refresh_due(&app.state).await.unwrap(), 0);
    let failures: i32 = scalar(
        &app,
        "select failures from user_avatars where user_id = $1",
        ann_id,
    )
    .await;
    assert_eq!(failures, 1);
    assert_eq!(avatar_url(&app, &ann).await, json!(newest));
    assert_eq!(colour(&app, &newest).await, GREEN);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_gitlab_picture_is_found_and_kept_current_without_a_token(db: PgPool) {
    let app = pictures_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    link(&app, ann_id, "gitlab", "7001", None).await;
    let first = format!("{}pics/gl-7001-a.png", app.fake_base);
    let second = format!("{}pics/gl-7001-b.png", app.fake_base);
    put_pic("gl-7001-a.png", Pic::Image(solid(RED)));
    put_pic("gl-7001-b.png", Pic::Image(solid(BLUE)));
    GITLAB_PICTURES
        .lock()
        .unwrap()
        .insert("7001".into(), first.clone());

    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/avatar").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(mine["choices"][0]["available"], true);
    assert_eq!(mine["choices"][0]["updates"], "daily");
    let url = chosen(&app, &ann, json!({ "provider": "gitlab" })).await;
    assert_eq!(colour(&app, &url).await, RED);

    // A new picture at GitLab: the daily check finds where it is now.
    GITLAB_PICTURES
        .lock()
        .unwrap()
        .insert("7001".into(), second.clone());
    make_due(&app).await;
    assert_eq!(slop_api::avatars::refresh_due(&app.state).await.unwrap(), 1);
    let newer = avatar_url(&app, &ann).await.as_str().unwrap().to_string();
    assert_eq!(colour(&app, &newer).await, BLUE);
    let (source, stored): (String, Option<String>) = sqlx::query_as(
        "select a.source_url, i.picture_url from user_avatars a
         join identities i on i.provider = a.provider and i.provider_subject = a.provider_subject
         where a.user_id = $1",
    )
    .bind(ann_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(source, second);
    assert_eq!(stored.as_deref(), Some(second.as_str()));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn unlinking_staff_and_closing_remove_the_picture_and_the_export_has_it(db: PgPool) {
    let app = pictures_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (_, staff) = make_admin(&app, "Olive Owner").await;
    link(&app, ann_id, "github", "9003", None).await;
    put_pic("u/9003", Pic::Image(solid(RED)));
    put_pic("ann-g.png", Pic::Image(solid(BLUE)));
    link(
        &app,
        ann_id,
        "google",
        "g-rm-ann",
        Some(format!("{}pics/ann-g.png", app.fake_base)),
    )
    .await;
    let count = || async {
        scalar::<i64>(
            &app,
            "select count(*) from user_avatars where user_id = $1",
            ann_id,
        )
        .await
    };

    // Unlinking the sign-in it comes from.
    chosen(&app, &ann, json!({ "provider": "github" })).await;
    let res = app
        .send(req(Method::DELETE, "/v1/me/identities/github/9003").cookie(&ann))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(count().await, 0);
    assert_eq!(avatar_url(&app, &ann).await, Value::Null);

    // The data download has it, and where each sign-in's picture is.
    chosen(&app, &ann, json!({ "provider": "google" })).await;
    let export = body_json(
        app.send(req(Method::GET, "/v1/me/export").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(export["profile_picture"]["from"], "google");
    assert_eq!(export["profile_picture"]["account_id"], "g-rm-ann");
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode(export["profile_picture"]["png_base64"].as_str().unwrap())
        .unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    assert_eq!(
        export["sign_ins"][0]["picture_url"],
        json!(format!("{}pics/ann-g.png", app.fake_base))
    );

    // Staff take it off, saying why; it's in the audit log and she's told.
    let profile = body_json(
        app.send(req(Method::GET, format!("/v1/admin/users/{ann_id}/profile")).cookie(&staff))
            .await,
    )
    .await;
    assert_eq!(profile["avatar_url"], avatar_url(&app, &ann).await);
    let edit = |remove: bool| {
        req(Method::PATCH, format!("/v1/admin/users/{ann_id}/profile"))
            .cookie(&staff)
            .json(json!({ "remove_picture": remove, "reason": "That picture is someone else's photo." }))
    };
    let res = app.send(edit(true)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["changed"], json!(["picture"]));
    assert_eq!(count().await, 0);
    let logged: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'user.profile_edited' and subject = $1",
    )
    .bind(ann_id.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        logged["changed"]["picture"],
        json!({ "from": "google", "to": null })
    );
    let told: i64 = scalar(
        &app,
        "select count(*) from notifications where user_id = $1 and title = 'SLAPorSLOP changed your profile'",
        ann_id,
    )
    .await;
    assert_eq!(told, 1);
    // Nothing left to take off.
    assert_eq!(
        app.send(edit(true)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Closing the account.
    chosen(&app, &ann, json!({ "provider": "google" })).await;
    let url = avatar_url(&app, &ann).await.as_str().unwrap().to_string();
    let res = app
        .send(
            req(Method::POST, "/v1/me/close")
                .cookie(&ann)
                .json(json!({ "confirm": "close my account" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(count().await, 0);
    let pictures: i64 = scalar(
        &app,
        "select count(picture_url) from identities where user_id = $1",
        ann_id,
    )
    .await;
    assert_eq!(pictures, 0);
    assert_eq!(
        fetch_picture(&app, &url).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn combining_keeps_the_signed_in_accounts_picture_or_brings_the_others(db: PgPool) {
    let app = pictures_app(db).await;
    put_pic("u/9004", Pic::Image(solid(RED)));
    put_pic("u/9005", Pic::Image(solid(BLUE)));
    put_pic("u/9006", Pic::Image(solid(GREEN)));
    let combine = |cookie: String, code: &'static str| {
        let app = &app;
        async move {
            let res = oauth_flow(app, "/v1/auth/google/link", &[&cookie], code).await;
            assert_eq!(location(&res), "/account/combine");
            let pending = set_cookie(&res, "slop_combine").unwrap();
            let res = app
                .send(
                    req(Method::POST, "/v1/auth/combine")
                        .cookie(&cookie)
                        .cookie(&pending),
                )
                .await;
            assert_eq!(res.status(), StatusCode::NO_CONTENT);
        }
    };

    // Both have one: the signed-in account's stays.
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, bob) = app.user("Bob", "US").await;
    link(&app, ann_id, "github", "9004", None).await;
    link(&app, bob_id, "github", "9005", None).await;
    link(&app, bob_id, "google", "g-merge-bob", None).await;
    chosen(&app, &ann, json!({ "provider": "github" })).await;
    chosen(&app, &bob, json!({ "provider": "github" })).await;
    combine(ann.clone(), "g-merge-bob").await;
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "select user_id, provider_subject from user_avatars order by provider_subject",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(rows, vec![(ann_id, "9004".to_string())]);
    let url = avatar_url(&app, &ann).await.as_str().unwrap().to_string();
    assert_eq!(colour(&app, &url).await, RED);

    // Only the other has one: it comes along.
    let (cara_id, cara) = app.user("Cara", "US").await;
    let (dan_id, dan) = app.user("Dan", "US").await;
    link(&app, dan_id, "github", "9006", None).await;
    link(&app, dan_id, "google", "g-merge-dan", None).await;
    chosen(&app, &dan, json!({ "provider": "github" })).await;
    combine(cara.clone(), "g-merge-dan").await;
    let owner: Uuid = scalar(
        &app,
        "select user_id from user_avatars where provider_subject = '9006' and $1 is not null",
        cara_id,
    )
    .await;
    assert_eq!(owner, cara_id);
    let url = avatar_url(&app, &cara).await.as_str().unwrap().to_string();
    assert_eq!(colour(&app, &url).await, GREEN);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn friends_teams_and_people_pages_show_the_picture(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, bob) = app.user("Bob", "US").await;
    link(&app, ann_id, "github", "9007", None).await;
    put_pic("u/9007", Pic::Image(solid(RED)));
    let url = chosen(&app, &ann, json!({ "provider": "github" })).await;
    let (a, b) = if ann_id < bob_id {
        (ann_id, bob_id)
    } else {
        (bob_id, ann_id)
    };
    sqlx::query("insert into friends (user_a, user_b) values ($1, $2)")
        .bind(a)
        .bind(b)
        .execute(&app.db)
        .await
        .unwrap();
    let friends = body_json(
        app.send(req(Method::GET, "/v1/me/friends").cookie(&bob))
            .await,
    )
    .await;
    assert_eq!(friends["friends"][0]["id"], json!(ann_id));
    assert_eq!(friends["friends"][0]["avatar_url"], json!(url));
    let friends = body_json(
        app.send(req(Method::GET, "/v1/me/friends").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(friends["friends"][0]["avatar_url"], Value::Null);

    // A team she runs.
    let res = app
        .send(
            req(Method::POST, "/v1/teams")
                .cookie(&ann)
                .json(json!({ "slug": "anns-crew", "name": "Ann's Crew", "kind": "player" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let team = body_json(
        app.send(req(Method::GET, "/v1/teams/anns-crew").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(team["members"][0]["avatar_url"], json!(url));

    // Her People page, which has no portfolio avatar: her picture stands in.
    sqlx::query(
        "insert into creatives (user_id, handle, name, ai_use, trader) values ($1, 'ann-art', 'Ann Art', 'none', false)",
    )
    .bind(ann_id)
    .execute(&app.db)
    .await
    .unwrap();
    let page = body_json(
        app.send(req(Method::GET, "/v1/people/ann-art").cookie(&ann))
            .await,
    )
    .await;
    assert_eq!(page["avatar_url"], json!(url), "{page}");
}
