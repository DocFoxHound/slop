//! Signed documents shared by the Slop API and the Slop client (docs/SECURITY.md §5.3, §5.5).
//!
//! - A **release manifest** lists every file of a release with its URL on the maker's release
//!   page, SHA-256 and size, and how to launch it. The client installs only files whose hash
//!   matches a manifest Slop signed.
//! - The **revocation list** names releases that must no longer launch (the kill switch).
//!
//! A signed document carries the exact JSON text that was signed, so both sides verify the same
//! bytes. Signatures are Ed25519 over a purpose prefix plus that text, so a document signed for
//! one purpose can't be passed off as another.
//!
//! Key management is deliberately simple for now: the client trusts a pinned list of public
//! keys. The TUF-style root / targets / timestamp roles in SECURITY §5.3 replace that later.

pub mod runtimes;
pub mod sshsig;
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub use ed25519_dalek;

pub const MANIFEST_SCHEMA: u32 = 1;
pub const REVOCATIONS_SCHEMA: u32 = 1;

// -------------------------------------------------------------------------------------------
// Documents

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Mac,
    /// Linux: a `.tar.gz` (or `.zip`) of the app's files, which the Slop app installs and runs.
    Linux,
    Android,
    /// A Debian/Ubuntu package of the same Linux build, for players who prefer one.
    #[serde(rename = "linux-deb")]
    LinuxDeb,
    /// A Fedora/RHEL package of the same Linux build.
    #[serde(rename = "linux-rpm")]
    LinuxRpm,
    /// A mod: one `.zip` for every platform, with nothing to start. The Slop app installs it for
    /// the game it belongs to.
    Any,
}

impl Platform {
    pub const ALL: [Platform; 7] = [
        Self::Windows,
        Self::Mac,
        Self::Linux,
        Self::Android,
        Self::LinuxDeb,
        Self::LinuxRpm,
        Self::Any,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Mac => "mac",
            Self::Linux => "linux",
            Self::Android => "android",
            Self::LinuxDeb => "linux-deb",
            Self::LinuxRpm => "linux-rpm",
            Self::Any => "any",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == value)
    }

    /// The platform this code was compiled for, if Slop supports it.
    pub fn current() -> Option<Self> {
        if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Self::Mac)
        } else if cfg!(target_os = "android") {
            Some(Self::Android)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// What a release file is (ARCHITECTURE §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// A zip of the app's files, installed into Slop's library folder. Preferred.
    Archive,
    /// The maker's own installer, run only after the user confirms.
    Installer,
    /// An Android package.
    Apk,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Installer => "installer",
            Self::Apk => "apk",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [Self::Archive, Self::Installer, Self::Apk]
            .into_iter()
            .find(|k| k.as_str() == value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    /// Path of the executable inside the archive, with `/` separators.
    pub exec: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub platform: Platform,
    pub kind: FileKind,
    /// Where the file lives: the maker's release page. Slop stores no builds.
    pub url: String,
    pub file_name: String,
    pub size: u64,
    /// Lowercase hex SHA-256 of the whole file.
    pub sha256: String,
    /// How to start the app. Present for archives.
    pub launch: Option<Launch>,
    /// The maker's signature: a GitHub artifact attestation (Sigstore bundle JSON) naming this
    /// file's SHA-256, the app's repository and the release's commit. Clients verify it
    /// themselves before installing (SECURITY §5.3).
    #[serde(default)]
    pub attestation: Option<String>,
    /// For files the maker built and signed themselves: their SSH signature and the registered
    /// key that made it (sshsig.rs). Present only when there's no attestation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maker_signature: Option<MakerSignature>,
    /// Files over GitHub's 2 GiB limit are released in parts, joined in order. Then `url` names
    /// the joined file (not uploaded itself), `size` and `sha256` are the joined file's, and each
    /// part has its own hash and attestation. Empty for a file in one piece.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<FilePart>,
}

/// One part of a file released in parts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePart {
    pub url: String,
    pub size: u64,
    /// Lowercase hex SHA-256 of this part.
    pub sha256: String,
    /// The maker's attestation naming this part's hash, like a whole file's.
    #[serde(default)]
    pub attestation: Option<String>,
}

