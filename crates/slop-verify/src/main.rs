// SPDX-License-Identifier: MIT OR Apache-2.0
//! `slop-verify`: check Slop's signed documents yourself, without trusting a Slop server (the
//! Slop open standard, docs/standard/README.md).
//!
//! ```text
//! slop-verify manifest <signed.json> --key <key> [--file <path>]... [--attestations] [--slop-build <prefix>]
//! slop-verify revocations <signed.json> --key <key>
//! slop-verify file <signed.json> --key <key> [--file <path>]
//! slop-verify sha256 <path>
//! ```
//!
//! `<key>` is one of Slop's public keys (`public_key` from `GET /v1/keys`); repeat `--key` for
//! several. Exit status: 0 verified, 1 not verified, 2 a usage mistake.

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::ExitCode,
};

use sha2::{Digest, Sha256, Sha512};
use slop_manifest::{
    FileVerification, MatchStatus, Purpose, ReleaseManifest, RevocationList, Signed,
    ed25519_dalek::VerifyingKey, sshsig,
};
use slop_provenance::{Expected, ProvenanceVerifier};
use time::OffsetDateTime;

const USAGE: &str = "\
usage:
  slop-verify manifest <signed.json> --key <key> [--file <path>]... [--attestations] [--slop-build <prefix>]
  slop-verify revocations <signed.json> --key <key>
  slop-verify file <signed.json> --key <key> [--file <path>]
  slop-verify sha256 <path>

<key> is a Slop public key (public_key from GET /v1/keys); repeat --key for several.
--file checks a downloaded file belongs to the document.
--attestations also checks each file's maker signature (a Sigstore attestation) offline.
--slop-build names Slop Build's workflow prefix, to report \"built by Slop Build\".";

enum Fail {
    Usage(String),
    NotVerified(String),
}

#[derive(Default)]
struct Args {
    doc: Option<PathBuf>,
    keys: Vec<VerifyingKey>,
    files: Vec<PathBuf>,
    attestations: bool,
    slop_build: Option<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(Fail::Usage(message)) => {
            eprintln!("{message}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Fail::NotVerified(message)) => {
            eprintln!("NOT VERIFIED: {message}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &[String]) -> Result<Vec<String>, Fail> {
    let (command, rest) = args
        .split_first()
        .ok_or_else(|| Fail::Usage("no command".into()))?;
    if command == "sha256" {
        let [path] = rest else {
            return Err(Fail::Usage("sha256 takes one file".into()));
        };
        let (sha256, _) = hash_file(Path::new(path))?;
        return Ok(vec![sha256]);
    }
    let args = parse(rest)?;
    let doc = args
        .doc
        .as_deref()
        .ok_or_else(|| Fail::Usage("which document?".into()))?;
    if args.keys.is_empty() {
        return Err(Fail::Usage("give Slop's public key with --key".into()));
    }
    let text = std::fs::read_to_string(doc)
        .map_err(|e| Fail::Usage(format!("can't read {}: {e}", doc.display())))?;
    let signed: Signed = serde_json::from_str(&text)
        .map_err(|e| Fail::NotVerified(format!("not a signed Slop document: {e}")))?;
    match command.as_str() {
        "manifest" => manifest(&signed, &args),
        "revocations" => revocations(&signed, &args),
        "file" => file(&signed, &args),
        other => Err(Fail::Usage(format!("unknown command {other}"))),
    }
}

fn parse(rest: &[String]) -> Result<Args, Fail> {
    let mut args = Args::default();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| Fail::Usage(format!("{name} needs a value")))
        };
        match arg.as_str() {
            "--key" => {
                let v = value("--key")?;
                let key = slop_manifest::verifying_key_from_base64(&v)
                    .ok_or_else(|| Fail::Usage(format!("not an Ed25519 public key: {v}")))?;
                args.keys.push(key);
            }
            "--file" => args.files.push(value("--file")?.into()),
            "--attestations" => args.attestations = true,
            "--slop-build" => args.slop_build = Some(value("--slop-build")?),
            flag if flag.starts_with("--") => {
                return Err(Fail::Usage(format!("unknown option {flag}")));
            }
            path if args.doc.is_none() => args.doc = Some(path.into()),
            extra => return Err(Fail::Usage(format!("unexpected {extra}"))),
        }
    }
    Ok(args)
}

