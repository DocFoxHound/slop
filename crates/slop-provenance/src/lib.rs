// SPDX-License-Identifier: MIT OR Apache-2.0
//! Maker signatures (docs/SECURITY.md §5.3).
//!
//! Every build must carry its maker's signature as well as Slop's. The default is a GitHub
//! artifact attestation: the maker's CI asks Sigstore for a short-lived certificate that says
//! "workflow Y in repo X at commit Z", signs a SLSA provenance statement naming the file's hash,
//! and logs it in Sigstore's public transparency log. There is no long-lived key to steal, and a
//! stolen Slop session alone can't produce one.
//!
//! [`ProvenanceVerifier`] checks such an attestation end to end (certificate chain to Sigstore's
//! root, transparency log, signature, issuer) and then checks that it names *this* file, built
//! from *this app's repository* at *this release's commit*. Both the API (before review) and the
//! desktop client (before installing) run the same check, so a compromised Slop server can't
//! slip in a build the maker didn't sign.

use serde::{Deserialize, Serialize};
use sigstore_verify::{
    VerificationPolicy,
    trust_root::{SigstoreInstance, TrustedRoot},
    types::{ArtifactDigest, Bundle, HashAlgorithm, SignatureContent},
};

/// OIDC issuer of GitHub Actions identity tokens.
pub const GITHUB_ACTIONS_ISSUER: &str = "https://token.actions.githubusercontent.com";
pub const IN_TOTO_PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";
pub const SLSA_PROVENANCE_V1: &str = "https://slsa.dev/provenance/v1";

/// Who built a file, as proven by its attestation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// The repository the build came from, e.g. `https://github.com/owner/repo`.
    pub source_repo: String,
    /// The commit it was built from.
    pub commit: String,
    /// The workflow that signed it (with its ref). For Slop Build, Slop's shared recipe.
    pub signer_workflow: String,
    /// Signed by Slop Build's recipe: shown as "Built by Slop Build from source".
    pub built_by_slop_build: bool,
    /// `github-hosted` or `self-hosted`. Self-hosted runners are controlled by the maker.
    pub runner: Option<String>,
    /// The CI run, where the build log can be inspected.
    pub run_url: Option<String>,
}

/// What an attestation must match.
#[derive(Clone, Copy, Debug)]
pub struct Expected<'a> {
    /// The app's linked repository.
    pub repo_url: &'a str,
    /// The release's commit (40 hex characters).
    pub commit: &'a str,
    /// Prefix of Slop Build's recipe workflow, e.g.
    /// `https://github.com/<org>/slop-build/.github/workflows/build.yml@`. `None` until Slop Build
    /// exists.
    pub slop_build_workflow: Option<&'a str>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProvenanceError {
    #[error("the attestation is malformed: {0}")]
    Malformed(String),
    #[error("the attestation's signature doesn't check out: {0}")]
    Signature(String),
    #[error("the attestation is for something else: {0}")]
    Mismatch(String),
}

/// Normalises a GitHub-style repository URL for comparison: lowercase host and path, no trailing
/// slash or `.git`.
fn normalize_repo(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_ascii_lowercase()
}

#[derive(Deserialize)]
struct Statement {
    #[serde(rename = "_type")]
    type_: String,
    #[serde(rename = "predicateType")]
    predicate_type: String,
    subject: Vec<Subject>,
}

#[derive(Deserialize)]
struct Subject {
    #[serde(default)]
    digest: std::collections::BTreeMap<String, String>,
}

pub struct ProvenanceVerifier {
    /// Sigstore's public-good instance: GitHub uses it for public repositories.
    public_good: TrustedRoot,
    /// GitHub's own Sigstore instance: used for private and internal repositories. It timestamps
    /// signatures with its own timestamp authority instead of logging them publicly.
    github: TrustedRoot,
}

impl ProvenanceVerifier {
    /// Trusts both instances GitHub signs attestations with. The trusted roots are the snapshots
    /// embedded in `sigstore-trust-root`; refreshing them over TUF is a later step.
    pub fn public_good() -> Result<Self, ProvenanceError> {
        let root = |instance| {
            TrustedRoot::from_embedded(instance)
                .map_err(|e| ProvenanceError::Signature(format!("Sigstore trusted root: {e}")))
        };
        Ok(Self {
            public_good: root(SigstoreInstance::PublicGood)?,
            github: root(SigstoreInstance::GitHub)?,
        })
    }

