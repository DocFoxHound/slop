// SPDX-License-Identifier: MIT OR Apache-2.0
//! A maker's own signature on a release file (SECURITY §5.3, "files attached by hand"): an
//! OpenSSH signature (`ssh-keygen -Y sign -n slop-release`) made with an Ed25519 key the maker
//! registered on SLAPorSLOP. The format is OpenSSH's PROTOCOL.sshsig; only `ssh-ed25519` keys and the
//! `sha512` and `sha256` message hashes are accepted, so no RSA code is involved.
//!
//! SLAPorSLOP and the desktop app both check it against the file's digest, so the file itself never
//! needs to be in memory.

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

/// The namespace makers sign with: a signature for anything else (an SSH login, a git commit)
/// can't be passed off as a release signature.
pub const NAMESPACE: &str = "slop-release";

#[derive(Debug, thiserror::Error)]
pub enum SshSigError {
    #[error("not an ssh-ed25519 public key")]
    BadKey,
    #[error("not an SSH signature (expected -----BEGIN SSH SIGNATURE-----)")]
    BadArmor,
    #[error("the SSH signature is damaged")]
    Malformed,
    #[error("the signature is for {0:?}, not {NAMESPACE:?}: sign with -n {NAMESPACE}")]
    Namespace(String),
    #[error("the signature was made with a different key")]
    OtherKey,
    #[error("the signature uses {0}; use an Ed25519 key with sha512")]
    Unsupported(String),
    #[error("the signature doesn't match the file")]
    Invalid,
}

/// SSH wire-format reader: u32 big-endian length, then that many bytes.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], SshSigError> {
        if self.0.len() < n {
            return Err(SshSigError::Malformed);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u32(&mut self) -> Result<u32, SshSigError> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn string(&mut self) -> Result<&'a [u8], SshSigError> {
        let n = self.u32()? as usize;
        self.bytes(n)
    }
}

fn put_string(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}

/// The 32-byte key from an `ssh-ed25519 AAAA… comment` line (or its wire blob).
fn key_from_blob(blob: &[u8]) -> Result<[u8; 32], SshSigError> {
    let mut r = Reader(blob);
    if r.string().map_err(|_| SshSigError::BadKey)? != b"ssh-ed25519" {
        return Err(SshSigError::BadKey);
    }
    let key = r.string().map_err(|_| SshSigError::BadKey)?;
    key.try_into().map_err(|_| SshSigError::BadKey)
}

/// Parses an OpenSSH public key line. Returns the key and the line normalised to
/// `ssh-ed25519 <base64>` (comment dropped).
pub fn parse_public_key(line: &str) -> Result<([u8; 32], String), SshSigError> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("ssh-ed25519") {
        return Err(SshSigError::BadKey);
    }
    let b64 = parts.next().ok_or(SshSigError::BadKey)?;
    let blob = B64.decode(b64).map_err(|_| SshSigError::BadKey)?;
    let key = key_from_blob(&blob)?;
    VerifyingKey::from_bytes(&key).map_err(|_| SshSigError::BadKey)?;
    Ok((key, format!("ssh-ed25519 {b64}")))
}

/// The key's fingerprint as `ssh-keygen -l` prints it: `SHA256:<base64, no padding>`.
pub fn fingerprint(key: &[u8; 32]) -> String {
    let mut blob = Vec::new();
    put_string(&mut blob, b"ssh-ed25519");
    put_string(&mut blob, key);
    let digest = Sha256::digest(&blob);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    )
}