fn check<T: serde::de::DeserializeOwned>(
    signed: &Signed,
    purpose: Purpose,
    args: &Args,
) -> Result<T, Fail> {
    slop_manifest::verify(signed, purpose, &args.keys)
        .map_err(|e| Fail::NotVerified(format!("Slop's signature: {e}")))
}

/// Lowercase hex SHA-256 and the raw SHA-512 of a file, read once.
fn hash_file(path: &Path) -> Result<(String, [u8; 64]), Fail> {
    let mut f =
        File::open(path).map_err(|e| Fail::Usage(format!("can't read {}: {e}", path.display())))?;
    let (mut s256, mut s512) = (Sha256::new(), Sha512::new());
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| Fail::Usage(format!("can't read {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        s256.update(&buf[..n]);
        s512.update(&buf[..n]);
    }
    Ok((hex::encode(s256.finalize()), s512.finalize().into()))
}

fn manifest(signed: &Signed, args: &Args) -> Result<Vec<String>, Fail> {
    let m: ReleaseManifest = check(signed, Purpose::ReleaseManifest, args)?;
    let mut out = vec![
        format!("Signed by Slop (key {})", signed.key_id),
        format!(
            "{} {} by {}, approved {}",
            m.app_title, m.version, m.maker, m.approved_at
        ),
        format!("Source: {} at commit {}", m.repo_url, m.commit),
    ];
    for f in &m.files {
        out.push(format!(
            "  {} {}: {} ({} bytes, sha256 {})",
            f.platform.as_str(),
            f.kind.as_str(),
            f.file_name,
            f.size,
            f.sha256
        ));
    }
    if let Some(of) = &m.mod_for {
        out.push(format!("A mod for {} ({})", of.app_title, of.app_slug));
    }
    if let Some(of) = &m.build_of {
        out.push(format!(
            "A community build of {} ({})",
            of.app_title, of.app_slug
        ));
    }
    for path in &args.files {
        let (sha256, sha512) = hash_file(path)?;
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let whole = m.files.iter().find(|f| f.sha256 == sha256);
        let part = m
            .files
            .iter()
            .flat_map(|f| f.parts.iter().enumerate().map(move |(i, p)| (f, i, p)))
            .find(|(_, _, p)| p.sha256 == sha256);
        match (whole, part) {
            (Some(f), _) => {
                if f.size != size {
                    return Err(Fail::NotVerified(format!(
                        "{} has the right hash but not the right size",
                        path.display()
                    )));
                }
                out.push(format!(
                    "{}: matches the {} file {}",
                    path.display(),
                    f.platform.as_str(),
                    f.file_name
                ));
                if let Some(sig) = &f.maker_signature {
                    let (key, _) = sshsig::parse_public_key(&sig.public_key).map_err(|e| {
                        Fail::NotVerified(format!("the maker's key in the manifest: {e}"))
                    })?;
                    let sha256_bytes: [u8; 32] = hex::decode(&sha256)
                        .ok()
                        .and_then(|b| b.try_into().ok())
                        .expect("our own hex");
                    sshsig::verify(&key, &sig.signature, &sha512, &sha256_bytes).map_err(|e| {
                        Fail::NotVerified(format!("the maker's SSH signature: {e}"))
                    })?;
                    out.push(format!(
                        "{}: the maker's SSH signature checks out (key {})",
                        path.display(),
                        sshsig::fingerprint(&key)
                    ));
                }
            }
            (None, Some((f, i, p))) => {
                if p.size != size {
                    return Err(Fail::NotVerified(format!(
                        "{} has the right hash but not the right size",
                        path.display()
                    )));
                }
                out.push(format!(
                    "{}: matches part {} of the {} file {}",
                    path.display(),
                    i + 1,
                    f.platform.as_str(),
                    f.file_name
                ));
            }
            (None, None) => {
                return Err(Fail::NotVerified(format!(
                    "{} (sha256 {sha256}) isn't in this release",
                    path.display()
                )));
            }
        }
    }
    if args.attestations {
        let verifier = ProvenanceVerifier::public_good()
            .map_err(|e| Fail::NotVerified(format!("Sigstore: {e}")))?;
        let expected = Expected {
            repo_url: &m.repo_url,
            commit: &m.commit,
            slop_build_workflow: args.slop_build.as_deref(),
        };
        for f in &m.files {
            let pieces: Vec<(String, &str, Option<&String>)> = if f.parts.is_empty() {
                vec![(
                    f.file_name.clone(),
                    f.sha256.as_str(),
                    f.attestation.as_ref(),
                )]
            } else {
                f.parts
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        (
                            format!("{} part {}", f.file_name, i + 1),
                            p.sha256.as_str(),
                            p.attestation.as_ref(),
                        )
                    })
                    .collect()
            };
            for (name, sha256, attestation) in pieces {
                let Some(bundle) = attestation else {
                    if f.maker_signature.is_none() {
                        return Err(Fail::NotVerified(format!("{name} has no maker signature")));
                    }
                    out.push(format!(
                        "{name}: signed with the maker's SSH key (check it with --file)"
                    ));
                    continue;
                };
                let p = verifier
                    .verify_sha256(bundle, sha256, expected)
                    .map_err(|e| Fail::NotVerified(format!("{name}: {e}")))?;
                out.push(format!(
                    "{name}: built from {} at {} by {}{}",
                    p.source_repo,
                    p.commit,
                    p.signer_workflow,
                    if p.built_by_slop_build {
                        " (Slop Build)"
                    } else {
                        ""
                    }
                ));
            }
        }
    }
    Ok(out)
}