    /// Cryptographic verification against either instance. Public-good signatures must be in
    /// the transparency log; GitHub-instance signatures must carry a verified timestamp instead.
    fn sigstore_verify(
        &self,
        artifact: ArtifactDigest,
        bundle: &Bundle,
        issuer: Option<&str>,
    ) -> Result<sigstore_verify::VerificationResult, ProvenanceError> {
        let with_issuer = |p: VerificationPolicy| match issuer {
            Some(i) => p.require_issuer(i),
            None => p,
        };
        let public = sigstore_verify::verify(
            artifact.clone(),
            bundle,
            &with_issuer(VerificationPolicy::any_identity()),
            &self.public_good,
        );
        match public {
            Ok(result) if result.certificate_verified() && result.tlog_verified() => {
                return Ok(result);
            }
            Ok(_) => {}
            Err(_) => {}
        }
        let policy = with_issuer(
            VerificationPolicy::any_identity()
                .skip_tlog_unsafe()
                .skip_sct(),
        );
        let result = sigstore_verify::verify(artifact, bundle, &policy, &self.github)
            .map_err(|e| ProvenanceError::Signature(e.to_string()))?;
        if !result.certificate_verified() || result.verified_timestamps().is_empty() {
            return Err(ProvenanceError::Signature(
                "certificate or signing time wasn't verified".into(),
            ));
        }
        Ok(result)
    }

    /// Verifies an attestation for a file with this SHA-256 (lowercase hex).
    pub fn verify_sha256(
        &self,
        bundle_json: &str,
        sha256_hex: &str,
        expected: Expected<'_>,
    ) -> Result<Provenance, ProvenanceError> {
        let bytes = hex::decode(sha256_hex)
            .ok()
            .filter(|b| b.len() == 32)
            .ok_or_else(|| ProvenanceError::Malformed("not a SHA-256 hex digest".into()))?;
        self.verify(
            bundle_json,
            "sha256",
            &bytes,
            HashAlgorithm::Sha2256,
            expected,
        )
    }

    /// Verifies an attestation for a file with this SHA-512. Only used with npm-style fixtures.
    pub fn verify_sha512(
        &self,
        bundle_json: &str,
        sha512_hex: &str,
        expected: Expected<'_>,
    ) -> Result<Provenance, ProvenanceError> {
        let bytes = hex::decode(sha512_hex)
            .ok()
            .filter(|b| b.len() == 64)
            .ok_or_else(|| ProvenanceError::Malformed("not a SHA-512 hex digest".into()))?;
        self.verify(
            bundle_json,
            "sha512",
            &bytes,
            HashAlgorithm::Sha2512,
            expected,
        )
    }

