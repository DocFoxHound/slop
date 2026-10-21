//! The client core against a fake Slop API and release host.

use std::{
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use slop_client::{
    Error,
    api::{ApiClient, MemoryTokenStore, TokenStore},
    library::Library,
    signin,
    slop_manifest::{
        self, FileKind, Launch, MANIFEST_SCHEMA, ManifestFile, Platform, Purpose,
        REVOCATIONS_SCHEMA, ReleaseManifest, RevocationList, Revoked, Signed,
        ed25519_dalek::SigningKey,
    },
};

#[derive(Default)]
struct Fake {
    /// state, challenge and port of the last desktop sign-in request
    request: Mutex<Option<(String, String, u16)>>,
    access: Mutex<String>,
    refresh: Mutex<String>,
    refreshes: AtomicUsize,
    expires_in: Mutex<u64>,
    revoked: Mutex<Vec<String>>,
    /// Hand out an approve URL on some other site.
    foreign_approve_url: std::sync::atomic::AtomicBool,
    files: Mutex<Vec<(String, Vec<u8>)>>,
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&[9; 32])
}

fn pkce(verifier: &str) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

async fn fake_api() -> (String, Arc<Fake>) {
    let fake = Arc::new(Fake {
        expires_in: Mutex::new(900),
        ..Default::default()
    });
    async fn create(
        State(f): State<Arc<Fake>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        *f.request.lock().unwrap() = Some((
            body["state"].as_str().unwrap().into(),
            body["code_challenge"].as_str().unwrap().into(),
            body["redirect_port"].as_u64().unwrap() as u16,
        ));
        let host = if f.foreign_approve_url.load(Ordering::SeqCst) {
            "evil.example".to_string()
        } else {
            headers["host"].to_str().unwrap().to_string()
        };
        Json(
            json!({ "id": "x", "approve_url": format!("http://{host}/desktop-signin/x"), "expires_at": "2030-01-01T00:00:00Z" }),
        )
    }
    async fn token(
        State(f): State<Arc<Fake>>,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        let ok = match body["grant_type"].as_str() {
            Some("authorization_code") => {
                let challenge = f.request.lock().unwrap().as_ref().unwrap().1.clone();
                body["code"] == "the-code"
                    && pkce(body["code_verifier"].as_str().unwrap()) == challenge
            }
            Some("refresh_token") => {
                f.refreshes.fetch_add(1, Ordering::SeqCst);
                body["refresh_token"].as_str() == Some(f.refresh.lock().unwrap().as_str())
            }
            _ => false,
        };
        if !ok {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": { "code": "unauthorized", "message": "no" } })),
            );
        }
        let n = f.refreshes.load(Ordering::SeqCst);
        *f.access.lock().unwrap() = format!("access-{n}");
        *f.refresh.lock().unwrap() = format!("refresh-{n}");
        (
            StatusCode::OK,
            Json(json!({
                "access_token": f.access.lock().unwrap().clone(),
                "refresh_token": f.refresh.lock().unwrap().clone(),
                "expires_in": *f.expires_in.lock().unwrap(),
                "token_type": "Bearer"
            })),
        )
    }
    async fn me(State(f): State<Arc<Fake>>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
        let expected = format!("Bearer {}", f.access.lock().unwrap());
        if headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(expected.as_str()) {
            (StatusCode::OK, Json(json!({ "display_name": "Octo" })))
        } else {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": { "code": "unauthorized", "message": "sign in" } })),
            )
        }
    }
    async fn revocations(State(f): State<Arc<Fake>>) -> Json<Signed> {
        let list = RevocationList {
            schema: REVOCATIONS_SCHEMA,
            issued_at: "2026-10-18T00:00:00Z".into(),
            expires_at: "2099-01-01T00:00:00Z".into(),
            revoked: f
                .revoked
                .lock()
                .unwrap()
                .iter()
                .map(|id| Revoked {
                    release_id: id.clone(),
                    app_slug: "game".into(),
                    reason: "Malware".into(),
                    revoked_at: "2026-10-18T00:00:00Z".into(),
                })
                .collect(),
        };
        Json(slop_manifest::sign(&list, Purpose::RevocationList, &key()))
    }
    async fn file(
        State(f): State<Arc<Fake>>,
        axum::extract::Path(name): axum::extract::Path<String>,
    ) -> (StatusCode, Vec<u8>) {
        match f.files.lock().unwrap().iter().find(|(n, _)| *n == name) {
            Some((_, bytes)) => (StatusCode::OK, bytes.clone()),
            None => (StatusCode::NOT_FOUND, vec![]),
        }
    }
    let app = Router::new()
        .route("/v1/auth/desktop/requests", post(create))
        .route("/v1/auth/desktop/token", post(token))
        .route(
            "/v1/auth/signout",
            post(|| async { StatusCode::NO_CONTENT }),
        )
        .route("/v1/me", get(me))
        .route("/v1/revocations", get(revocations))
        .route("/files/{name}", get(file))
        // A private repository's file: refused at its own URL; Slop hands out a link instead.
        .route("/private/{name}", get(|| async { StatusCode::NOT_FOUND }))
        .route(
            "/v1/apps/game/releases/{release}/download/{platform}",
            get(|headers: HeaderMap| async move {
                let host = headers["host"].to_str().unwrap().to_string();
                Json(json!({ "url": format!("http://{host}/files/game.zip") }))
            }),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/"), fake)
}

/// The fake's approve URL must be on the API's own site, so point it there.
async fn api_for(base: &str) -> (ApiClient, Arc<MemoryTokenStore>) {
    let store = Arc::new(MemoryTokenStore::default());
    (
        ApiClient::new(base.parse().unwrap(), store.clone()).unwrap(),
        store,
    )
}