/// A maker's own signature on a file (SECURITY §5.3, files attached by hand).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MakerSignature {
    /// `ssh-ed25519 <base64>`: the key registered on the maker's Slop account.
    pub public_key: String,
    /// The armored `ssh-keygen -Y sign -n slop-release` signature.
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema: u32,
    pub app_id: String,
    pub app_slug: String,
    pub app_title: String,
    /// Every app clearly names its maker (LEGAL-GUIDANCE rule 4).
    pub maker: String,
    pub repo_url: String,
    pub release_id: String,
    pub version: String,
    /// The commit the release was built from. Tags can move; commits can't.
    pub commit: String,
    pub files: Vec<ManifestFile>,
    /// RFC 3339.
    pub approved_at: String,
    /// A mod: the game it's for. Signed with the rest, so a mod can only be added to that game.
    /// Absent on everything else, so older apps still read those manifests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mod_for: Option<ModFor>,
    /// A community build: the original app it's a build of, so the Slop app can offer it on the
    /// original's Play button. Absent on everything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_of: Option<ModFor>,
    /// The folder the app reads mods from, when its maker named one: the Slop app keeps only
    /// approved mods there (DESIGN §9). Absent: the app reads `SLOP_MODS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mods_folder: Option<ModsFolder>,
    /// Shared runtimes the app needs (runtimes.rs IDs); the Slop app installs missing ones
    /// before the first start, with the player's say-so.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<String>,
}

/// Where an app reads mods from, and how they're laid out there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModsFolder {
    pub base: ModsBase,
    /// Relative to the base, with `/` between parts: `mods`, `Content/Paks/~mods`.
    pub path: String,
    pub layout: ModsLayout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModsBase {
    /// The app's installed files.
    Game,
    /// Its save folder (`SLOP_SAVE_DIR`).
    Saves,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModsLayout {
    /// Each mod in a folder of its own, named after its Slop address.
    Folder,
    /// Every mod's files straight in the folder (for games that read `.pak` or `.zip` files
    /// there).
    Flat,
}

impl ModsFolder {
    /// A plain relative path: 1–200 characters, no `..`, no leading `/`, no backslashes.
    pub fn is_valid(&self) -> bool {
        !self.path.is_empty()
            && self.path.len() <= 200
            && !self.path.starts_with('/')
            && !self.path.contains('\\')
            && !self.path.contains(':')
            && self
                .path
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != ".." && !p.chars().any(char::is_control))
    }
}

/// The original app a mod or community build is for (DESIGN §9).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModFor {
    pub app_id: String,
    pub app_slug: String,
    pub app_title: String,
}

impl ReleaseManifest {
    pub fn file_for(&self, platform: Platform) -> Option<&ManifestFile> {
        self.files.iter().find(|f| f.platform == platform)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revoked {
    pub release_id: String,
    pub app_slug: String,
    pub reason: String,
    pub revoked_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevocationList {
    pub schema: u32,
    /// RFC 3339. A list past its expiry is stale (stops replay of an old list).
    pub issued_at: String,
    pub expires_at: String,
    pub revoked: Vec<Revoked>,
}

impl RevocationList {
    pub fn is_revoked(&self, release_id: &str) -> Option<&Revoked> {
        self.revoked.iter().find(|r| r.release_id == release_id)
    }

    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        OffsetDateTime::parse(&self.expires_at, &Rfc3339).map_or(true, |exp| exp <= now)
    }
}

// -------------------------------------------------------------------------------------------
// Signing

/// What a signature is for. The purpose is part of the signed bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    ReleaseManifest,
    RevocationList,
}

impl Purpose {
    fn prefix(self) -> &'static [u8] {
        match self {
            Self::ReleaseManifest => b"slop/release-manifest/v1\n",
            Self::RevocationList => b"slop/revocation-list/v1\n",
        }
    }
}