    fn verify(
        &self,
        bundle_json: &str,
        digest_name: &str,
        digest: &[u8],
        algorithm: HashAlgorithm,
        expected: Expected<'_>,
    ) -> Result<Provenance, ProvenanceError> {
        let bundle = Bundle::from_json(bundle_json)
            .map_err(|e| ProvenanceError::Malformed(e.to_string()))?;

        // 1. It must be a SLSA provenance statement that names this exact file.
        let SignatureContent::DsseEnvelope(envelope) = &bundle.content else {
            return Err(ProvenanceError::Malformed(
                "not an attestation (no DSSE envelope)".into(),
            ));
        };
        if envelope.payload_type != IN_TOTO_PAYLOAD_TYPE {
            return Err(ProvenanceError::Malformed(format!(
                "unexpected payload type {}",
                envelope.payload_type
            )));
        }
        let statement: Statement = serde_json::from_slice(envelope.payload.as_bytes())
            .map_err(|e| ProvenanceError::Malformed(format!("bad in-toto statement: {e}")))?;
        if !statement.type_.starts_with("https://in-toto.io/Statement/") {
            return Err(ProvenanceError::Malformed(
                "not an in-toto statement".into(),
            ));
        }
        if statement.predicate_type != SLSA_PROVENANCE_V1 {
            return Err(ProvenanceError::Mismatch(format!(
                "it's a {} attestation, not SLSA build provenance",
                statement.predicate_type
            )));
        }
        let want = hex::encode(digest);
        if !statement.subject.iter().any(|s| {
            s.digest
                .get(digest_name)
                .is_some_and(|d| d.eq_ignore_ascii_case(&want))
        }) {
            return Err(ProvenanceError::Mismatch(
                "it doesn't name this file's hash".into(),
            ));
        }

        // 2. Full Sigstore verification: certificate chain, transparency log, signature, issuer.
        let artifact = ArtifactDigest::new(algorithm, digest.to_vec())
            .map_err(|e| ProvenanceError::Malformed(e.to_string()))?;
        let result = self.sigstore_verify(artifact, &bundle, Some(GITHUB_ACTIONS_ISSUER))?;
        let cert = result
            .certificate()
            .ok_or_else(|| ProvenanceError::Signature("no signing certificate".into()))?;
        let claims = &cert.ci_claims;

        // 3. It must come from this app's repository, at this release's commit.
        let source_repo = claims.source_repository_uri.clone().ok_or_else(|| {
            ProvenanceError::Mismatch("the certificate names no source repository".into())
        })?;
        if normalize_repo(&source_repo) != normalize_repo(expected.repo_url) {
            return Err(ProvenanceError::Mismatch(format!(
                "built from {source_repo}, not {}",
                expected.repo_url
            )));
        }
        let commit = claims
            .source_repository_digest
            .clone()
            .ok_or_else(|| ProvenanceError::Mismatch("the certificate names no commit".into()))?;
        if !commit.eq_ignore_ascii_case(expected.commit) {
            return Err(ProvenanceError::Mismatch(format!(
                "built from commit {commit}, not {}",
                expected.commit
            )));
        }
        let signer_workflow = claims
            .build_signer_uri
            .clone()
            .or_else(|| result.identity().map(|i| i.as_str().to_string()))
            .ok_or_else(|| ProvenanceError::Mismatch("the certificate names no workflow".into()))?;
        let built_by_slop_build = expected
            .slop_build_workflow
            .is_some_and(|prefix| !prefix.is_empty() && signer_workflow.starts_with(prefix));

        Ok(Provenance {
            source_repo,
            commit: commit.to_ascii_lowercase(),
            signer_workflow,
            built_by_slop_build,
            runner: claims.runner_environment.clone(),
            run_url: claims.run_invocation_uri.clone(),
        })
    }
}

/// `owner/repo` of a GitHub repository URL, for the attestations API.
pub fn github_repo_path(repo_url: &str) -> Option<String> {
    let rest = repo_url.trim().strip_prefix("https://github.com/")?;
    let path = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    (parts.next().is_none() && ok(owner) && ok(repo) && repo != "." && repo != "..")
        .then(|| format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_paths() {
        assert_eq!(
            github_repo_path("https://github.com/Octo/Game").as_deref(),
            Some("Octo/Game")
        );
        assert_eq!(
            github_repo_path("https://github.com/octo/game.git/").as_deref(),
            Some("octo/game")
        );
        assert_eq!(github_repo_path("https://gitlab.com/octo/game"), None);
        assert_eq!(
            github_repo_path("https://github.com/octo/game/tree/main"),
            None
        );
        assert_eq!(github_repo_path("https://github.com/octo/.."), None);
    }

    /// GitHub's own instance (private repositories): verified with GitHub's root and a timestamp.
    #[test]
    fn github_instance_signatures_verify() {
        let v = ProvenanceVerifier::public_good().unwrap();
        let json = include_str!("../tests/fixtures/github-private-attestation.sigstore.json");
        let bundle = Bundle::from_json(json).unwrap();
        let digest = |hex_digest: &str| {
            ArtifactDigest::new(HashAlgorithm::Sha2256, hex::decode(hex_digest).unwrap()).unwrap()
        };
        let ok = v.sigstore_verify(
            digest("76f1fe8593bf227cca2c089e3c16dc95014a8d3e89c5dd220530469ca043c428"),
            &bundle,
            None,
        );
        assert!(ok.is_ok(), "{:?}", ok.err());
        assert!(!ok.unwrap().verified_timestamps().is_empty());
        // Another file's hash doesn't verify.
        let bad = v.sigstore_verify(digest(&"00".repeat(32)), &bundle, None);
        assert!(bad.is_err());
        // It isn't a GitHub Actions build, so the Actions issuer requirement fails.
        let wrong_issuer = v.sigstore_verify(
            digest("76f1fe8593bf227cca2c089e3c16dc95014a8d3e89c5dd220530469ca043c428"),
            &bundle,
            Some(GITHUB_ACTIONS_ISSUER),
        );
        assert!(wrong_issuer.is_err());
    }

    #[test]
    fn repo_comparison_ignores_case_and_suffixes() {
        assert_eq!(
            normalize_repo("https://github.com/Octo/Game.git/"),
            normalize_repo("https://github.com/octo/game")
        );
    }
}
