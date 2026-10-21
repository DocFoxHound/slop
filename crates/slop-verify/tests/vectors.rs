// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Slop open standard's test files (docs/standard/vectors/): made here with a published test
//! key, so they never change unless the formats do, and checked with the `slop-verify` program.
//! `SLOP_UPDATE_VECTORS=1 cargo test -p slop-verify` rewrites them after a deliberate change.

use std::{path::PathBuf, process::Command};

use sha2::{Digest, Sha256};
use slop_manifest::{
    FileKind, FileMatch, FileVerification, Launch, MANIFEST_SCHEMA, ManifestFile, MatchStatus,
    Platform, Purpose, REVOCATIONS_SCHEMA, ReleaseManifest, RevocationList, Revoked,
    VERIFICATION_SCHEMA, ed25519_dalek::SigningKey,
};

fn vectors() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/standard/vectors")
}

/// The test key: SHA-256 of a public phrase. Never trust it for anything real.
fn test_key() -> SigningKey {
    SigningKey::from_bytes(&Sha256::digest(b"slop open standard test key").into())
}

const EXAMPLE_FILE: &[u8] = b"Not a real game: the Slop open standard's example file.\n";

fn documents() -> Vec<(&'static str, String)> {
    let key = test_key();
    let sha256 = hex::encode(Sha256::digest(EXAMPLE_FILE));
    let manifest = ReleaseManifest {
        schema: MANIFEST_SCHEMA,
        app_id: "0192f0c4-0000-7000-8000-000000000001".into(),
        app_slug: "example-game".into(),
        app_title: "Example Game".into(),
        maker: "Example Maker".into(),
        repo_url: "https://github.com/example/example-game".into(),
        release_id: "0192f0c4-0000-7000-8000-000000000002".into(),
        version: "1.0.0".into(),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        files: vec![ManifestFile {
            platform: Platform::Windows,
            kind: FileKind::Archive,
            url:
                "https://github.com/example/example-game/releases/download/v1.0.0/example-file.bin"
                    .into(),
            file_name: "example-file.bin".into(),
            size: EXAMPLE_FILE.len() as u64,
            sha256: sha256.clone(),
            launch: Some(Launch {
                exec: "ExampleGame.exe".into(),
                args: vec![],
            }),
            attestation: None,
            maker_signature: None,
            parts: vec![],
        }],
        approved_at: "2026-10-21T12:00:00Z".into(),
        mod_for: None,
        build_of: None,
        mods_folder: None,
        runtimes: vec!["vcredist-x64".into()],
    };
    let revocations = RevocationList {
        schema: REVOCATIONS_SCHEMA,
        issued_at: "2026-10-21T12:00:00Z".into(),
        expires_at: "2099-12-31T00:00:00Z".into(),
        revoked: vec![Revoked {
            release_id: "0192f0c4-0000-7000-8000-000000000003".into(),
            app_slug: "bad-example".into(),
            reason: "Malware found by re-scan".into(),
            revoked_at: "2026-10-21T11:00:00Z".into(),
        }],
    };
    let verification = FileVerification {
        schema: VERIFICATION_SCHEMA,
        sha256,
        issued_at: "2026-10-21T12:00:00Z".into(),
        expires_at: "2099-12-31T00:00:00Z".into(),
        matches: vec![FileMatch {
            app_slug: "example-game".into(),
            app_title: "Example Game".into(),
            maker: "Example Maker".into(),
            release_id: "0192f0c4-0000-7000-8000-000000000002".into(),
            version: "1.0.0".into(),
            platform: Platform::Windows,
            status: MatchStatus::Approved,
            approved_at: "2026-10-21T12:00:00Z".into(),
            revoked_at: None,
            scan: "passed".into(),
            defender: Some("clean".into()),
            page_url: "https://slop.example/app/example-game".into(),
        }],
    };
    let pretty = |v: &slop_manifest::Signed| serde_json::to_string_pretty(v).unwrap() + "\n";
    let signed_manifest = slop_manifest::sign(&manifest, Purpose::ReleaseManifest, &key);
    let mut tampered = signed_manifest.clone();
    tampered.payload = tampered.payload.replace("1.0.0", "1.0.1");
    vec![
        (
            "test-key.json",
            serde_json::to_string_pretty(&serde_json::json!({
                "warning": "A published test key. Never trust it for anything real.",
                "secret_key": slop_manifest::signing_key_to_base64(&key),
                "public_key": slop_manifest::verifying_key_to_base64(&key.verifying_key()),
                "key_id": slop_manifest::key_id(&key.verifying_key()),
            }))
            .unwrap()
                + "\n",
        ),
        ("release-manifest.json", pretty(&signed_manifest)),
        ("release-manifest.tampered.json", pretty(&tampered)),
        (
            "revocation-list.json",
            pretty(&slop_manifest::sign(
                &revocations,
                Purpose::RevocationList,
                &key,
            )),
        ),
        (
            "file-verification.json",
            pretty(&slop_manifest::sign(
                &verification,
                Purpose::FileVerification,
                &key,
            )),
        ),
        (
            "example-file.bin",
            String::from_utf8(EXAMPLE_FILE.to_vec()).unwrap(),
        ),
    ]
}

#[test]
fn the_test_files_are_current() {
    let dir = vectors();
    let update = std::env::var("SLOP_UPDATE_VECTORS").is_ok_and(|v| v == "1");
    if update {
        std::fs::create_dir_all(&dir).unwrap();
    }
    for (name, content) in documents() {
        let path = dir.join(name);
        if update {
            std::fs::write(&path, &content).unwrap();
        } else {
            assert_eq!(
                std::fs::read_to_string(&path).unwrap_or_default(),
                content,
                "{name} is out of date: run SLOP_UPDATE_VECTORS=1 cargo test -p slop-verify"
            );
        }
    }
}

