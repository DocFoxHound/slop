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
        }],
        approved_at: "2026-10-18T00:00:00Z".into(),
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
