// SPDX-License-Identifier: AGPL-3.0-or-later
//! A throwaway Slop for the browser tests (web/e2e). Never for real use.
//!
//! Makes a new database (`slop_e2e_<random>_sim`) on the test Postgres, migrates it, starts a
//! fake GitHub (release files, repository and account pictures, and a stand-in Google sign-in)
//! and a fake Stripe (fake_stripe.rs), runs the
//! API with the release fetcher on a free port, serving the built website (`web/dist`), and
//! seeds it (seed.rs: accounts for every kind of person, apps, money, People, teams, chat and
//! admin queues). Prints one line of JSON (`url`, `fake`, `stripe`, `cookie_name`, `cookies`,
//! `ids`) once it's ready, and drops the database when stopped (Ctrl-C, SIGTERM, or stdin
//! closing). web/e2e/fixtures.ts lists what's seeded.
//!
//! `cargo run -p slop-api --example e2e_server` with `DATABASE_URL` pointing at the test server.
//! `SLOP_E2E_SIMULATE=<apps>,<users>` fills the store with the simulation first (the load test).

mod fake_stripe;
mod seed;

use std::{io::Write as _, net::SocketAddr, path::PathBuf};

use anyhow::Context;
use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query},
    http::HeaderMap,
    response::Redirect,
    routing::{get, post},
};
use slop_api::{
    AppState, MIGRATOR,
    auth::session,
    config::{CatalogRules, Config, GithubConfig, ReleaseConfig},
    router,
};
use sqlx::{Connection, Executor, PgConnection, PgPool};

/// A tiny Windows build: a `.zip` with the program to start.
fn release_zip() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut buf);
    let opts: zip::write::SimpleFileOptions = Default::default();
    zip.start_file("pong.exe", opts).unwrap();
    zip.write_all(b"MZ e2e test build").unwrap();
    zip.finish().unwrap();
    buf.into_inner()
}

/// The commit every fake repository is at.
const MEDIA_COMMIT: &str = "d00dfeed00d00dfeed00d00dfeed00d00dfeed00";

fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbImage::from_fn(w, h, |x, y| {
        // A diagonal band, so pictures aren't flat.
        if (x + y) % 160 < 40 {
            image::Rgb([rgb[0] / 2, rgb[1] / 2, rgb[2] / 2])
        } else {
            image::Rgb(rgb)
        }
    })
    .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
    .expect("a PNG");
    out
}

/// An app's icon (media.rs): a disc on a gradient, placed and coloured from `seed` (the
/// repository's name), so each app's looks different, and unlike any other app's.
fn icon_png(size: u32, seed: &str) -> Vec<u8> {
    let h = seed.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    });
    let hue = |shift: u32| ((h >> shift) & 0xff) as u8;
    let (cx, cy) = (
        0.25 + (hue(8) as f32 / 512.0),
        0.25 + (hue(16) as f32 / 512.0),
    );
    let vertical = h & 1 == 1;
    let mut out = Vec::new();
    image::RgbImage::from_fn(size, size, |x, y| {
        let (fx, fy) = (x as f32 / size as f32, y as f32 / size as f32);
        if (fx - cx).powi(2) + (fy - cy).powi(2) < 0.06 {
            image::Rgb([240, 230, hue(24) / 2 + 100])
        } else {
            let g = ((if vertical { fy } else { fx }) * 200.0) as u8;
            image::Rgb([hue(32) / 2 + g / 3, hue(40) / 3 + g / 2, 255 - g])
        }
    })
    .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
    .expect("a PNG");
    out
}