fn slop_verify(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_slop-verify"))
        .args(args)
        .current_dir(vectors())
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    )
}

fn public_key() -> String {
    slop_manifest::verifying_key_to_base64(&test_key().verifying_key())
}

#[test]
fn slop_verify_accepts_the_test_files() {
    let key = public_key();
    let (code, out) = slop_verify(&[
        "manifest",
        "release-manifest.json",
        "--key",
        &key,
        "--file",
        "example-file.bin",
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Example Game 1.0.0 by Example Maker"), "{out}");
    assert!(
        out.contains("example-file.bin: matches the windows file"),
        "{out}"
    );

    let (code, out) = slop_verify(&["revocations", "revocation-list.json", "--key", &key]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("1 revoked"), "{out}");

    let (code, out) = slop_verify(&[
        "file",
        "file-verification.json",
        "--key",
        &key,
        "--file",
        "example-file.bin",
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("APPROVED: Example Game 1.0.0"), "{out}");

    let (code, out) = slop_verify(&["sha256", "example-file.bin"]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), hex::encode(Sha256::digest(EXAMPLE_FILE)));
}

#[test]
fn slop_verify_refuses_what_doesnt_check_out() {
    let key = public_key();
    // Changed after signing.
    let (code, out) = slop_verify(&["manifest", "release-manifest.tampered.json", "--key", &key]);
    assert_eq!(code, 1, "{out}");
    // Signed by someone else.
    let other = slop_manifest::verifying_key_to_base64(
        &SigningKey::from_bytes(&rand::random::<[u8; 32]>()).verifying_key(),
    );
    let (code, _) = slop_verify(&["manifest", "release-manifest.json", "--key", &other]);
    assert_eq!(code, 1);
    // Signed for another purpose: a manifest isn't a file verification.
    let (code, _) = slop_verify(&["file", "release-manifest.json", "--key", &key]);
    assert_eq!(code, 1);
    // A file that isn't in the release.
    let (code, out) = slop_verify(&[
        "manifest",
        "release-manifest.json",
        "--key",
        &key,
        "--file",
        "test-key.json",
    ]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("isn't in this release"), "{out}");
    // Usage mistakes.
    assert_eq!(slop_verify(&["manifest", "release-manifest.json"]).0, 2);
    assert_eq!(slop_verify(&[]).0, 2);
}

/// Every field the code writes is in the JSON Schema, and every field in the schema is one the
/// code writes: checked on documents with every optional field filled in.
#[test]
fn the_schemas_match_the_code() {
    use serde_json::Value;
    fn same_fields(value: &Value, schema: &Value, at: &str) {
        let schema = schema
            .get("oneOf")
            .and_then(|v| v.as_array())
            .and_then(|o| o.iter().find(|s| s["type"] == "object"))
            .unwrap_or(schema);
        match value {
            Value::Object(map) => {
                let props = schema["properties"]
                    .as_object()
                    .unwrap_or_else(|| panic!("{at}: the schema has no properties"));
                let mut written: Vec<_> = map.keys().cloned().collect();
                let mut listed: Vec<_> = props.keys().cloned().collect();
                written.sort();
                listed.sort();
                assert_eq!(written, listed, "{at}");
                for (k, v) in map {
                    same_fields(v, &props[k], &format!("{at}.{k}"));
                }
            }
            Value::Array(items) => {
                if let Some(first) = items.first() {
                    same_fields(first, &schema["items"], &format!("{at}[0]"));
                }
            }
            _ => {}
        }
    }
    let schema = |name: &str| -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(vectors().join("../schemas").join(name)).unwrap(),
        )
        .unwrap()
    };
    let docs = documents();
    let payload = |name: &str| -> Value {
        let signed: slop_manifest::Signed =
            serde_json::from_str(&docs.iter().find(|(n, _)| *n == name).unwrap().1).unwrap();
        serde_json::from_str(&signed.payload).unwrap()
    };
    // The manifest with every optional field.
    let mut manifest = payload("release-manifest.json");
    let app = serde_json::json!({ "app_id": "a", "app_slug": "b", "app_title": "C" });
    manifest["mod_for"] = app.clone();
    manifest["build_of"] = app;
    manifest["mods_folder"] =
        serde_json::json!({ "base": "game", "path": "mods", "layout": "folder" });
    let file = &mut manifest["files"][0];
    file["maker_signature"] = serde_json::json!({ "public_key": "k", "signature": "s" });
    file["parts"] = serde_json::json!([{ "url": "u", "size": 1, "sha256": "0".repeat(64), "attestation": null }]);
    // Round-trip through the real type, so only fields the code knows survive.
    let manifest: ReleaseManifest = serde_json::from_value(manifest).unwrap();
    same_fields(
        &serde_json::to_value(&manifest).unwrap(),
        &schema("release-manifest.schema.json"),
        "manifest",
    );
    same_fields(
        &payload("revocation-list.json"),
        &schema("revocation-list.schema.json"),
        "revocations",
    );
    let mut verification = payload("file-verification.json");
    verification["matches"][0]["revoked_at"] = "2026-10-21T13:00:00Z".into();
    let verification: FileVerification = serde_json::from_value(verification).unwrap();
    same_fields(
        &serde_json::to_value(&verification).unwrap(),
        &schema("file-verification.schema.json"),
        "file verification",
    );
    let envelope: Value = serde_json::from_str(
        &docs
            .iter()
            .find(|(n, _)| *n == "release-manifest.json")
            .unwrap()
            .1,
    )
    .unwrap();
    same_fields(&envelope, &schema("signed.schema.json"), "envelope");
}