/// A document and Slop's signature over it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed {
    /// The exact JSON text that was signed.
    pub payload: String,
    /// Base64 Ed25519 signature.
    pub signature: String,
    pub key_id: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("signed by an unknown key")]
    UnknownKey,
    #[error("the signature doesn't match")]
    BadSignature,
    #[error("the document is malformed: {0}")]
    Malformed(String),
}

/// A short ID for a public key: the first 16 hex characters of its SHA-256.
pub fn key_id(key: &VerifyingKey) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))[..16].to_string()
}

fn message(purpose: Purpose, payload: &str) -> Vec<u8> {
    [purpose.prefix(), payload.as_bytes()].concat()
}

/// Serialises `doc` and signs it.
pub fn sign<T: Serialize>(doc: &T, purpose: Purpose, key: &SigningKey) -> Signed {
    let payload = serde_json::to_string(doc).expect("documents always serialise");
    let signature = key.sign(&message(purpose, &payload));
    Signed {
        signature: STANDARD.encode(signature.to_bytes()),
        key_id: key_id(&key.verifying_key()),
        payload,
    }
}

/// Checks the signature against the trusted keys, then parses the payload.
pub fn verify<T: DeserializeOwned>(
    signed: &Signed,
    purpose: Purpose,
    trusted: &[VerifyingKey],
) -> Result<T, VerifyError> {
    let key = trusted
        .iter()
        .find(|k| key_id(k) == signed.key_id)
        .ok_or(VerifyError::UnknownKey)?;
    let bytes: [u8; 64] = STANDARD
        .decode(&signed.signature)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or(VerifyError::BadSignature)?;
    key.verify_strict(
        &message(purpose, &signed.payload),
        &Signature::from_bytes(&bytes),
    )
    .map_err(|_| VerifyError::BadSignature)?;
    serde_json::from_str(&signed.payload).map_err(|e| VerifyError::Malformed(e.to_string()))
}

/// Reads a base64 32-byte Ed25519 secret key.
pub fn signing_key_from_base64(value: &str) -> Option<SigningKey> {
    let bytes: [u8; 32] = STANDARD.decode(value.trim()).ok()?.try_into().ok()?;
    Some(SigningKey::from_bytes(&bytes))
}