/// Checks an armored SSH signature over a file, given the file's SHA-512 and SHA-256 digests.
pub fn verify(
    key: &[u8; 32],
    armored: &str,
    sha512: &[u8; 64],
    sha256: &[u8; 32],
) -> Result<(), SshSigError> {
    let body: String = armored
        .trim()
        .strip_prefix("-----BEGIN SSH SIGNATURE-----")
        .and_then(|s| s.trim_end().strip_suffix("-----END SSH SIGNATURE-----"))
        .ok_or(SshSigError::BadArmor)?
        .split_whitespace()
        .collect();
    let blob = B64.decode(body).map_err(|_| SshSigError::Malformed)?;
    let mut r = Reader(&blob);
    if r.bytes(6)? != b"SSHSIG" || r.u32()? != 1 {
        return Err(SshSigError::Malformed);
    }
    let signer = key_from_blob(r.string()?)
        .map_err(|_| SshSigError::Unsupported("a key other than Ed25519".into()))?;
    let namespace = r.string()?;
    let _reserved = r.string()?;
    let hash_alg = r.string()?;
    let sig_blob = r.string()?;
    if namespace != NAMESPACE.as_bytes() {
        return Err(SshSigError::Namespace(
            String::from_utf8_lossy(namespace).into(),
        ));
    }
    if &signer != key {
        return Err(SshSigError::OtherKey);
    }
    let digest: &[u8] = match hash_alg {
        b"sha512" => sha512,
        b"sha256" => sha256,
        other => {
            return Err(SshSigError::Unsupported(
                String::from_utf8_lossy(other).into(),
            ));
        }
    };
    let mut s = Reader(sig_blob);
    if s.string()? != b"ssh-ed25519" {
        return Err(SshSigError::Unsupported("a non-Ed25519 signature".into()));
    }
    let sig = Signature::from_slice(s.string()?).map_err(|_| SshSigError::Malformed)?;
    // What was signed (PROTOCOL.sshsig): the magic, namespace, reserved, hash algorithm, H(file).
    let mut signed = b"SSHSIG".to_vec();
    put_string(&mut signed, namespace);
    put_string(&mut signed, b"");
    put_string(&mut signed, hash_alg);
    put_string(&mut signed, digest);
    VerifyingKey::from_bytes(key)
        .map_err(|_| SshSigError::BadKey)?
        .verify(&signed, &sig)
        .map_err(|_| SshSigError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::Sha512;

    /// Makes a signature the way `ssh-keygen -Y sign` does.
    fn sign(key: &SigningKey, namespace: &str, file: &[u8]) -> String {
        let public = key.verifying_key().to_bytes();
        let mut pub_blob = Vec::new();
        put_string(&mut pub_blob, b"ssh-ed25519");
        put_string(&mut pub_blob, &public);
        let mut signed = b"SSHSIG".to_vec();
        put_string(&mut signed, namespace.as_bytes());
        put_string(&mut signed, b"");
        put_string(&mut signed, b"sha512");
        put_string(&mut signed, &Sha512::digest(file));
        let sig = key.sign(&signed);
        let mut sig_blob = Vec::new();
        put_string(&mut sig_blob, b"ssh-ed25519");
        put_string(&mut sig_blob, &sig.to_bytes());
        let mut blob = b"SSHSIG".to_vec();
        blob.extend_from_slice(&1u32.to_be_bytes());
        put_string(&mut blob, &pub_blob);
        put_string(&mut blob, namespace.as_bytes());
        put_string(&mut blob, b"");
        put_string(&mut blob, b"sha512");
        put_string(&mut blob, &sig_blob);
        format!(
            "-----BEGIN SSH SIGNATURE-----\n{}\n-----END SSH SIGNATURE-----\n",
            B64.encode(blob)
        )
    }

    fn digests(file: &[u8]) -> ([u8; 64], [u8; 32]) {
        (Sha512::digest(file).into(), Sha256::digest(file).into())
    }

    #[test]
    fn signatures_check_out_only_for_their_key_file_and_namespace() {
        let key = SigningKey::from_bytes(&[3; 32]);
        let public = key.verifying_key().to_bytes();
        let file = b"the game";
        let (s512, s256) = digests(file);
        let good = sign(&key, NAMESPACE, file);
        verify(&public, &good, &s512, &s256).unwrap();
        let (o512, o256) = digests(b"another game");
        assert!(matches!(
            verify(&public, &good, &o512, &o256),
            Err(SshSigError::Invalid)
        ));
        let other = SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes();
        assert!(matches!(
            verify(&other, &good, &s512, &s256),
            Err(SshSigError::OtherKey)
        ));
        let login = sign(&key, "file", file);
        assert!(matches!(
            verify(&public, &login, &s512, &s256),
            Err(SshSigError::Namespace(_))
        ));
        assert!(matches!(
            verify(&public, "nope", &s512, &s256),
            Err(SshSigError::BadArmor)
        ));
    }

    /// Against OpenSSH itself, where it's installed: a key and a signature made by ssh-keygen.
    #[test]
    fn real_ssh_keygen_signatures_verify() {
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!("slop-sshsig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("id");
        let made = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "maker@test", "-f"])
            .arg(&key)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("ssh-keygen isn't installed; skipping");
            return;
        }
        let file = dir.join("game.zip");
        std::fs::write(&file, b"a real build").unwrap();
        let signed = Command::new("ssh-keygen")
            .args(["-Y", "sign", "-n", NAMESPACE, "-f"])
            .arg(&key)
            .arg(&file)
            .status()
            .unwrap();
        assert!(signed.success());
        let line = std::fs::read_to_string(dir.join("id.pub")).unwrap();
        let (public, _) = parse_public_key(&line).unwrap();
        let sig = std::fs::read_to_string(dir.join("game.zip.sig")).unwrap();
        let (s512, s256) = digests(b"a real build");
        verify(&public, &sig, &s512, &s256).unwrap();
        let fp = Command::new("ssh-keygen")
            .arg("-lf")
            .arg(dir.join("id.pub"))
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&fp.stdout).contains(&fingerprint(&public)));
        let (o512, o256) = digests(b"tampered");
        assert!(verify(&public, &sig, &o512, &o256).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn public_keys_parse_and_fingerprint_like_ssh_keygen() {
        let key = SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes();
        let mut blob = Vec::new();
        put_string(&mut blob, b"ssh-ed25519");
        put_string(&mut blob, &key);
        let line = format!("ssh-ed25519 {} me@laptop", B64.encode(&blob));
        let (parsed, normal) = parse_public_key(&line).unwrap();
        assert_eq!(parsed, key);
        assert!(!normal.contains("me@laptop"));
        assert!(fingerprint(&key).starts_with("SHA256:"));
        assert!(parse_public_key("ssh-rsa AAAAB3Nza").is_err());
    }
}