/// A repository's store pictures (`.slop/media.json` and the files it lists), as
/// raw.githubusercontent.com would serve them. Served by the test server itself, so the
/// website's content security policy lets the pictures show.
///
/// Every repository has the same thumbnail and gallery, and its own icon. Some don't:
/// `neon-drift` has only an icon; a repository whose name has `noicon` in it has no icon, and one
/// with `badicon` names one that isn't square.
///
/// `dee-design` is a People portfolio with its own page design (people/design.rs), for
/// people-design.e2e.ts: three works, and `.slop/page/` with one thing in each file that Slop
/// removes.
async fn raw_file(Path(path): Path<String>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some((repo, file)) = path.split_once("/.slop/") else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    // <owner>/<repo>/<commit>
    let name = repo.split('/').nth(1).unwrap_or_default();
    if name == "dee-design" {
        return match designed_portfolio(file) {
            Some(body) => body.into_response(),
            None => axum::http::StatusCode::NOT_FOUND.into_response(),
        };
    }
    let body = match file {
        "media.json" if name == "neon-drift" => serde_json::json!({ "icon": "icon.png" })
            .to_string()
            .into_bytes(),
        "media.json" => {
            let mut media = serde_json::json!({
                "thumbnail": "thumb.png",
                "gallery": [
                    { "image": "shots/one.png", "caption": "The keep at dusk" },
                    { "image": "shots/two.png", "caption": "Lighting the first lantern" },
                    { "youtube": "https://youtu.be/dQw4w9WgXcQ" }
                ]
            });
            if name.contains("badicon") {
                media["icon"] = "icon-wide.png".into();
            } else if !name.contains("noicon") {
                media["icon"] = "icon.png".into();
            }
            media.to_string().into_bytes()
        }
        "icon.png" => icon_png(512, name),
        "icon-wide.png" => png(600, 300, [90, 60, 160]),
        "thumb.png" => png(920, 430, [200, 120, 40]),
        "shots/one.png" => png(1280, 720, [40, 90, 160]),
        "shots/two.png" => png(1280, 720, [60, 140, 80]),
        _ => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };
    body.into_response()
}

