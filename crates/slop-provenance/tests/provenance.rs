// SPDX-License-Identifier: MIT OR Apache-2.0
//! Against a real GitHub Actions attestation (see fixtures/README.md).

use slop_provenance::{Expected, ProvenanceError, ProvenanceVerifier};

const BUNDLE: &str = include_str!("fixtures/github-actions-provenance.sigstore.json");
const NOT_GITHUB: &str = include_str!("fixtures/dsse.sigstore.json");
const SHA512: &str = "46d4e2f74c4877316640000a6fdf8a8b59f1e0847667973e9859f774dd31b8f1e0937813b777fb66a2ac67d50540fe34640966eee9fc2ccca387082b4c85cd3c";
const REPO: &str = "https://github.com/sigstore/sigstore-js";
const COMMIT: &str = "f0b49a04e5a62250e0f60fb128004a73110fe311";

fn expected() -> Expected<'static> {
    Expected {
        repo_url: REPO,
        commit: COMMIT,
        slop_build_workflow: None,
    }
}

#[test]
fn a_real_attestation_verifies_and_names_its_build() {
    let v = ProvenanceVerifier::public_good().unwrap();
    let p = v.verify_sha512(BUNDLE, SHA512, expected()).unwrap();
    assert_eq!(p.source_repo, REPO);
    assert_eq!(p.commit, COMMIT);
    assert_eq!(
        p.signer_workflow,
        "https://github.com/sigstore/sigstore-js/.github/workflows/release.yml@refs/heads/main"
    );
    assert_eq!(p.runner.as_deref(), Some("github-hosted"));
    assert!(!p.built_by_slop_build);
    assert!(
        p.run_url
            .as_deref()
            .unwrap()
            .starts_with("https://github.com/sigstore/sigstore-js/actions/runs/")
    );
}

#[test]
fn the_slop_build_recipe_is_recognised() {
    let v = ProvenanceVerifier::public_good().unwrap();
    let e = Expected {
        slop_build_workflow: Some(
            "https://github.com/sigstore/sigstore-js/.github/workflows/release.yml@",
        ),
        ..expected()
    };
    assert!(
        v.verify_sha512(BUNDLE, SHA512, e)
            .unwrap()
            .built_by_slop_build
    );
}

#[test]
fn another_repo_or_commit_is_refused() {
    let v = ProvenanceVerifier::public_good().unwrap();
    // Someone else's genuine attestation can't vouch for this app.
    let other = Expected {
        repo_url: "https://github.com/evil/sigstore-js",
        ..expected()
    };
    assert!(matches!(
        v.verify_sha512(BUNDLE, SHA512, other),
        Err(ProvenanceError::Mismatch(_))
    ));
    let other = Expected {
        commit: "0000000000000000000000000000000000000000",
        ..expected()
    };
    assert!(matches!(
        v.verify_sha512(BUNDLE, SHA512, other),
        Err(ProvenanceError::Mismatch(_))
    ));
    // Case and a .git suffix don't matter.
    let same = Expected {
        repo_url: "https://github.com/Sigstore/sigstore-js.git",
        ..expected()
    };
    assert!(v.verify_sha512(BUNDLE, SHA512, same).is_ok());
}

#[test]
fn another_file_is_refused() {
    let v = ProvenanceVerifier::public_good().unwrap();
    let mut wrong = SHA512.to_string();
    wrong.replace_range(0..2, "00");
    assert!(matches!(
        v.verify_sha512(BUNDLE, &wrong, expected()),
        Err(ProvenanceError::Mismatch(_))
    ));
    assert!(matches!(
        v.verify_sha256(BUNDLE, &"ab".repeat(32), expected()),
        Err(ProvenanceError::Mismatch(_))
    ));
}

#[test]
fn a_tampered_statement_is_refused() {
    let v = ProvenanceVerifier::public_good().unwrap();
    // Point the statement at a different file: the signature no longer matches.
    let mut bundle: serde_json::Value = serde_json::from_str(BUNDLE).unwrap();
    let payload = bundle["dsseEnvelope"]["payload"]
        .as_str()
        .unwrap()
        .to_string();
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let statement = String::from_utf8(engine.decode(payload).unwrap()).unwrap();
    let forged_digest = "11".repeat(64);
    let forged = statement.replace(SHA512, &forged_digest);
    bundle["dsseEnvelope"]["payload"] = serde_json::Value::String(engine.encode(forged));
    let err = v
        .verify_sha512(&bundle.to_string(), &forged_digest, expected())
        .unwrap_err();
    assert!(matches!(err, ProvenanceError::Signature(_)), "{err}");
}

#[test]
fn non_github_signatures_are_refused() {
    let v = ProvenanceVerifier::public_good().unwrap();
    let err = v
        .verify_sha256(NOT_GITHUB, &"ab".repeat(32), expected())
        .unwrap_err();
    assert!(
        !matches!(err, ProvenanceError::Malformed(ref m) if m.contains("SHA-256")),
        "{err}"
    );
}

#[test]
fn garbage_is_malformed() {
    let v = ProvenanceVerifier::public_good().unwrap();
    assert!(matches!(
        v.verify_sha256("{}", &"ab".repeat(32), expected()),
        Err(ProvenanceError::Malformed(_))
    ));
}