fn revocations(signed: &Signed, args: &Args) -> Result<Vec<String>, Fail> {
    let list: RevocationList = check(signed, Purpose::RevocationList, args)?;
    if list.is_expired(OffsetDateTime::now_utc()) {
        return Err(Fail::NotVerified(format!(
            "the list expired at {}: fetch a fresh one",
            list.expires_at
        )));
    }
    let mut out = vec![
        format!("Signed by Slop (key {})", signed.key_id),
        format!(
            "Revocation list issued {}, valid until {}: {} revoked",
            list.issued_at,
            list.expires_at,
            list.revoked.len()
        ),
    ];
    for r in &list.revoked {
        out.push(format!(
            "  {} release {} (revoked {}): {}",
            r.app_slug, r.release_id, r.revoked_at, r.reason
        ));
    }
    Ok(out)
}

fn file(signed: &Signed, args: &Args) -> Result<Vec<String>, Fail> {
    let answer: FileVerification = check(signed, Purpose::FileVerification, args)?;
    if answer.is_expired(OffsetDateTime::now_utc()) {
        return Err(Fail::NotVerified(format!(
            "the answer expired at {}: ask again",
            answer.expires_at
        )));
    }
    let mut out = vec![format!("Signed by Slop (key {})", signed.key_id)];
    match args.files.as_slice() {
        [] => {}
        [path] => {
            let (sha256, _) = hash_file(path)?;
            if sha256 != answer.sha256 {
                return Err(Fail::NotVerified(format!(
                    "{} isn't the file this answer is about (sha256 {sha256}, not {})",
                    path.display(),
                    answer.sha256
                )));
            }
            out.push(format!("{} is the file asked about", path.display()));
        }
        _ => return Err(Fail::Usage("file takes one --file".into())),
    }
    if answer.matches.is_empty() {
        out.push(format!(
            "Slop doesn't know sha256 {} (as of {})",
            answer.sha256, answer.issued_at
        ));
    }
    for m in &answer.matches {
        out.push(format!(
            "{}: {} {} by {} ({}), {}; scan: {}{}; {}",
            match m.status {
                MatchStatus::Approved => "APPROVED",
                MatchStatus::Revoked => "REVOKED",
            },
            m.app_title,
            m.version,
            m.maker,
            m.platform.as_str(),
            m.revoked_at
                .as_deref()
                .map_or(format!("approved {}", m.approved_at), |at| format!(
                    "revoked {at}"
                )),
            m.scan,
            m.defender
                .as_deref()
                .map_or(String::new(), |d| format!(", Microsoft Defender: {d}")),
            m.page_url
        ));
    }
    Ok(out)
}