/// Dee Designer's portfolio repository (seed.rs), as raw_file serves it.
fn designed_portfolio(file: &str) -> Option<Vec<u8>> {
    Some(match file {
        "portfolio.json" => serde_json::json!({
            "avatar": "avatar.png",
            "works": [
                { "image": "works/lantern.png", "title": "Lantern", "skill": "pixel-art" },
                { "image": "works/keep.png", "title": "The keep", "skill": "pixel-art" },
                { "youtube": "https://youtu.be/dQw4w9WgXcQ", "title": "Showreel" }
            ]
        })
        .to_string()
        .into_bytes(),
        "page/index.html" => br#"<header class="intro">
  <h2>Hello, I'm Dee</h2>
  <p>I paint lanterns, keeps and the little lights between them.</p>
</header>
<script>document.title = "not allowed"</script>
<img class="hero" src="images/hero.png" alt="A row of lanterns at dusk">
<section class="featured">
  <h3>Featured</h3>
  <slop-work ref="1"></slop-work>
</section>
<section>
  <h3>Everything</h3>
  <slop-works></slop-works>
</section>
<p class="contact">Say hello: <a href="https://example.com/dee">my site</a></p>
"#
        .to_vec(),
        "page/style.css" => br#".page { background: #1b1430; padding: 24px; border-radius: 12px; }
.intro h2 { font-family: "Pixelify Sans", monospace; color: #ffd166; font-size: 32px; margin: 0 0 8px; }
.intro p, .contact { color: #f1eaff; }
.hero { width: 100%; border-radius: 8px; }
h3 { color: #ffd166; font-family: "Pixelify Sans", monospace; }
.contact a { color: #ffd166; }
.overlay { position: fixed; inset: 0; }
"#
        .to_vec(),
        "avatar.png" => png(256, 256, [255, 209, 102]),
        "works/lantern.png" => png(640, 480, [230, 150, 60]),
        "works/keep.png" => png(640, 480, [70, 60, 120]),
        "page/images/hero.png" => png(1200, 400, [40, 30, 70]),
        _ => return None,
    })
}

/// Stands in for GitHub: release files under any `/<owner>/<repo>/releases/download/...`.
async fn fake_github() -> anyhow::Result<String> {
    let zip = Bytes::from(release_zip());
    let app = Router::new()
        // Account pictures (avatars.rs): GitHub's at /avatars/u/<id>, one colour per account.
        .route(
            "/avatars/u/{id}",
            get(|Path(id): Path<u64>| async move {
                let shade = (id % 200) as u8;
                (
                    [("content-type", "image/png")],
                    png(240, 240, [40, 120 + shade / 4, 200]),
                )
            }),
        )
        // A stand-in Google sign-in (auth/oauth.rs), for linking and "Refresh picture": it says
        // yes at once and always names Pia Picture's Google account (seed.rs), whose picture is
        // one of the account pictures above.
        .route(
            "/google/authorize",
            get(
                |Query(q): Query<std::collections::HashMap<String, String>>| async move {
                    let mut back = url::Url::parse(q.get("redirect_uri").map_or("", |s| s))
                        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;
                    back.query_pairs_mut()
                        .append_pair("code", "e2e-google")
                        .append_pair("state", q.get("state").map_or("", |s| s));
                    Ok::<_, axum::http::StatusCode>(Redirect::to(back.as_str()))
                },
            ),
        )
        .route(
            "/google/token",
            post(|| async { axum::Json(serde_json::json!({ "access_token": "e2e-google" })) }),
        )
        .route(
            "/google/api/userinfo",
            get(|headers: HeaderMap| async move {
                let host = headers
                    .get("host")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("127.0.0.1")
                    .to_string();
                axum::Json(serde_json::json!({
                    "sub": seed::PIA_GOOGLE,
                    "name": "Pia Picture",
                    "email": "pia@example.test",
                    "email_verified": true,
                    "picture": format!("http://{host}/avatars/u/5252"),
                }))
            }),
        )
        // A repository's owner (repo_owner.rs): only Dee Designer's portfolio repository, owned
        // by her linked GitHub account (seed.rs). Others aren't found, as before.
        .route(
            "/repos/{owner}/{repo}",
            get(|Path((owner, repo)): Path<(String, String)>| async move {
                if repo != "dee-design" {
                    return Err(axum::http::StatusCode::NOT_FOUND);
                }
                Ok(axum::Json(serde_json::json!({
                    "name": repo,
                    "private": false,
                    "owner": { "id": seed::DEE_GITHUB, "login": owner, "type": "User" },
                })))
            }),
        )
        // The newest commit, for store pictures (media.rs).
        .route(
            "/repos/{owner}/{repo}/commits/HEAD",
            get(|| async { axum::Json(serde_json::json!({ "sha": MEDIA_COMMIT })) }),
        )
        .route(
            "/{owner}/{repo}/releases/download/{tag}/{file}",
            get(
                move |Path((_, _, _, file)): Path<(String, String, String, String)>| {
                    let zip = zip.clone();
                    async move {
                        if file.ends_with(".zip") {
                            Ok(zip)
                        } else {
                            Err(axum::http::StatusCode::NOT_FOUND)
                        }
                    }
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(format!("http://{addr}/"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "slop_api=warn,e2e_server=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let server = std::env::var("DATABASE_URL").context("DATABASE_URL: the test Postgres")?;
    let name = format!("slop_e2e_{:08x}_sim", rand::random::<u32>());
    let mut admin_conn = PgConnection::connect(&server).await?;
    admin_conn
        .execute(sqlx::AssertSqlSafe(format!("create database {name}")))
        .await?;
    let mut url = url::Url::parse(&server)?;
    url.set_path(&name);
    let db = PgPool::connect(url.as_str()).await?;
    let result = run(&db).await;
    db.close().await;
    admin_conn
        .execute(sqlx::AssertSqlSafe(format!(
            "drop database {name} with (force)"
        )))
        .await?;
    result
}

async fn run(db: &PgPool) -> anyhow::Result<()> {
    MIGRATOR.run(db).await?;
    slop_api::legal::sync(db).await?;
    let fake = fake_github().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr: SocketAddr = listener.local_addr()?;
    let stripe = fake_stripe::FakeStripe::start(format!("http://{addr}/v1/stripe/webhook")).await?;
    let web_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    anyhow::ensure!(
        web_dir.join("index.html").exists(),
        "build the website first: cd web && npm run build"
    );
    let config = Config {
        bind: addr,
        database_url: String::new(),
        migrate_database_url: None,
        bad_link_feeds: vec![],
        public_url: format!("http://{addr}").parse()?,
        github: Some(GithubConfig {
            client_id: "e2e".into(),
            client_secret: "e2e".into(),
            oauth_base: fake.parse()?,
            api_base: fake.parse()?,
        }),
        google: Some(slop_api::config::OAuthConfig {
            client_id: "e2e".into(),
            client_secret: "e2e".into(),
            authorize_url: format!("{fake}google/authorize").parse()?,
            token_url: format!("{fake}google/token").parse()?,
            api_base: format!("{fake}google/api/").parse()?,
        }),
        gitlab: None,
        bitbucket: None,
        microsoft: None,
        discord: None,
        trusted_origins: vec![],
        country_header: None,
        catalog: CatalogRules::default(),
        releases: ReleaseConfig {
            signing_key: Some(slop_manifest::ed25519_dalek::SigningKey::from_bytes(
                &[7; 32],
            )),
            max_file_bytes: 10 * 1024 * 1024,
            github_api_base: fake.parse()?,
            allow_http_downloads: true,
            require_maker_signature: false,
            scanner: slop_api::scan::Scanner::Off,
            slop_build_workflow: None,
            slop_build_version: "v1".into(),
            github_oidc_issuer: fake.trim_end_matches('/').to_string(),
            github_token: None,
            github_app: None,
            require_repo_owner: false,
            repo_token_key: Some([9; 32]),
            osv_api_base: fake.parse()?,
            source_hosts: vec![],
        },
        // Browser tests, and the load test's pretend players, all come from one address.
        rate_limits: slop_api::ratelimit::Limits {
            auth_per_minute: 1_000_000,
            write_per_minute: 1_000_000,
            read_per_minute: 1_000_000,
        },
        // A handful of seeded accounts create the apps, offers, reports and reviews of every
        // test file in one run, more than a person would in a day. No browser test meets a real
        // per-account limit: the ones about a 429 answer it themselves.
        account_limit_scale: 1_000,
        desktop: Default::default(),
        demos: Default::default(),
        media: slop_api::config::MediaConfig {
            // Served by this server (raw_file), not the fake GitHub: see there.
            github_raw_base: format!("http://{addr}/raw/").parse()?,
            // Profile pictures (avatars.rs) from the fake GitHub, on this machine.
            github_avatar_base: format!("{fake}avatars/").parse()?,
            avatar_test_hosts: vec!["127.0.0.1".into()],
        },
        // Stripe, faked (fake_stripe.rs): checkout links lead to its pay page.
        payments: slop_api::config::PaymentsConfig {
            stripe: Some(slop_api::config::StripeConfig {
                secret_key: fake_stripe::SECRET_KEY.into(),
                webhook_secret: fake_stripe::WEBHOOK_SECRET.into(),
                api_base: stripe.base.parse()?,
            }),
            simulated: false,
            vat_id: None,
            vies_url: None,
        },
        email: None,
        push: None,
        doh_url: format!("{fake}dns-query").parse()?,
        web_dir: Some(web_dir),
        client_ip_header: None,
        simulation: false,
        ops: Default::default(),
        approvals_required: 1,
        field_key: Some([11; 32]),
        security_contact: None,
        contact_email: Some("hello@slop.example".into()),
        source_url: None,
    };

    // The load test wants a full store: the simulation's games, players, votes and history.
    if let Ok(size) = std::env::var("SLOP_E2E_SIMULATE") {
        let (apps, users) = size
            .split_once(',')
            .and_then(|(a, u)| Some((a.parse().ok()?, u.parse().ok()?)))
            .context("SLOP_E2E_SIMULATE is <apps>,<users>")?;
        slop_api::sim::generate(
            db,
            slop_api::sim::Settings {
                apps,
                users,
                ..Default::default()
            },
        )
        .await?;
    }
    let state = AppState::new(db.clone(), config.clone())?;
    tokio::spawn(slop_api::releases::run_fetcher(state.clone()));
    tokio::spawn(slop_api::bounties::run_charger(state.clone()));
    let app = router(state)?.route("/raw/{*path}", get(raw_file));
    // As the real server does (main.rs). It serves while the seed runs: the fake Stripe's
    // webhooks reach it.
    let listener = axum::serve::ListenerExt::tap_io(listener, |tcp| {
        let _ = tcp.set_nodelay(true);
    });
    tokio::spawn({
        let app = app.clone();
        async move { axum::serve(listener, app).await }
    });

    let seeded = seed::seed(&seed::Ctx {
        db: db.clone(),
        config: config.clone(),
        router: app,
        fake: fake.clone(),
        stripe: stripe.clone(),
    })
    .await
    .context("seeding")?;

    let cookie_name = session::cookie_name(&config);
    println!(
        "{}",
        serde_json::json!({
            "url": format!("http://{addr}"),
            "fake": fake,
            "stripe": stripe.base,
            "cookie_name": cookie_name,
            "cookies": seeded.cookies,
            "ids": seeded.ids,
        })
    );

    // Until stopped, or until whoever started it goes away.
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let stdin_closed = tokio::task::spawn_blocking(|| {
        let mut sink = String::new();
        while std::io::stdin().read_line(&mut sink).is_ok_and(|n| n > 0) {}
    });
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
        _ = stdin_closed => {}
    }
    Ok(())
}