/// Reads a base64 32-byte Ed25519 public key.
pub fn verifying_key_from_base64(value: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = STANDARD.decode(value.trim()).ok()?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

pub fn verifying_key_to_base64(key: &VerifyingKey) -> String {
    STANDARD.encode(key.as_bytes())
}

pub fn signing_key_to_base64(key: &SigningKey) -> String {
    STANDARD.encode(key.to_bytes())
}

/// Checks that a launch path is a plain relative path inside the install folder.
pub fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 260
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&rand::random::<[u8; 32]>())
    }

    fn manifest() -> ReleaseManifest {
        ReleaseManifest {
            schema: MANIFEST_SCHEMA,
            app_id: "a".into(),
            app_slug: "pixel-dungeon".into(),
            app_title: "Pixel Dungeon".into(),
            maker: "Octo".into(),
            repo_url: "https://github.com/octo/pixel-dungeon".into(),
            release_id: "r1".into(),
            version: "1.0.0".into(),
            commit: "0".repeat(40),
            files: vec![ManifestFile {
                platform: Platform::Linux,
                kind: FileKind::Archive,
                url: "https://github.com/octo/pixel-dungeon/releases/download/v1/linux.zip".into(),
                file_name: "linux.zip".into(),
                size: 10,
                sha256: "ab".repeat(32),
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
            build_of: None,
            mods_folder: None,
            runtimes: vec![],
        }
    }

    #[test]
    fn mod_for_is_left_out_unless_set() {
        let plain = serde_json::to_string(&manifest()).unwrap();
        assert!(!plain.contains("mod_for"));
        assert!(!plain.contains("build_of"));
        let mut m = manifest();
        m.mod_for = Some(ModFor {
            app_id: "g".into(),
            app_slug: "the-game".into(),
            app_title: "The Game".into(),
        });
        let back: ReleaseManifest =
            serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn mods_folders_are_plain_relative_paths() {
        let f = |path: &str| ModsFolder {
            base: ModsBase::Game,
            path: path.into(),
            layout: ModsLayout::Folder,
        };
        assert!(f("mods").is_valid());
        assert!(f("Content/Paks/~mods").is_valid());
        for bad in [
            "", "/mods", "../mods", "a/../b", "a//b", "C:/mods", "a\\b", "./mods",
        ] {
            assert!(!f(bad).is_valid(), "{bad}");
        }
        let json = serde_json::to_value(f("mods")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"base": "game", "path": "mods", "layout": "folder"})
        );
    }

    #[test]
    fn round_trip() {
        let k = key();
        let signed = sign(&manifest(), Purpose::ReleaseManifest, &k);
        let back: ReleaseManifest =
            verify(&signed, Purpose::ReleaseManifest, &[k.verifying_key()]).unwrap();
        assert_eq!(back, manifest());
    }

    #[test]
    fn tampering_is_detected() {
        let k = key();
        let mut signed = sign(&manifest(), Purpose::ReleaseManifest, &k);
        signed.payload = signed.payload.replace("ababab", "cdcdcd");
        let err =
            verify::<ReleaseManifest>(&signed, Purpose::ReleaseManifest, &[k.verifying_key()]);
        assert_eq!(err, Err(VerifyError::BadSignature));
    }

    #[test]
    fn unknown_keys_and_wrong_purposes_fail() {
        let k = key();
        let signed = sign(&manifest(), Purpose::ReleaseManifest, &k);
        assert_eq!(
            verify::<ReleaseManifest>(&signed, Purpose::ReleaseManifest, &[key().verifying_key()]),
            Err(VerifyError::UnknownKey)
        );
        assert_eq!(
            verify::<ReleaseManifest>(&signed, Purpose::RevocationList, &[k.verifying_key()]),
            Err(VerifyError::BadSignature)
        );
    }

    #[test]
    fn key_encoding_round_trips() {
        let k = key();
        let again = signing_key_from_base64(&signing_key_to_base64(&k)).unwrap();
        assert_eq!(again.to_bytes(), k.to_bytes());
        let public =
            verifying_key_from_base64(&verifying_key_to_base64(&k.verifying_key())).unwrap();
        assert_eq!(key_id(&public), key_id(&k.verifying_key()));
        assert!(signing_key_from_base64("not base64").is_none());
    }

    #[test]
    fn revocation_expiry() {
        let list = RevocationList {
            schema: REVOCATIONS_SCHEMA,
            issued_at: "2026-10-18T00:00:00Z".into(),
            expires_at: "2026-10-18T06:00:00Z".into(),
            revoked: vec![],
        };
        let at = |s: &str| OffsetDateTime::parse(s, &Rfc3339).unwrap();
        assert!(!list.is_expired(at("2026-10-18T05:59:59Z")));
        assert!(list.is_expired(at("2026-10-18T06:00:00Z")));
    }

    #[test]
    fn relative_paths() {
        for ok in [
            "game",
            "bin/game",
            "My Game.exe",
            "Game.app/Contents/MacOS/Game",
        ] {
            assert!(is_safe_relative_path(ok), "{ok}");
        }
        for bad in [
            "", "/bin/sh", "../x", "a/../b", "a//b", "C:\\x", "C:x", "a\\b", "./a", "a/",
        ] {
            assert!(!is_safe_relative_path(bad), "{bad}");
        }
    }
}