async fn signed_in(base: &str, fake: &Fake) -> (ApiClient, Arc<MemoryTokenStore>) {
    let (api, store) = api_for(base).await;
    fake.request
        .lock()
        .unwrap()
        .replace(("s".into(), pkce("v"), 1));
    *fake.refresh.lock().unwrap() = "refresh-start".into();
    store.save("refresh-start");
    (api, store)
}

// ---------------------------------------------------------------------------------------------
// Sign-in and tokens

#[tokio::test]
async fn browser_sign_in_through_loopback() {
    let (base, fake) = fake_api().await;
    let (api, store) = api_for(&base).await;

    // The browser may only be sent to the Slop site itself.
    fake.foreign_approve_url.store(true, Ordering::SeqCst);
    assert!(matches!(
        signin::begin(&api, "Laptop").await,
        Err(Error::Verify(_))
    ));
    fake.foreign_approve_url.store(false, Ordering::SeqCst);

    let pending = signin::begin(&api, "Laptop").await.unwrap();
    assert!(pending.approve_url.starts_with(base.trim_end_matches('/')));
    let (state, _, port) = fake.request.lock().unwrap().clone().unwrap();

    // The "browser" asks for a favicon, then hits the callback with a wrong state, then the
    // right one.
    let browser = tokio::spawn(async move {
        let http = reqwest::Client::new();
        let cb = format!("http://127.0.0.1:{port}");
        assert_eq!(
            http.get(format!("{cb}/favicon.ico"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            http.get(format!("{cb}/callback?code=the-code&state=wrong"))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        let page = http
            .get(format!("{cb}/callback?code=the-code&state={state}"))
            .send()
            .await
            .unwrap();
        assert_eq!(page.status(), 200);
        assert!(page.text().await.unwrap().contains("signed in"));
    });
    pending.finish(&api, Duration::from_secs(5)).await.unwrap();
    browser.await.unwrap();

    assert_eq!(store.load().as_deref(), Some("refresh-0"));
    let me = api
        .request(reqwest::Method::GET, "/v1/me", None)
        .await
        .unwrap();
    assert_eq!(me.status, 200);
    assert_eq!(me.body["display_name"], "Octo");

    api.sign_out().await.unwrap();
    assert!(store.load().is_none());
    assert!(!api.has_session());
}

#[tokio::test]
async fn concurrent_requests_share_one_refresh() {
    let (base, fake) = fake_api().await;
    let (api, store) = signed_in(&base, &fake).await;
    let calls: Vec<_> = (0..8)
        .map(|_| {
            let api = api.clone();
            tokio::spawn(async move {
                api.request(reqwest::Method::GET, "/v1/me", None)
                    .await
                    .unwrap()
                    .status
            })
        })
        .collect();
    for c in calls {
        assert_eq!(c.await.unwrap(), 200);
    }
    // Rotating refresh tokens: a second refresh with the old token would have signed us out.
    assert_eq!(fake.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(store.load().as_deref(), Some("refresh-1"));
}

#[tokio::test]
async fn a_revoked_session_signs_out_cleanly() {
    let (base, fake) = fake_api().await;
    let (api, store) = signed_in(&base, &fake).await;
    *fake.refresh.lock().unwrap() = "someone-else-used-it".into();
    let res = api
        .request(reqwest::Method::GET, "/v1/me", None)
        .await
        .unwrap();
    assert_eq!(res.status, 401);
    assert!(store.load().is_none(), "a dead refresh token is forgotten");
}

// ---------------------------------------------------------------------------------------------
// Install, launch and revocation

fn zip_of(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        for (name, data, mode) in entries {
            let opts = zip::write::SimpleFileOptions::default().unix_permissions(*mode);
            zip.start_file(*name, opts).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

const GAME: &[u8] = b"#!/bin/sh\necho \"launched with $1\" > out.txt\n";

fn manifest_for(base: &str, file: &str, bytes: &[u8], release_id: &str) -> Signed {
    manifest_with(base, file, bytes, release_id, None)
}

fn manifest_with(
    base: &str,
    file: &str,
    bytes: &[u8],
    release_id: &str,
    mods_folder: Option<slop_manifest::ModsFolder>,
) -> Signed {
    let manifest = ReleaseManifest {
        schema: MANIFEST_SCHEMA,
        app_id: "id".into(),
        app_slug: "game".into(),
        app_title: "Game".into(),
        maker: "Octo".into(),
        repo_url: "https://github.com/octo/game".into(),
        release_id: release_id.into(),
        version: "1.0.0".into(),
        commit: "a".repeat(40),
        files: vec![ManifestFile {
            platform: Platform::Linux,
            kind: FileKind::Archive,
            url: format!("{base}files/{file}"),
            file_name: file.into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
            launch: Some(Launch {
                exec: "bin/game".into(),
                args: vec!["hello".into()],
            }),
            attestation: None,
            maker_signature: None,
            parts: vec![],
        }],
        approved_at: "2026-10-18T00:00:00Z".into(),
        mod_for: None,
        build_of: None,
        mods_folder,
    };
    slop_manifest::sign(&manifest, Purpose::ReleaseManifest, &key())
}

/// A library that accepts unsigned builds: these tests are about Slop's signature and the
/// download checks. Maker signatures are covered in `unsigned_builds_and_repo_switches_are_refused`
/// and the slop-provenance tests.
fn library(dir: &std::path::Path) -> Library {
    Library::new(dir, vec![key().verifying_key()])
        .unwrap()
        .allow_http_downloads(true)
        .require_maker_signature(false)
}

fn tempdir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("slop-client-test-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
#[tokio::test]
async fn install_verify_launch_and_revoke() {
    let (base, fake) = fake_api().await;
    let zip = zip_of(&[("bin/game", GAME, 0o755), ("data/readme.txt", b"hi", 0o644)]);
    fake.files
        .lock()
        .unwrap()
        .push(("game.zip".into(), zip.clone()));
    let signed = manifest_for(&base, "game.zip", &zip, "r1");
    let dir = tempdir();
    let lib = library(&dir);
    let (api, _) = api_for(&base).await;

    let seen = Mutex::new(0u64);
    let installed = lib
        .install(&signed, Platform::Linux, None, |got, _| {
            *seen.lock().unwrap() = got
        })
        .await
        .unwrap();
    assert_eq!(installed.version, "1.0.0");
    assert_eq!(*seen.lock().unwrap(), zip.len() as u64);
    assert_eq!(lib.installed(), vec![installed.clone()]);

    // Launch: the "game" runs in its own folder with the manifest's arguments.
    let revocations = lib.revocations_for_launch(&api).await;
    let status = lib
        .launch_command("game", revocations.as_ref())
        .unwrap()
        .status()
        .unwrap();
    assert!(status.success());
    let out = std::fs::read_to_string(dir.join("apps/game/files/out.txt")).unwrap();
    assert_eq!(out.trim(), "launched with hello");

    // Tampering with the installed program blocks the launch.
    std::fs::write(
        dir.join("apps/game/files/bin/game"),
        b"#!/bin/sh\necho pwned\n",
    )
    .unwrap();
    assert!(matches!(
        lib.launch_command("game", None),
        Err(Error::Verify(_))
    ));

    // Reinstall, then revoke: the kill switch stops it, even offline from the saved list.
    lib.install(&signed, Platform::Linux, None, |_, _| {})
        .await
        .unwrap();
    fake.revoked.lock().unwrap().push("r1".into());
    let revocations = lib.revocations_for_launch(&api).await;
    assert!(
        matches!(lib.launch_command("game", revocations.as_ref()), Err(Error::Revoked(r)) if r == "Malware")
    );
    assert!(lib.cached_revocations().unwrap().is_revoked("r1").is_some());

    lib.uninstall("game").unwrap();
    assert!(lib.installed().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

/// A mod's manifest: one `.zip` for every platform, nothing to start, and the game it's for.
fn mod_manifest(base: &str, bytes: &[u8], release_id: &str) -> Signed {
    let manifest = ReleaseManifest {
        schema: MANIFEST_SCHEMA,
        app_id: "mod-id".into(),
        app_slug: "big-mod".into(),
        app_title: "Big Mod".into(),
        maker: "Ann".into(),
        repo_url: "https://github.com/ann/big-mod".into(),
        release_id: release_id.into(),
        version: "0.1.0".into(),
        commit: "b".repeat(40),
        files: vec![ManifestFile {
            platform: Platform::Any,
            kind: FileKind::Archive,
            url: format!("{base}files/mod.zip"),
            file_name: "mod.zip".into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
            launch: None,
            attestation: None,
            maker_signature: None,
            parts: vec![],
        }],
        approved_at: "2026-10-18T00:00:00Z".into(),
        mod_for: Some(slop_manifest::ModFor {
            app_id: "id".into(),
            app_slug: "game".into(),
            app_title: "Game".into(),
        }),
        build_of: None,
        mods_folder: None,
    };
    slop_manifest::sign(&manifest, Purpose::ReleaseManifest, &key())
}

/// A community build of "game": its own app, signed as a build of the original.
fn community_manifest(base: &str, file: &str, bytes: &[u8], release_id: &str) -> Signed {
    let manifest = ReleaseManifest {
        schema: MANIFEST_SCHEMA,
        app_id: "plus-id".into(),
        app_slug: "game-plus".into(),
        app_title: "Game Plus".into(),
        maker: "Bo".into(),
        repo_url: "https://github.com/bo/game".into(),
        release_id: release_id.into(),
        version: "1.1.0".into(),
        commit: "c".repeat(40),
        files: vec![ManifestFile {
            platform: Platform::Linux,
            kind: FileKind::Archive,
            url: format!("{base}files/{file}"),
            file_name: file.into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
            launch: Some(Launch {
                exec: "bin/game".into(),
                args: vec![],
            }),
            attestation: None,
            maker_signature: None,
            parts: vec![],
        }],
        approved_at: "2026-10-18T00:00:00Z".into(),
        mod_for: None,
        build_of: Some(slop_manifest::ModFor {
            app_id: "id".into(),
            app_slug: "game".into(),
            app_title: "Game".into(),
        }),
        mods_folder: None,
    };
    slop_manifest::sign(&manifest, Purpose::ReleaseManifest, &key())
}

/// A community build installs beside its original, and the player can pick it for the
/// original's Play button (DESIGN §9). Each keeps its own save folder.
#[tokio::test]
async fn community_builds_can_be_picked_to_play_instead_of_the_original() {
    let (base, fake) = fake_api().await;
    let game = zip_of(&[("bin/game", b"#!/bin/sh\n", 0o755)]);
    let plus = zip_of(&[("bin/game", b"#!/bin/sh\necho plus\n", 0o755)]);
    fake.files.lock().unwrap().extend([
        ("game.zip".into(), game.clone()),
        ("plus.zip".into(), plus.clone()),
    ]);
    let dir = tempdir();
    let lib = library(&dir);
    lib.install(
        &manifest_for(&base, "game.zip", &game, "r1"),
        Platform::Linux,
        None,
        |_, _| {},
    )
    .await
    .unwrap();
    assert!(
        lib.set_play_choice("game", Some("game-plus")).is_err(),
        "only once it's installed"
    );
    let installed = lib
        .install(
            &community_manifest(&base, "plus.zip", &plus, "p1"),
            Platform::Linux,
            None,
            |_, _| {},
        )
        .await
        .unwrap();
    assert_eq!(installed.build_of.as_deref(), Some("game"));
    let slugs: Vec<_> = lib.installed().into_iter().map(|a| a.slug).collect();
    assert_eq!(slugs, ["game", "game-plus"], "both stay installed");
    assert_eq!(lib.installed()[0].build_of, None);

    lib.set_play_choice("game", Some("game-plus")).unwrap();
    assert_eq!(
        lib.play_choices().get("game").map(String::as_str),
        Some("game-plus")
    );
    assert!(
        lib.set_play_choice("game-plus", Some("game")).is_err(),
        "the original isn't a build of its build"
    );

    // Separate save folders; saves can be copied across.
    std::fs::write(lib.save_dir("game").unwrap().join("slot1.json"), b"{}").unwrap();
    assert_eq!(lib.save_size("game-plus").unwrap(), 0);
    lib.copy_saves("game", "game-plus").unwrap();
    assert_eq!(lib.save_size("game-plus").unwrap(), 2);

    // Back to the original, or the pick goes when the build does.
    lib.set_play_choice("game", None).unwrap();
    assert!(lib.play_choices().is_empty());
    lib.set_play_choice("game", Some("game-plus")).unwrap();
    lib.uninstall("game-plus").unwrap();
    assert!(lib.play_choices().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn mods_are_added_to_their_game_and_handed_to_it_at_launch() {
    let (base, fake) = fake_api().await;
    let game = zip_of(&[(
        "bin/game",
        b"#!/bin/sh\nprintf '%s' \"$SLOP_MODS\" > mods.txt\n",
        0o755,
    )]);
    let a_mod = zip_of(&[
        ("textures/sky.png", b"png", 0o644),
        ("run.sh", b"#!/bin/sh\n", 0o755),
    ]);
    fake.files.lock().unwrap().extend([
        ("game.zip".into(), game.clone()),
        ("mod.zip".into(), a_mod.clone()),
    ]);
    let dir = tempdir();
    let lib = library(&dir);
    let (api, _) = api_for(&base).await;
    let signed_mod = mod_manifest(&base, &a_mod, "m1");

    // Only into an installed game, and never as an app of its own.
    assert!(matches!(
        lib.install_mod(&signed_mod, None, |_, _| {}).await,
        Err(Error::Install(m)) if m.contains("install Game first")
    ));
    lib.install(
        &manifest_for(&base, "game.zip", &game, "r1"),
        Platform::Linux,
        None,
        |_, _| {},
    )
    .await
    .unwrap();
    assert!(
        lib.install(&signed_mod, Platform::Any, None, |_, _| {})
            .await
            .is_err()
    );
    let added = lib.install_mod(&signed_mod, None, |_, _| {}).await.unwrap();
    assert_eq!(
        (added.slug.as_str(), added.game_slug.as_str()),
        ("big-mod", "game")
    );
    assert_eq!(lib.installed_mods("game"), vec![added]);
    assert_eq!(
        lib.installed().len(),
        1,
        "a mod isn't an app in the library"
    );

    // The game is told where its mods are.
    let run = |lib: &Library, revocations: Option<&slop_manifest::RevocationList>| {
        assert!(
            lib.launch_command("game", revocations)
                .unwrap()
                .status()
                .unwrap()
                .success()
        );
        std::fs::read_to_string(dir.join("apps/game/files/mods.txt")).unwrap()
    };
    let revocations = lib.revocations_for_launch(&api).await;
    assert_eq!(
        run(&lib, revocations.as_ref()),
        dir.join("mods/big-mod/files").to_str().unwrap()
    );
    assert!(dir.join("mods/big-mod/files/textures/sky.png").exists());

    // A pulled mod isn't handed over, but the game still starts.
    fake.revoked.lock().unwrap().push("m1".into());
    let revocations = lib.revocations_for_launch(&api).await;
    assert_eq!(run(&lib, revocations.as_ref()), "");
    assert_eq!(
        lib.installed_mods("game")[0].revoked.as_deref(),
        Some("Malware")
    );

    // Uninstalling the game takes its mods with it.
    lib.uninstall("game").unwrap();
    assert!(!dir.join("mods/big-mod").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// A game's own mods folder (DESIGN §9): before each start Slop puts the approved mods there
/// and moves anything else out, telling the player. A mods folder in the saves isn't swept for
/// code: mods may carry it.
#[cfg(unix)]
#[tokio::test]
async fn the_games_own_mods_folder_holds_only_approved_mods() {
    let (base, fake) = fake_api().await;
    let game = zip_of(&[("bin/game", b"#!/bin/sh\n", 0o755)]);
    let a_mod = zip_of(&[("Plugin.dll", b"MZ\x90\x00 managed code", 0o644)]);
    fake.files.lock().unwrap().extend([
        ("game.zip".into(), game.clone()),
        ("mod.zip".into(), a_mod.clone()),
    ]);
    for (folder_base, folder) in [
        (slop_manifest::ModsBase::Game, "apps/game/files/mods"),
        (slop_manifest::ModsBase::Saves, "saves/game/BepInEx/plugins"),
    ] {
        let dir = tempdir();
        let lib = library(&dir);
        let path = if folder_base == slop_manifest::ModsBase::Game {
            "mods"
        } else {
            "BepInEx/plugins"
        };
        lib.install(
            &manifest_with(
                &base,
                "game.zip",
                &game,
                "r1",
                Some(slop_manifest::ModsFolder {
                    base: folder_base,
                    path: path.into(),
                    layout: slop_manifest::ModsLayout::Flat,
                }),
            ),
            Platform::Linux,
            None,
            |_, _| {},
        )
        .await
        .unwrap();
        lib.install_mod(&mod_manifest(&base, &a_mod, "m1"), None, |_, _| {})
            .await
            .unwrap();
        let target = dir.join(folder);
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("MyCheat.dll"), b"MZ hand-made").unwrap();

        let launch = lib.prepare_launch("game", None, None, None).unwrap();
        assert_eq!(launch.unapproved_mods, ["MyCheat.dll"], "{folder}");
        assert!(
            launch.quarantined.is_empty(),
            "approved mod code isn't swept"
        );
        assert_eq!(
            std::fs::read(target.join("Plugin.dll")).unwrap(),
            b"MZ\x90\x00 managed code"
        );
        assert!(!target.join("MyCheat.dll").exists());
        assert!(
            dir.join("quarantine/game/unapproved-mods/MyCheat.dll")
                .exists()
        );

        // Removing the mod empties the folder at the next start, quietly.
        lib.uninstall_mod("big-mod").unwrap();
        let launch = lib.prepare_launch("game", None, None, None).unwrap();
        assert!(launch.unapproved_mods.is_empty());
        assert!(!target.join("Plugin.dll").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// Sandbox mode (DESIGN §6): the same save folder in both modes; in the box the app writes
/// only there and can't see the rest of the home folder. Skipped where bubblewrap can't run.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn sandboxed_apps_see_only_their_own_folders() {
    if let Err(why) = slop_client::sandbox::available() {
        eprintln!("skipping: {why}");
        return;
    }
    let (base, fake) = fake_api().await;
    // The "game" reports what it can reach, into its save folder.
    let script = br#"#!/bin/sh
cd "$SLOP_SAVE_DIR" || exit 3
echo "mode=${SLOP_SANDBOX:-normal}" > report.txt
if cat "$SECRET" >/dev/null 2>&1; then echo "secret=seen" >> report.txt; else echo "secret=hidden" >> report.txt; fi
if echo x > "$ESCAPE" 2>/dev/null; then echo "escape=written" >> report.txt; else echo "escape=blocked" >> report.txt; fi
if echo x > "$(dirname "$0")/../new" 2>/dev/null; then echo "files=written" >> report.txt; else echo "files=readonly" >> report.txt; fi
echo "config=$XDG_CONFIG_HOME" >> report.txt
cat old.txt >> report.txt 2>/dev/null
if echo '{"mods": [{"slug": "dragons"}]}' > "$SLOP_LINK/requests/r.json" 2>/dev/null; then echo "request=written" >> report.txt; else echo "request=blocked" >> report.txt; fi
if echo x > "$SLOP_LINK/answers/fake.json" 2>/dev/null; then echo "answers=written" >> report.txt; else echo "answers=readonly" >> report.txt; fi
"#;
    let zip = zip_of(&[("bin/game", script, 0o755)]);
    fake.files
        .lock()
        .unwrap()
        .push(("game.zip".into(), zip.clone()));
    // A pretend home folder with a secret in it, holding the library like the real one.
    let home = tempdir();
    let dir = home.join(".local/share/slop");
    let lib = library(&dir);
    lib.install(
        &manifest_for(&base, "game.zip", &zip, "r1"),
        Platform::Linux,
        None,
        |_, _| {},
    )
    .await
    .unwrap();
    std::fs::write(home.join("secret.txt"), b"password").unwrap();
    let saves = lib.save_dir("game").unwrap();
    std::fs::write(saves.join("old.txt"), b"old=kept\n").unwrap();
    std::fs::write(saves.join("plugin.so"), b"\x7fELF\x02").unwrap();

    // Slop Link: the game writes requests, and can only read answers in the box.
    let link = dir.join("link/run");
    std::fs::create_dir_all(link.join("requests")).unwrap();
    std::fs::create_dir_all(link.join("answers")).unwrap();
    let run = |sandbox: Option<slop_client::sandbox::AppSandbox>| {
        let _ = std::fs::remove_file(link.join("answers/fake.json"));
        let mut launch = lib
            .prepare_launch("game", None, sandbox, Some(&link))
            .unwrap();
        let status = launch
            .command
            .env("HOME", &home)
            .env("SECRET", home.join("secret.txt"))
            .env("ESCAPE", home.join("escape.txt"))
            .status()
            .unwrap();
        assert!(status.success(), "{status}");
        (
            launch.quarantined,
            std::fs::read_to_string(saves.join("report.txt")).unwrap(),
        )
    };

    let (quarantined, normal) = run(None);
    assert_eq!(
        quarantined,
        ["plugin.so"],
        "programs never stay in a save folder"
    );
    assert!(
        normal.contains("mode=normal") && normal.contains("secret=seen"),
        "{normal}"
    );
    assert!(normal.contains(&format!("config={}", saves.join(".config").display())));
    let _ = std::fs::remove_file(home.join("escape.txt"));

    let (_, boxed) = run(Some(slop_client::sandbox::AppSandbox {
        on: true,
        block_network: true,
    }));
    assert!(boxed.contains("mode=1"), "{boxed}");
    assert!(boxed.contains("secret=hidden"), "{boxed}");
    assert!(
        boxed.contains("escape=blocked") || !home.join("escape.txt").exists(),
        "{boxed}"
    );
    assert!(
        !home.join("escape.txt").exists(),
        "nothing written outside the save folder"
    );
    assert!(boxed.contains("files=readonly"), "{boxed}");
    assert!(
        boxed.contains("request=written") && boxed.contains("answers=readonly"),
        "{boxed}"
    );
    assert!(link.join("requests/r.json").exists());
    assert!(
        boxed.contains("old=kept"),
        "same saves in both modes: {boxed}"
    );
    assert_eq!(
        lib.save_copies("game").unwrap().len(),
        1,
        "copied before the sandboxed session"
    );

    // Saves outlive an uninstall, until the player deletes them.
    lib.uninstall("game").unwrap();
    assert!(saves.join("old.txt").exists());
    lib.delete_saves("game").unwrap();
    assert!(!dir.join("saves/game").exists());
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test]
async fn a_changed_file_is_never_installed() {
    let (base, fake) = fake_api().await;
    let zip = zip_of(&[("bin/game", GAME, 0o755)]);
    let signed = manifest_for(&base, "game.zip", &zip, "r1");
    // The maker's file was swapped after Slop scanned it.
    let mut swapped = zip.clone();
    swapped[10] ^= 0xff;
    fake.files
        .lock()
        .unwrap()
        .push(("game.zip".into(), swapped));
    let dir = tempdir();
    let lib = library(&dir);
    let err = lib
        .install(&signed, Platform::Linux, None, |_, _| {})
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Verify(_)), "{err}");
    assert!(lib.installed().is_empty());
    assert_eq!(
        std::fs::read_dir(dir.join("staging")).unwrap().count(),
        0,
        "nothing left behind"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn forged_manifests_and_unsafe_archives_are_refused() {
    let (base, fake) = fake_api().await;
    let dir = tempdir();
    let lib = library(&dir);

    // Signed by someone else.
    let zip = zip_of(&[("bin/game", GAME, 0o755)]);
    let mut forged = manifest_for(&base, "game.zip", &zip, "r1");
    let manifest: ReleaseManifest = serde_json::from_str(&forged.payload).unwrap();
    forged = slop_manifest::sign(
        &manifest,
        Purpose::ReleaseManifest,
        &SigningKey::from_bytes(&[1; 32]),
    );
    assert!(matches!(
        lib.install(&forged, Platform::Linux, None, |_, _| {}).await,
        Err(Error::Verify(_))
    ));

    // Zip-slip: an entry that climbs out of the app's folder.
    let evil = zip_of(&[
        ("bin/game", GAME, 0o755),
        ("../../escaped.txt", b"x", 0o644),
    ]);
    fake.files
        .lock()
        .unwrap()
        .push(("evil.zip".into(), evil.clone()));
    let signed = manifest_for(&base, "evil.zip", &evil, "r2");
    assert!(matches!(
        lib.install(&signed, Platform::Linux, None, |_, _| {}).await,
        Err(Error::Verify(_))
    ));
    assert!(
        !dir.join("escaped.txt").exists() && !dir.parent().unwrap().join("escaped.txt").exists()
    );

    // No build for this platform.
    let zip = zip_of(&[("bin/game", GAME, 0o755)]);
    let signed = manifest_for(&base, "game.zip", &zip, "r3");
    assert!(matches!(
        lib.install(&signed, Platform::Windows, None, |_, _| {})
            .await,
        Err(Error::Install(_))
    ));
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn unsigned_builds_and_repo_switches_are_refused() {
    let (base, fake) = fake_api().await;
    let zip = zip_of(&[("bin/game", GAME, 0o755)]);
    fake.files
        .lock()
        .unwrap()
        .push(("game.zip".into(), zip.clone()));
    let dir = tempdir();

    // By default a build must carry the maker's signature.
    let strict = Library::new(&dir, vec![key().verifying_key()])
        .unwrap()
        .allow_http_downloads(true);
    let signed = manifest_for(&base, "game.zip", &zip, "r1");
    let err = strict
        .install(&signed, Platform::Linux, None, |_, _| {})
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Verify(ref m) if m.contains("maker signature")),
        "{err}"
    );

    // A forged attestation is refused too.
    let mut manifest: ReleaseManifest = serde_json::from_str(&signed.payload).unwrap();
    manifest.files[0].attestation = Some(
        include_str!(
            "../../slop-provenance/tests/fixtures/github-actions-provenance.sigstore.json"
        )
        .into(),
    );
    let forged = slop_manifest::sign(&manifest, Purpose::ReleaseManifest, &key());
    let err = strict
        .install(&forged, Platform::Linux, None, |_, _| {})
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Verify(ref m) if m.contains("maker signature")),
        "{err}"
    );

    // Once installed from one repository, an update from another is refused.
    let lib = library(&dir);
    lib.install(&signed, Platform::Linux, None, |_, _| {})
        .await
        .unwrap();
    let mut moved: ReleaseManifest = serde_json::from_str(&signed.payload).unwrap();
    moved.repo_url = "https://github.com/someone-else/game".into();
    moved.release_id = "r2".into();
    let moved = slop_manifest::sign(&moved, Purpose::ReleaseManifest, &key());
    let err = lib
        .install(&moved, Platform::Linux, None, |_, _| {})
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Verify(ref m) if m.contains("installed app came from")),
        "{err}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn private_files_are_fetched_through_a_slop_link() {
    let (base, fake) = fake_api().await;
    let zip = zip_of(&[("bin/game", GAME, 0o755)]);
    fake.files
        .lock()
        .unwrap()
        .push(("game.zip".into(), zip.clone()));
    // The manifest points at the private URL, which refuses anonymous downloads.
    let mut manifest: ReleaseManifest =
        serde_json::from_str(&manifest_for(&base, "game.zip", &zip, "r1").payload).unwrap();
    manifest.files[0].url = format!("{base}private/game.zip");
    let signed = slop_manifest::sign(&manifest, Purpose::ReleaseManifest, &key());
    let dir = tempdir();
    let lib = library(&dir);

    let err = lib
        .install(&signed, Platform::Linux, None, |_, _| {})
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Install(ref m) if m.contains("404")),
        "{err}"
    );

    let (api, _) = api_for(&base).await;
    let installed = lib
        .install(&signed, Platform::Linux, Some(&api), |_, _| {})
        .await
        .unwrap();
    assert_eq!(installed.version, "1.0.0");
    std::fs::remove_dir_all(dir).unwrap();
}

fn tar_gz_of(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    {
        let mut tar = tar::Builder::new(&mut gz);
        for (name, data, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(*mode);
            header.set_entry_type(tar::EntryType::Regular);
            // Write the name directly, so tests can build archives the builder would refuse.
            let raw = &mut header.as_old_mut().name;
            raw[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            tar.append(&header, *data).unwrap();
        }
        tar.finish().unwrap();
    }
    gz.finish().unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn tar_gz_builds_install_and_launch() {
    let (base, fake) = fake_api().await;
    let tgz = tar_gz_of(&[("bin/game", GAME, 0o755), ("data/readme.txt", b"hi", 0o644)]);
    fake.files
        .lock()
        .unwrap()
        .push(("game-linux.tar.gz".into(), tgz.clone()));
    let signed = manifest_for(&base, "game-linux.tar.gz", &tgz, "r1");
    let dir = tempdir();
    let lib = library(&dir);
    let (api, _) = api_for(&base).await;
    lib.install(&signed, Platform::Linux, None, |_, _| {})
        .await
        .unwrap();
    let revocations = lib.revocations_for_launch(&api).await;
    let status = lib
        .launch_command("game", revocations.as_ref())
        .unwrap()
        .status()
        .unwrap();
    assert!(status.success());
    let out = std::fs::read_to_string(dir.join("apps/game/files/out.txt")).unwrap();
    assert_eq!(out.trim(), "launched with hello");

    // A path that climbs out of the app's folder is refused, and nothing is installed.
    let evil = tar_gz_of(&[("../escape", b"x", 0o644), ("bin/game", GAME, 0o755)]);
    fake.files
        .lock()
        .unwrap()
        .push(("evil.tar.gz".into(), evil.clone()));
    let dir = tempdir();
    let lib = library(&dir);
    let err = lib
        .install(
            &manifest_for(&base, "evil.tar.gz", &evil, "r2"),
            Platform::Linux,
            None,
            |_, _| {},
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unsafe"), "{err}");
    assert!(!dir.join("apps/escape").exists() && !dir.join("escape").exists());
    assert!(lib.installed().is_empty());
}

/// An `ssh-keygen -Y sign -n slop-release` signature over `file`, and the key's OpenSSH line.
fn ssh_sign(key: &SigningKey, file: &[u8]) -> (String, String) {
    use base64::Engine;
    use slop_manifest::ed25519_dalek::Signer;
    let b64 = base64::engine::general_purpose::STANDARD;
    let string = |out: &mut Vec<u8>, s: &[u8]| {
        out.extend_from_slice(&(s.len() as u32).to_be_bytes());
        out.extend_from_slice(s);
    };
    let mut pub_blob = Vec::new();
    string(&mut pub_blob, b"ssh-ed25519");
    string(&mut pub_blob, &key.verifying_key().to_bytes());
    let mut signed = b"SSHSIG".to_vec();
    string(&mut signed, b"slop-release");
    string(&mut signed, b"");
    string(&mut signed, b"sha512");
    string(&mut signed, &sha2::Sha512::digest(file));
    let mut sig_blob = Vec::new();
    string(&mut sig_blob, b"ssh-ed25519");
    string(&mut sig_blob, &key.sign(&signed).to_bytes());
    let mut blob = b"SSHSIG".to_vec();
    blob.extend_from_slice(&1u32.to_be_bytes());
    string(&mut blob, &pub_blob);
    string(&mut blob, b"slop-release");
    string(&mut blob, b"");
    string(&mut blob, b"sha512");
    string(&mut blob, &sig_blob);
    (
        format!("ssh-ed25519 {}", b64.encode(&pub_blob)),
        format!(
            "-----BEGIN SSH SIGNATURE-----\n{}\n-----END SSH SIGNATURE-----\n",
            b64.encode(&blob)
        ),
    )
}

/// Builds the maker signed by hand: the app checks the maker's key signed exactly this file.
#[tokio::test]
async fn hand_built_files_need_the_makers_signature() {
    let (base, fake) = fake_api().await;
    let zip = zip_of(&[("bin/game", GAME, 0o755)]);
    fake.files
        .lock()
        .unwrap()
        .push(("game.zip".into(), zip.clone()));
    let maker = SigningKey::from_bytes(&[30; 32]);
    let with_signature = |signer: &SigningKey| {
        let (public_key, signature) = ssh_sign(signer, &zip);
        let mut manifest: ReleaseManifest =
            serde_json::from_str(&manifest_for(&base, "game.zip", &zip, "r1").payload).unwrap();
        manifest.files[0].maker_signature = Some(slop_manifest::MakerSignature {
            // Slop vouches for the maker's registered key; here it's the signer's own line, or
            // the maker's when testing a stranger's signature.
            public_key: if signer == &maker {
                public_key
            } else {
                ssh_sign(&maker, &zip).0
            },
            signature,
        });
        slop_manifest::sign(&manifest, Purpose::ReleaseManifest, &key())
    };
    let dir = tempdir();
    let strict = Library::new(&dir, vec![key().verifying_key()])
        .unwrap()
        .allow_http_downloads(true)
        .require_maker_signature(true);
    let installed = strict
        .install(&with_signature(&maker), Platform::Linux, None, |_, _| {})
        .await
        .unwrap();
    assert_eq!(installed.version, "1.0.0");
    strict.uninstall("game").unwrap();
    let stranger = SigningKey::from_bytes(&[31; 32]);
    let err = strict
        .install(&with_signature(&stranger), Platform::Linux, None, |_, _| {})
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Verify(m) if m.contains("maker signature")),
        "{err:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// A file released in parts (over GitHub's 2 GiB limit, here just split in two).
fn parts_manifest(base: &str, whole: &[u8], parts: &[(&str, &[u8])]) -> Signed {
    let manifest = ReleaseManifest {
        schema: MANIFEST_SCHEMA,
        app_id: "id".into(),
        app_slug: "game".into(),
        app_title: "Game".into(),
        maker: "Octo".into(),
        repo_url: "https://github.com/octo/game".into(),
        release_id: "r1".into(),
        version: "1.0.0".into(),
        commit: "a".repeat(40),
        files: vec![ManifestFile {
            platform: Platform::Linux,
            kind: FileKind::Archive,
            url: format!("{base}files/game.zip"),
            file_name: "game.zip".into(),
            size: whole.len() as u64,
            sha256: hex::encode(Sha256::digest(whole)),
            launch: Some(Launch {
                exec: "bin/game".into(),
                args: vec!["hello".into()],
            }),
            attestation: None,
            maker_signature: None,
            parts: parts
                .iter()
                .map(|(name, bytes)| slop_manifest::FilePart {
                    url: format!("{base}files/{name}"),
                    size: bytes.len() as u64,
                    sha256: hex::encode(Sha256::digest(bytes)),
                    attestation: None,
                })
                .collect(),
        }],
        approved_at: "2026-10-18T00:00:00Z".into(),
        mod_for: None,
        build_of: None,
        mods_folder: None,
    };
    slop_manifest::sign(&manifest, Purpose::ReleaseManifest, &key())
}

#[tokio::test]
async fn files_in_parts_are_checked_and_joined() {
    let (base, fake) = fake_api().await;
    let zip = zip_of(&[("bin/game", GAME, 0o755), ("data/readme.txt", b"hi", 0o644)]);
    let (one, two) = zip.split_at(zip.len() / 2);
    let signed = parts_manifest(&base, &zip, &[("game.zip.001", one), ("game.zip.002", two)]);
    fake.files.lock().unwrap().extend([
        ("game.zip.001".to_string(), one.to_vec()),
        ("game.zip.002".to_string(), two.to_vec()),
    ]);
    let dir = tempdir();
    let lib = library(&dir);
    let seen = Mutex::new(0u64);
    lib.install(&signed, Platform::Linux, None, |got, total| {
        assert_eq!(total, zip.len() as u64);
        *seen.lock().unwrap() = got
    })
    .await
    .unwrap();
    assert_eq!(*seen.lock().unwrap(), zip.len() as u64);
    assert_eq!(
        std::fs::read(dir.join("apps/game/files/bin/game")).unwrap(),
        GAME
    );

    // A part swapped after Slop checked it: nothing is installed.
    lib.uninstall("game").unwrap();
    let mut swapped = two.to_vec();
    swapped[0] ^= 0xff;
    fake.files.lock().unwrap()[1].1 = swapped;
    let err = lib
        .install(&signed, Platform::Linux, None, |_, _| {})
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Verify(m) if m.contains("part 2")),
        "{err}"
    );
    assert!(lib.installed().is_empty());

    // Parts without the maker's signature aren't installed where signatures are required.
    fake.files.lock().unwrap()[1].1 = two.to_vec();
    let strict = Library::new(&dir, vec![key().verifying_key()])
        .unwrap()
        .allow_http_downloads(true);
    let err = strict
        .install(&signed, Platform::Linux, None, |_, _| {})
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Verify(m) if m.contains("part 1")),
        "{err}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// Sandbox mode on Windows (AppContainer): run on a Windows PC with
/// `cargo test -p slop-client --test client windows_sandbox`. Starts Windows' own cmd.exe in an
/// app's container and checks it reaches only the app's folders.
#[cfg(windows)]
#[test]
fn windows_sandbox_sees_only_its_own_folders() {
    use slop_client::sandbox::{Spec, container_name};
    let root = tempdir();
    let (files, saves, cache, mods) = (
        root.join("apps/game/files"),
        root.join("saves/game"),
        root.join("cache/game"),
        root.join("saves/game/mods"),
    );
    for d in [&files, &saves, &cache, &mods] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::copy(r"C:\Windows\System32\cmd.exe", files.join("cmd.exe")).unwrap();
    let secret = root.join("secret.txt");
    std::fs::write(&secret, b"password").unwrap();
    let p = |p: &std::path::Path| p.display().to_string();
    let script = format!(
        r#"type "{}" > "{}\copy.txt" & echo x> "{}\escape.txt" & echo x> "{}\new.txt" & echo x> "{}\cheat.txt" & echo ok> "{}\ok.txt""#,
        p(&secret),
        p(&saves),
        p(&root),
        p(&files),
        p(&mods),
        p(&saves)
    );
    let name = container_name("slop-test-game");
    let spec = Spec {
        name: name.clone(),
        network: false,
        read: vec![files.clone()],
        write: vec![saves.clone(), cache.clone()],
        read_only: vec![mods.clone()],
        cwd: files.clone(),
        program: files.join("cmd.exe"),
        args: vec!["/c".into(), script],
    };
    slop_client::appcontainer::run(&spec).unwrap();
    assert!(
        saves.join("ok.txt").exists(),
        "it ran and could write its saves"
    );
    let copied = std::fs::read(saves.join("copy.txt")).unwrap_or_default();
    assert!(!copied.starts_with(b"password"), "the secret stayed hidden");
    assert!(!root.join("escape.txt").exists(), "nothing written outside");
    assert!(!files.join("new.txt").exists(), "its files are read-only");
    assert!(
        !mods.join("cheat.txt").exists(),
        "its mods folder is read-only"
    );
    slop_client::appcontainer::remove(&name);
    let _ = std::fs::remove_dir_all(root);
}
