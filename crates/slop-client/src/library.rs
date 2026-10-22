// SPDX-License-Identifier: AGPL-3.0-or-later
//! Installing, verifying and launching apps (ARCHITECTURE §6.3, SECURITY §5.3, §5.5, §7).
//!
//! Layout under the library root (in the user's profile, so no admin rights are needed):
//!
//! ```text
//! apps/<slug>/install.json    the signed manifest and what was installed
//! apps/<slug>/files/…         an archive's contents
//! apps/<slug>/installer/…     a maker's installer, kept for the user to run
//! mods/<slug>/install.json    a mod: its signed manifest, which names the game it's for
//! mods/<slug>/files/…         the mod's contents, handed to its game at launch (SLOP_MODS)
//! saves/<slug>/               the app's save folder (SLOP_SAVE_DIR), kept when it's uninstalled
//! save-copies/<slug>/<time>/  copies of it taken before sandboxed sessions
//! quarantine/<slug>/…         programs found in the save folder and moved out of it
//! cache/<slug>/               the app's cache, never copied
//! cache-sandbox/<slug>/       its cache in sandbox mode, emptied before each sandboxed session
//! settings/sandbox.json       which apps run in sandbox mode
//! staging/                    downloads and unpacking in progress
//! revocations.json            the last signed revocation list
//! ```
//!
//! Nothing is written into `apps/` until the download matches the size and SHA-256 in a manifest
//! signed by a trusted Slop key. Every launch re-checks the manifest signature, the revocation
//! list and the hash of the program being started, so tampering with the library folder can't
//! slip anything past verification.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use slop_manifest::{
    FileKind, Platform, Purpose, ReleaseManifest, RevocationList, Signed,
    ed25519_dalek::VerifyingKey, is_safe_relative_path,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    Error, Result,
    api::ApiClient,
    sandbox::{AppSandbox, SandboxSettings},
};

/// What `install.json` holds.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallRecord {
    pub manifest: Signed,
    pub platform: Platform,
    pub installed_at: String,
    /// SHA-256 of the program to launch, taken when it was unpacked.
    pub exec_sha256: Option<String>,
    /// Who built it, from the maker's attestation, verified at install time.
    #[serde(default)]
    pub provenance: Option<slop_provenance::Provenance>,
}

/// An installed app, as the UI sees it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct InstalledApp {
    pub slug: String,
    pub title: String,
    pub maker: String,
    pub version: String,
    pub release_id: String,
    pub kind: FileKind,
    pub installed_at: String,
    /// Verified maker attestation, if the build had one.
    pub provenance: Option<slop_provenance::Provenance>,
    /// A community build: the original app's address (signed in its manifest), so it can be
    /// picked on the original's Play button.
    pub build_of: Option<String>,
}

/// An installed mod (DESIGN §9), as the UI sees it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct InstalledMod {
    pub slug: String,
    pub title: String,
    pub maker: String,
    pub version: String,
    pub release_id: String,
    /// The game it's for.
    pub game_slug: String,
    pub game_title: String,
    pub installed_at: String,
    /// Why it was pulled, if the saved revocation list names it. A pulled mod isn't handed to
    /// its game.
    pub revoked: Option<String>,
}

/// A checked app, ready to start.
pub struct Launch {
    pub command: Command,
    pub sandboxed: bool,
    /// Programs and libraries found in the save folder and moved out (relative paths).
    pub quarantined: Vec<String>,
    /// The saves were too big to copy before this sandboxed session.
    pub copy_skipped: bool,
    /// Things in the game's mods folder that aren't mods from Slop, moved out (DESIGN §9).
    pub unapproved_mods: Vec<String>,
}

pub struct Library {
    root: PathBuf,
    trusted: Vec<VerifyingKey>,
    http: reqwest::Client,
    allow_http: bool,
    provenance: slop_provenance::ProvenanceVerifier,
    require_maker_signature: bool,
}

fn valid_slug(slug: &str) -> bool {
    (3..=60).contains(&slug.len())
        && slug.split('-').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

impl Library {
    pub fn new(root: impl Into<PathBuf>, trusted: Vec<VerifyingKey>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent(concat!("slop-desktop/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let root = root.into();
        fs::create_dir_all(root.join("apps"))?;
        for dir in [
            "mods",
            "saves",
            "save-copies",
            "quarantine",
            "cache",
            "settings",
        ] {
            fs::create_dir_all(root.join(dir))?;
        }
        fs::create_dir_all(root.join("staging"))?;
        let provenance = slop_provenance::ProvenanceVerifier::public_good()
            .map_err(|e| Error::Verify(e.to_string()))?;
        Ok(Self {
            root,
            trusted,
            http,
            allow_http: false,
            provenance,
            require_maker_signature: true,
        })
    }

    /// Whether builds must carry a verified maker signature (SECURITY §5.3). On by default; only
    /// development builds against a local API turn it off.
    pub fn require_maker_signature(mut self, require: bool) -> Self {
        self.require_maker_signature = require;
        self
    }

    /// Allows plain-http downloads. Only for local development against a dev API, and tests.
    pub fn allow_http_downloads(mut self, allow: bool) -> Self {
        self.allow_http = allow;
        self
    }

    fn app_dir(&self, slug: &str) -> Result<PathBuf> {
        if !valid_slug(slug) {
            return Err(Error::Install(format!("not an app: {slug:?}")));
        }
        Ok(self.root.join("apps").join(slug))
    }

    fn mod_dir(&self, slug: &str) -> Result<PathBuf> {
        if !valid_slug(slug) {
            return Err(Error::Install(format!("not a mod: {slug:?}")));
        }
        Ok(self.root.join("mods").join(slug))
    }

    pub fn verify_manifest(&self, signed: &Signed) -> Result<ReleaseManifest> {
        let manifest: ReleaseManifest =
            slop_manifest::verify(signed, Purpose::ReleaseManifest, &self.trusted)
                .map_err(|e| Error::Verify(format!("release manifest: {e}")))?;
        if !valid_slug(&manifest.app_slug) {
            return Err(Error::Verify("release manifest names a bad app".into()));
        }
        Ok(manifest)
    }

    /// Reads and re-verifies an app's install record.
    pub fn record(&self, slug: &str) -> Result<(InstallRecord, ReleaseManifest)> {
        let path = self.app_dir(slug)?.join("install.json");
        let bytes =
            fs::read(&path).map_err(|_| Error::Install(format!("{slug} isn't installed")))?;
        let record: InstallRecord = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Verify("install record is damaged".into()))?;
        let manifest = self.verify_manifest(&record.manifest)?;
        if manifest.app_slug != slug || manifest.mod_for.is_some() {
            return Err(Error::Verify(
                "install record belongs to another app".into(),
            ));
        }
        Ok((record, manifest))
    }

    /// Reads and re-verifies a mod's install record. Returns the game it's for too.
    pub fn mod_record(
        &self,
        slug: &str,
    ) -> Result<(InstallRecord, ReleaseManifest, slop_manifest::ModFor)> {
        let path = self.mod_dir(slug)?.join("install.json");
        let bytes = fs::read(&path).map_err(|_| Error::Install(format!("{slug} isn't added")))?;
        let record: InstallRecord = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Verify("install record is damaged".into()))?;
        let manifest = self.verify_manifest(&record.manifest)?;
        let game = manifest.mod_for.clone();
        match game {
            Some(game) if manifest.app_slug == slug && record.platform == Platform::Any => {
                Ok((record, manifest, game))
            }
            _ => Err(Error::Verify(
                "install record belongs to something else".into(),
            )),
        }
    }

    /// The installed game with this ID (its address can change), if any.
    fn installed_game(&self, app_id: &str) -> Option<(String, ReleaseManifest)> {
        fs::read_dir(self.root.join("apps"))
            .ok()?
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .find_map(|slug| {
                let (_, manifest) = self.record(&slug).ok()?;
                (manifest.app_id == app_id).then_some((slug, manifest))
            })
    }

    /// Every mod added, verified, with the game each is for: (mod, game's app ID).
    fn all_mods(&self) -> Vec<(InstalledMod, String)> {
        let Ok(entries) = fs::read_dir(self.root.join("mods")) else {
            return vec![];
        };
        let revocations = self.cached_revocations();
        let mut mods: Vec<(InstalledMod, String)> = entries
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|slug| {
                let (record, manifest, game) = self.mod_record(&slug).ok()?;
                let revoked = revocations
                    .as_ref()
                    .and_then(|r| r.is_revoked(&manifest.release_id))
                    .map(|r| r.reason.clone());
                Some((
                    InstalledMod {
                        slug,
                        title: manifest.app_title,
                        maker: manifest.maker,
                        version: manifest.version,
                        release_id: manifest.release_id,
                        game_slug: game.app_slug,
                        game_title: game.app_title,
                        installed_at: record.installed_at,
                        revoked,
                    },
                    game.app_id,
                ))
            })
            .collect();
        mods.sort_by_key(|(m, _)| m.title.to_lowercase());
        mods
    }

    /// The mods added to an installed game.
    pub fn installed_mods(&self, game_slug: &str) -> Vec<InstalledMod> {
        let Ok((_, game)) = self.record(game_slug) else {
            return vec![];
        };
        self.all_mods()
            .into_iter()
            .filter(|(_, id)| *id == game.app_id)
            .map(|(mut m, _)| {
                m.game_slug = game_slug.to_string();
                m
            })
            .collect()
    }

    /// Everything installed and still verifiable.
    pub fn installed(&self) -> Vec<InstalledApp> {
        let Ok(entries) = fs::read_dir(self.root.join("apps")) else {
            return vec![];
        };
        let mut apps: Vec<InstalledApp> = entries
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|slug| {
                let (record, manifest) = self.record(&slug).ok()?;
                let kind = manifest.file_for(record.platform)?.kind;
                Some(InstalledApp {
                    slug,
                    title: manifest.app_title,
                    maker: manifest.maker,
                    version: manifest.version,
                    release_id: manifest.release_id,
                    kind,
                    installed_at: record.installed_at,
                    provenance: record.provenance,
                    build_of: manifest.build_of.map(|o| o.app_slug),
                })
            })
            .collect();
        apps.sort_by_key(|a| a.title.to_lowercase());
        apps
    }

    /// Downloads, verifies and installs the build for `platform` from a signed manifest.
    /// `progress` gets (bytes so far, total bytes).
    /// `links`: where to ask for a download link when the file's own URL isn't public (a private
    /// repository, ARCHITECTURE §6.5). The file is checked against the manifest either way.
    pub async fn install(
        &self,
        signed: &Signed,
        platform: Platform,
        links: Option<&ApiClient>,
        progress: impl Fn(u64, u64),
    ) -> Result<InstalledApp> {
        let manifest = self.verify_manifest(signed)?;
        let file = manifest
            .file_for(platform)
            .ok_or_else(|| {
                Error::Install(format!(
                    "{} has no build for this computer yet",
                    manifest.app_title
                ))
            })?
            .clone();
        if file.kind == FileKind::Apk {
            return Err(Error::Install(
                "Android builds install on Android phones".into(),
            ));
        }
        if manifest.mod_for.is_some() || platform == Platform::Any {
            return Err(Error::Install(
                "this is a mod: add it to its game from the game's page in your library".into(),
            ));
        }
        let previous = self.record(&manifest.app_slug).ok().map(|(_, m)| m);
        let provenance = self.check_maker_signature(previous.as_ref(), &manifest, &file)?;
        let final_dir = self.app_dir(&manifest.app_slug)?;
        self.stage_and_swap(
            &final_dir, signed, &manifest, &file, platform, provenance, links, &progress,
        )
        .await?;

        let (record, _) = self.record(&manifest.app_slug)?;
        Ok(InstalledApp {
            slug: manifest.app_slug,
            title: manifest.app_title,
            maker: manifest.maker,
            version: manifest.version,
            release_id: manifest.release_id,
            kind: file.kind,
            installed_at: record.installed_at,
            provenance: record.provenance,
            build_of: manifest.build_of.map(|o| o.app_slug),
        })
    }

    /// Downloads, verifies and adds a mod to its game, which must be installed. Mods are a
    /// `.zip` of files for the game to load; nothing in one is started by Slop.
    pub async fn install_mod(
        &self,
        signed: &Signed,
        links: Option<&ApiClient>,
        progress: impl Fn(u64, u64),
    ) -> Result<InstalledMod> {
        let manifest = self.verify_manifest(signed)?;
        let game = manifest
            .mod_for
            .clone()
            .ok_or_else(|| Error::Install(format!("{} isn't a mod", manifest.app_title)))?;
        let (game_slug, _) = self.installed_game(&game.app_id).ok_or_else(|| {
            Error::Install(format!(
                "install {} first, then add its mods",
                game.app_title
            ))
        })?;
        let file = manifest
            .file_for(Platform::Any)
            .filter(|f| f.kind == FileKind::Archive)
            .ok_or_else(|| Error::Install(format!("{} has no files yet", manifest.app_title)))?
            .clone();
        let previous = self.mod_record(&manifest.app_slug).ok().map(|(_, m, _)| m);
        let provenance = self.check_maker_signature(previous.as_ref(), &manifest, &file)?;
        let final_dir = self.mod_dir(&manifest.app_slug)?;
        self.stage_and_swap(
            &final_dir,
            signed,
            &manifest,
            &file,
            Platform::Any,
            provenance,
            links,
            &progress,
        )
        .await?;
        let mut added = self
            .installed_mods(&game_slug)
            .into_iter()
            .find(|m| m.slug == manifest.app_slug)
            .ok_or_else(|| Error::Install("the mod didn't install".into()))?;
        added.game_slug = game_slug;
        Ok(added)
    }

    pub fn uninstall_mod(&self, slug: &str) -> Result<()> {
        let dir = self.mod_dir(slug)?;
        if !dir.exists() {
            return Err(Error::Install(format!("{slug} isn't added")));
        }
        self.remove_dir(&dir)
    }

    /// Moves a folder aside, then deletes it, so a half-deleted one never looks installed.
    fn remove_dir(&self, dir: &Path) -> Result<()> {
        let trash = self
            .root
            .join("staging")
            .join(format!("old-{}", crate::random_token()));
        fs::rename(dir, &trash)?;
        fs::remove_dir_all(&trash)?;
        Ok(())
    }

    /// Downloads and unpacks into a fresh staging folder, then swaps it into `final_dir`; the
    /// old install is removed afterwards.
    #[allow(clippy::too_many_arguments)] // private helper; each argument is distinct
    async fn stage_and_swap(
        &self,
        final_dir: &Path,
        signed: &Signed,
        manifest: &ReleaseManifest,
        file: &slop_manifest::ManifestFile,
        platform: Platform,
        provenance: Option<slop_provenance::Provenance>,
        links: Option<&ApiClient>,
        progress: &impl Fn(u64, u64),
    ) -> Result<()> {
        let staging = self.root.join("staging").join(crate::random_token());
        fs::create_dir_all(&staging)?;
        let result = self
            .install_into(
                &staging, signed, manifest, file, platform, provenance, links, progress,
            )
            .await;
        if let Err(err) = result {
            let _ = fs::remove_dir_all(&staging);
            return Err(err);
        }
        let old = self
            .root
            .join("staging")
            .join(format!("old-{}", crate::random_token()));
        if final_dir.exists() {
            fs::rename(final_dir, &old)?;
        }
        if let Err(err) = fs::rename(staging.join("app"), final_dir) {
            if old.exists() {
                let _ = fs::rename(&old, final_dir);
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(err.into());
        }
        let _ = fs::remove_dir_all(&old);
        let _ = fs::remove_dir_all(&staging);
        Ok(())
    }

    /// Checks the maker's signature on a build, before anything is downloaded, and that an
    /// update is signed by the same maker as what's installed. `previous`: the installed release
    /// of the same app or mod, if any.
    fn check_maker_signature(
        &self,
        previous: Option<&ReleaseManifest>,
        manifest: &ReleaseManifest,
        file: &slop_manifest::ManifestFile,
    ) -> Result<Option<slop_provenance::Provenance>> {
        let provenance = self.verify_maker_signature(previous, manifest, file)?;
        // The same signer as the installed version (SECURITY §5.3): Slop's signature on the
        // manifest vouches for the maker's, so without this whoever held Slop's key could swap
        // in a key of their own, or a repository that took over the old one's name.
        if let Some(installed) = previous
            && let Some(before) = installed.files.iter().find(|f| f.platform == file.platform)
        {
            let was = self.signer_of(installed, before, None);
            let now = self.signer_of(manifest, file, provenance.as_ref());
            if let Some(problem) = Signer::changed(&was, &now) {
                return Err(Error::Verify(format!(
                    "{problem}; uninstall it first if you trust the change"
                )));
            }
        }
        Ok(provenance)
    }

    /// Who signed a build: the maker's SSH key, or the repository (by GitHub's IDs) whose CI
    /// attested it. `known`: the provenance already verified for this file, if any.
    fn signer_of(
        &self,
        manifest: &ReleaseManifest,
        file: &slop_manifest::ManifestFile,
        known: Option<&slop_provenance::Provenance>,
    ) -> Signer {
        let bundle = file
            .attestation
            .as_ref()
            .map(|b| (b, &file.sha256))
            .or_else(|| {
                file.parts
                    .first()
                    .and_then(|p| p.attestation.as_ref().map(|b| (b, &p.sha256)))
            });
        if let Some((bundle, sha256)) = bundle {
            let provenance = known.cloned().or_else(|| {
                let expected = slop_provenance::Expected {
                    repo_id: None,
                    repo_url: &manifest.repo_url,
                    commit: &manifest.commit,
                    slop_build_workflow: None,
                };
                self.provenance.verify_sha256(bundle, sha256, expected).ok()
            });
            return Signer::Repository(
                provenance.and_then(|p| p.source_repo_id.zip(p.source_owner_id)),
            );
        }
        match &file.maker_signature {
            Some(sig) => Signer::Key(
                sig.public_key
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            None => Signer::Unsigned,
        }
    }

    fn verify_maker_signature(
        &self,
        previous: Option<&ReleaseManifest>,
        manifest: &ReleaseManifest,
        file: &slop_manifest::ManifestFile,
    ) -> Result<Option<slop_provenance::Provenance>> {
        // An update must come from the same repository as what's installed: not even Slop can
        // quietly repoint an installed app at someone else's code.
        if let Some(installed) = previous {
            let norm = |s: &str| {
                s.trim_end_matches('/')
                    .trim_end_matches(".git")
                    .to_ascii_lowercase()
            };
            if norm(&installed.repo_url) != norm(&manifest.repo_url) {
                return Err(Error::Verify(format!(
                    "this update comes from {}, but the installed app came from {}; uninstall it first if you trust the change",
                    manifest.repo_url, installed.repo_url
                )));
            }
        }
        // In parts: every part must carry the maker's attestation for its own hash.
        if !file.parts.is_empty() && file.attestation.is_none() && file.maker_signature.is_none() {
            let expected = || slop_provenance::Expected {
                repo_id: None,
                repo_url: &manifest.repo_url,
                commit: &manifest.commit,
                slop_build_workflow: None,
            };
            let mut first = None;
            for (i, part) in file.parts.iter().enumerate() {
                match &part.attestation {
                    Some(bundle) => {
                        let p = self
                            .provenance
                            .verify_sha256(bundle, &part.sha256, expected())
                            .map_err(|e| {
                                Error::Verify(format!("maker signature on part {}: {e}", i + 1))
                            })?;
                        first.get_or_insert(p);
                    }
                    None if self.require_maker_signature => {
                        return Err(Error::Verify(format!(
                            "part {} has no maker signature, so it can't be installed",
                            i + 1
                        )));
                    }
                    None => {}
                }
            }
            return Ok(first);
        }
        match &file.attestation {
            Some(bundle) => {
                let expected = slop_provenance::Expected {
                    repo_id: None,
                    repo_url: &manifest.repo_url,
                    commit: &manifest.commit,
                    slop_build_workflow: None,
                };
                self.provenance
                    .verify_sha256(bundle, &file.sha256, expected)
                    .map(Some)
                    .map_err(|e| Error::Verify(format!("maker signature: {e}")))
            }
            // A maker's own SSH signature is checked once the file is here (install_into).
            None if file.maker_signature.is_some() => Ok(None),
            None if self.require_maker_signature => Err(Error::Verify(
                "this build has no maker signature, so it can't be installed".into(),
            )),
            None => Ok(None),
        }
    }

    #[allow(clippy::too_many_arguments)] // private helper; each argument is distinct
    async fn install_into(
        &self,
        staging: &Path,
        signed: &Signed,
        manifest: &ReleaseManifest,
        file: &slop_manifest::ManifestFile,
        platform: Platform,
        provenance: Option<slop_provenance::Provenance>,
        links: Option<&ApiClient>,
        progress: &impl Fn(u64, u64),
    ) -> Result<()> {
        let download = staging.join("download");
        let link_path = format!(
            "/v1/apps/{}/releases/{}/download/{}",
            manifest.app_slug,
            manifest.release_id,
            platform.as_str()
        );
        let sha512 = if file.parts.is_empty() {
            self.download_verified(
                &file.url,
                links.map(|api| (api, link_path.as_str())),
                file.size,
                &file.sha256,
                &download,
                progress,
            )
            .await?
        } else {
            self.download_parts_verified(file, links, &link_path, &download, progress)
                .await?
        };
        // Built and signed by hand: the maker's key (vouched for by Slop's signature on the
        // manifest) must have signed exactly this file.
        if file.attestation.is_none()
            && let Some(sig) = &file.maker_signature
        {
            let (key, _) = slop_manifest::sshsig::parse_public_key(&sig.public_key)
                .map_err(|e| Error::Verify(format!("maker signature: {e}")))?;
            let sha256: [u8; 32] = hex::decode(&file.sha256)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| Error::Verify("maker signature: bad digest".into()))?;
            slop_manifest::sshsig::verify(&key, &sig.signature, &sha512, &sha256)
                .map_err(|e| Error::Verify(format!("maker signature: {e}")))?;
        }

        let app = staging.join("app");
        fs::create_dir_all(&app)?;
        let exec_sha256 = 'exec: {
            match file.kind {
                FileKind::Archive => {
                    // A mod has nothing to start: its files are for the game to load.
                    let launch = if platform == Platform::Any {
                        None
                    } else {
                        Some(
                            file.launch
                                .as_ref()
                                .filter(|l| is_safe_relative_path(&l.exec))
                                .ok_or_else(|| {
                                    Error::Verify(
                                        "the manifest has no valid program to start".into(),
                                    )
                                })?,
                        )
                    };
                    let files = app.join("files");
                    let archive_path = download.clone();
                    let files_clone = files.clone();
                    let tarball = is_tar_gz(&file.file_name);
                    tokio::task::spawn_blocking(move || {
                        if tarball {
                            extract_tar_gz(&archive_path, &files_clone)
                        } else {
                            extract_zip(&archive_path, &files_clone)
                        }
                    })
                    .await
                    .map_err(|e| Error::Install(e.to_string()))??;
                    let Some(launch) = launch else {
                        // Nothing to hash: no mod file is ever started.
                        break 'exec None;
                    };
                    let exec = files.join(&launch.exec);
                    if !fs::symlink_metadata(&exec).is_ok_and(|m| m.is_file()) {
                        return Err(Error::Install(format!(
                            "the build doesn't contain {}",
                            launch.exec
                        )));
                    }
                    make_executable(&exec)?;
                    Some(sha256_file(&exec)?)
                }
                FileKind::Installer => {
                    let dir = app.join("installer");
                    fs::create_dir_all(&dir)?;
                    let name = safe_file_name(&file.file_name);
                    fs::rename(&download, dir.join(name))?;
                    Some(file.sha256.clone())
                }
                FileKind::Apk => unreachable!("rejected above"),
            }
        };

        let record = InstallRecord {
            manifest: signed.clone(),
            platform,
            installed_at: OffsetDateTime::now_utc().format(&Rfc3339).expect("formats"),
            exec_sha256,
            provenance,
        };
        fs::write(
            app.join("install.json"),
            serde_json::to_vec_pretty(&record).expect("serialises"),
        )?;
        tracing::info!(app = %manifest.app_slug, version = %manifest.version, "installed");
        Ok(())
    }

    /// Downloads to `dest`, refusing anything bigger than `size` and anything whose SHA-256
    /// doesn't match. The maker's file must be exactly the one Slop scanned and signed.
    /// A file released in parts: each downloaded in order and checked against its own hash, then
    /// the joined file against the whole's. Returns the joined file's SHA-512.
    async fn download_parts_verified(
        &self,
        file: &slop_manifest::ManifestFile,
        links: Option<&ApiClient>,
        link_path: &str,
        dest: &Path,
        progress: &impl Fn(u64, u64),
    ) -> Result<[u8; 64]> {
        let total: u64 = file.parts.iter().map(|p| p.size).sum();
        if total != file.size {
            return Err(Error::Verify("the parts don't add up to the file".into()));
        }
        let mut out = fs::File::create(dest)?;
        let (mut whole256, mut whole512) = (Sha256::new(), sha2::Sha512::new());
        let mut got_total: u64 = 0;
        for (i, part) in file.parts.iter().enumerate() {
            let mut res = self.http.get(self.download_url(&part.url)?).send().await?;
            if matches!(res.status().as_u16(), 401 | 403 | 404)
                && let Some(api) = links
            {
                #[derive(serde::Deserialize)]
                struct Link {
                    url: String,
                }
                let link: Link = api.get_json(&format!("{link_path}?part={}", i + 1)).await?;
                res = self.http.get(self.download_url(&link.url)?).send().await?;
            }
            if !res.status().is_success() {
                return Err(Error::Install(format!(
                    "part {} of the maker's release file is unavailable (HTTP {})",
                    i + 1,
                    res.status().as_u16()
                )));
            }
            let mut part256 = Sha256::new();
            let mut got: u64 = 0;
            while let Some(chunk) = res.chunk().await? {
                got += chunk.len() as u64;
                if got > part.size {
                    return Err(Error::Verify(format!(
                        "part {} changed after Slop checked it (it's bigger)",
                        i + 1
                    )));
                }
                part256.update(&chunk);
                whole256.update(&chunk);
                whole512.update(&chunk);
                out.write_all(&chunk)?;
                got_total += chunk.len() as u64;
                progress(got_total, total);
            }
            if got != part.size || hex::encode(part256.finalize()) != part.sha256 {
                return Err(Error::Verify(format!(
                    "part {} changed after Slop checked it; not installing",
                    i + 1
                )));
            }
        }
        out.sync_all()?;
        if hex::encode(whole256.finalize()) != file.sha256 {
            return Err(Error::Verify(
                "the joined file isn't the one Slop checked; not installing".into(),
            ));
        }
        Ok(whole512.finalize().into())
    }

    async fn download_verified(
        &self,
        url: &str,
        links: Option<(&ApiClient, &str)>,
        size: u64,
        sha256: &str,
        dest: &Path,
        progress: &impl Fn(u64, u64),
    ) -> Result<[u8; 64]> {
        let mut res = self.http.get(self.download_url(url)?).send().await?;
        // Private repositories: the release file isn't public, so ask Slop for a short-lived link.
        if matches!(res.status().as_u16(), 401 | 403 | 404)
            && let Some((api, path)) = links
        {
            #[derive(serde::Deserialize)]
            struct Link {
                url: String,
            }
            let link: Link = api.get_json(path).await?;
            res = self.http.get(self.download_url(&link.url)?).send().await?;
        }
        if !res.status().is_success() {
            return Err(Error::Install(format!(
                "the maker's release file is unavailable (HTTP {})",
                res.status().as_u16()
            )));
        }
        let mut out = fs::File::create(dest)?;
        let mut hasher = Sha256::new();
        let mut hasher512 = sha2::Sha512::new();
        let mut got: u64 = 0;
        while let Some(chunk) = res.chunk().await? {
            got += chunk.len() as u64;
            if got > size {
                return Err(Error::Verify(
                    "the file changed after Slop checked it (it's bigger)".into(),
                ));
            }
            hasher.update(&chunk);
            hasher512.update(&chunk);
            out.write_all(&chunk)?;
            progress(got, size);
        }
        out.sync_all()?;
        if got != size || hex::encode(hasher.finalize()) != sha256 {
            return Err(Error::Verify(
                "the file changed after Slop checked it; not installing".into(),
            ));
        }
        Ok(hasher512.finalize().into())
    }

    fn download_url(&self, url: &str) -> Result<url::Url> {
        let url = url::Url::parse(url).map_err(|_| Error::Verify("bad download URL".into()))?;
        if url.scheme() != "https" && !(self.allow_http && url.scheme() == "http") {
            return Err(Error::Verify("downloads must use https".into()));
        }
        Ok(url)
    }

    /// Uninstalls an app and the mods added to it. Its saves stay (`delete_saves` removes them).
    pub fn uninstall(&self, slug: &str) -> Result<()> {
        let dir = self.app_dir(slug)?;
        if !dir.exists() {
            return Err(Error::Install(format!("{slug} isn't installed")));
        }
        let mods = self.installed_mods(slug);
        self.remove_dir(&dir)?;
        // Its sandbox on Windows; the saves keep their permissions for a reinstall, which gets
        // the same container back.
        #[cfg(windows)]
        crate::appcontainer::remove(&crate::sandbox::container_name(slug));
        for m in mods {
            if let Ok(dir) = self.mod_dir(&m.slug) {
                let _ = self.remove_dir(&dir);
            }
        }
        Ok(())
    }

    /// The folders of the mods to hand to a game at launch: its mods, minus any pulled.
    fn approved_mods(
        &self,
        game_id: &str,
        revocations: Option<&RevocationList>,
    ) -> Vec<(String, PathBuf)> {
        self.all_mods()
            .into_iter()
            .filter(|(_, id)| id == game_id)
            .filter_map(|(m, _)| {
                if let Some(r) = revocations.and_then(|r| r.is_revoked(&m.release_id)) {
                    tracing::warn!(r#mod = %m.slug, reason = %r.reason, "not loading a pulled mod");
                    return None;
                }
                let dir = self.mod_dir(&m.slug).ok()?.join("files");
                Some((m.slug, dir))
            })
            .collect()
    }

    /// The game's own mods folder, when its maker named one (DESIGN §9).
    fn declared_mods_folder(
        &self,
        slug: &str,
        manifest: &ReleaseManifest,
    ) -> Result<Option<(PathBuf, slop_manifest::ModsLayout)>> {
        let Some(f) = manifest.mods_folder.as_ref().filter(|f| f.is_valid()) else {
            return Ok(None);
        };
        let base = match f.base {
            slop_manifest::ModsBase::Game => self.app_dir(slug)?.join("files"),
            slop_manifest::ModsBase::Saves => self.save_dir(slug)?,
        };
        Ok(Some((base.join(&f.path), f.layout)))
    }

    /// Puts the game's approved mods in its declared mods folder, moving anything else out.
    /// Returns the mods' folders (as in `SLOP_MODS`) and what was moved out. Called before each
    /// start, and when the player adds mods while the game runs.
    pub fn sync_mods(
        &self,
        slug: &str,
        revocations: Option<&RevocationList>,
    ) -> Result<(Vec<PathBuf>, crate::mods_folder::Synced)> {
        let (_, manifest) = self.record(slug)?;
        let mods = self.approved_mods(&manifest.app_id, revocations);
        let synced = match self.declared_mods_folder(slug, &manifest)? {
            Some((target, layout)) => crate::mods_folder::sync(
                &target,
                layout,
                &mods,
                &self.root.join("mod-sync").join(format!("{slug}.json")),
                &self
                    .root
                    .join("quarantine")
                    .join(slug)
                    .join("unapproved-mods"),
            )?,
            None => Default::default(),
        };
        Ok((mods.into_iter().map(|(_, dir)| dir).collect(), synced))
    }

    /// Builds the command that starts an installed app in normal mode, after checking the
    /// manifest signature, the revocation list and the program's hash.
    pub fn launch_command(
        &self,
        slug: &str,
        revocations: Option<&RevocationList>,
    ) -> Result<Command> {
        Ok(self.prepare_launch(slug, revocations, None, None)?.command)
    }

    /// Checks an installed app, sweeps its save folder, and builds the command that starts it:
    /// in sandbox mode when `sandbox` is given (a copy of the saves is taken first).
    /// `link` is the run's Slop Link folder (link.rs), when the caller watches one.
    pub fn prepare_launch(
        &self,
        slug: &str,
        revocations: Option<&RevocationList>,
        sandbox: Option<AppSandbox>,
        link: Option<&Path>,
    ) -> Result<Launch> {
        let (record, manifest) = self.record(slug)?;
        if let Some(revoked) = revocations.and_then(|r| r.is_revoked(&manifest.release_id)) {
            return Err(Error::Revoked(revoked.reason.clone()));
        }
        let file = manifest
            .file_for(record.platform)
            .ok_or_else(|| Error::Verify("install record doesn't match its manifest".into()))?;
        if file.kind != FileKind::Archive {
            return Err(Error::Install(
                "this app runs its own installer; start it from your system".into(),
            ));
        }
        let launch = file
            .launch
            .as_ref()
            .filter(|l| is_safe_relative_path(&l.exec))
            .ok_or_else(|| Error::Verify("the manifest has no valid program to start".into()))?;
        let files = self.app_dir(slug)?.join("files");
        let exec = files.join(&launch.exec);
        let actual =
            sha256_file(&exec).map_err(|_| Error::Verify(format!("{} is missing", launch.exec)))?;
        if Some(actual) != record.exec_sha256 {
            return Err(Error::Verify(
                "the app's files changed since it was installed; reinstall it".into(),
            ));
        }

        // The save folder: swept every time (bar a mods folder in it), copied before a
        // sandboxed session.
        let saves = self.save_dir(slug)?;
        let mods_folder = self.declared_mods_folder(slug, &manifest)?;
        let in_saves = mods_folder
            .as_ref()
            .map(|(dir, _)| dir.clone())
            .filter(|dir| dir.starts_with(&saves));
        let quarantined = crate::saves::sweep(
            &saves,
            &self.root.join("quarantine").join(slug),
            in_saves.as_deref(),
        )?;
        let mut copy_skipped = false;
        if sandbox.is_some() {
            copy_skipped =
                crate::saves::take_copy(&saves, &self.root.join("save-copies").join(slug))?
                    .is_none()
                    && crate::saves::size(&saves)? > 0;
        }
        // A sandboxed session gets a cache of its own, emptied each time, so nothing it leaves
        // there (libraries, compiled shaders or scripts) is loaded by a later normal session.
        let cache = match sandbox {
            Some(_) => {
                let c = self.root.join("cache-sandbox").join(slug);
                if c.exists() {
                    fs::remove_dir_all(&c)?;
                }
                c
            }
            None => self.root.join("cache").join(slug),
        };
        fs::create_dir_all(&cache)?;
        let (mods, synced) = self.sync_mods(slug, revocations)?;
        // Slop Link: the game writes requests and reads answers.
        let read_only: Vec<PathBuf> = in_saves
            .into_iter()
            .chain(link.map(|l| l.join("answers")))
            .collect();
        let writable: Vec<PathBuf> = link.map(|l| l.join("requests")).into_iter().collect();

        let mut cmd = match sandbox {
            Some(opts) => crate::sandbox::command(
                &crate::sandbox::Sandbox {
                    files: &files,
                    mods: &mods,
                    saves: &saves,
                    cache: &cache,
                    read_only: &read_only,
                    writable: &writable,
                    network: !opts.block_network,
                },
                &exec,
                &launch.args,
                crate::sandbox::container_name(slug),
            )
            .map_err(|e| Error::Install(e.to_string()))?,
            None => {
                let mut cmd = Command::new(&exec);
                cmd.args(&launch.args).current_dir(&files);
                cmd
            }
        };
        // Where the app keeps saves, the same in both modes (DESIGN §6). The usual per-user
        // folders point there too, so engines that follow them (Godot, and Unity and most
        // others on Linux) need no change.
        cmd.env("SLOP_SAVE_DIR", &saves);
        #[cfg(target_os = "linux")]
        cmd.env("XDG_DATA_HOME", saves.join(".share"))
            .env("XDG_CONFIG_HOME", saves.join(".config"))
            .env("XDG_STATE_HOME", saves.join(".state"))
            .env("XDG_CACHE_HOME", &cache);
        #[cfg(windows)]
        cmd.env("APPDATA", saves.join("AppData").join("Roaming"))
            .env("LOCALAPPDATA", saves.join("AppData").join("Local"));
        if let Some(link) = link {
            cmd.env("SLOP_LINK", link);
        }
        // The game's mods (DESIGN §9): a list of folders, separated like PATH.
        if !mods.is_empty()
            && let Ok(list) = std::env::join_paths(&mods)
        {
            cmd.env("SLOP_MODS", list);
        }
        Ok(Launch {
            command: cmd,
            sandboxed: sandbox.is_some(),
            quarantined,
            copy_skipped,
            unapproved_mods: synced.unapproved,
        })
    }

    // -----------------------------------------------------------------------------------------
    // Saves and sandbox settings

    /// An app's save folder, created if needed.
    /// For save sync (save_sync.rs): the app's save folder, its copies, a scratch folder, and
    /// where the sync state is kept.
    pub fn save_sync_paths(&self, slug: &str) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf)> {
        Ok((
            self.save_dir(slug)?,
            self.root.join("save-copies").join(slug),
            self.root.join("staging"),
            self.root.join("save-sync.json"),
        ))
    }

    pub fn save_dir(&self, slug: &str) -> Result<PathBuf> {
        if !valid_slug(slug) {
            return Err(Error::Install(format!("not an app: {slug:?}")));
        }
        let dir = self.root.join("saves").join(slug);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Copies of an app's saves, newest first.
    pub fn save_copies(&self, slug: &str) -> Result<Vec<crate::saves::SaveCopy>> {
        self.save_dir(slug)?;
        Ok(crate::saves::copies(
            &self.root.join("save-copies").join(slug),
        ))
    }

    /// Puts a copy of an app's saves back (the current ones are copied first).
    pub fn restore_saves(&self, slug: &str, id: &str) -> Result<()> {
        let saves = self.save_dir(slug)?;
        crate::saves::restore(
            &saves,
            &self.root.join("save-copies").join(slug),
            id,
            &self.root.join("staging"),
        )
        .map_err(|e| Error::Install(format!("couldn't restore the saves: {e}")))
    }

    /// Replaces `to`'s saves with a copy of `from`'s (switching to a community build). `to`'s
    /// current saves, if any, are copied first.
    pub fn copy_saves(&self, from: &str, to: &str) -> Result<()> {
        let source = self.save_dir(from)?;
        let target = self.save_dir(to)?;
        crate::saves::take_copy(&target, &self.root.join("save-copies").join(to))?;
        let incoming = self
            .root
            .join("staging")
            .join(format!("saves-{}", crate::random_token()));
        crate::saves::copy_tree(&source, &incoming)?;
        crate::saves::replace_dir(&target, &incoming, &self.root.join("staging"))?;
        Ok(())
    }

    /// Deletes an app's saves and their copies.
    pub fn delete_saves(&self, slug: &str) -> Result<()> {
        self.app_dir(slug)?;
        for dir in [
            "saves",
            "save-copies",
            "quarantine",
            "cache",
            "cache-sandbox",
        ] {
            let path = self.root.join(dir).join(slug);
            if path.exists() {
                self.remove_dir(&path)?;
            }
        }
        Ok(())
    }

    /// The shared runtimes an installed app needs and this computer doesn't have (Windows).
    pub fn missing_runtimes(
        &self,
        slug: &str,
    ) -> Result<Vec<&'static slop_manifest::runtimes::Runtime>> {
        let (_, manifest) = self.record(slug)?;
        Ok(manifest
            .runtimes
            .iter()
            .filter_map(|id| slop_manifest::runtimes::runtime(id))
            .filter(|r| !crate::runtimes::installed(r))
            .collect())
    }

    /// Installs the shared runtimes an app is missing, one at a time, from their vendors.
    pub async fn install_runtimes(&self, slug: &str) -> Result<()> {
        for r in self.missing_runtimes(slug)? {
            crate::runtimes::install(&self.http, r, &self.root.join("staging")).await?;
        }
        Ok(())
    }

    /// Bytes in an app's save folder, so the UI can offer to copy saves into an empty one.
    pub fn save_size(&self, slug: &str) -> Result<u64> {
        Ok(crate::saves::size(&self.save_dir(slug)?)?)
    }

    fn play_choices_path(&self) -> PathBuf {
        self.root.join("settings").join("play.json")
    }

    /// Which installed community build the player picked to start from each original's Play
    /// button (original → build). Picks whose build is gone or no longer the original's are
    /// left out.
    pub fn play_choices(&self) -> BTreeMap<String, String> {
        let saved: BTreeMap<String, String> = fs::read(self.play_choices_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let installed = self.installed();
        saved
            .into_iter()
            .filter(|(app, build)| {
                installed
                    .iter()
                    .any(|a| &a.slug == build && a.build_of.as_ref() == Some(app))
            })
            .collect()
    }

    /// Picks what the original `app`'s Play button starts: an installed community build of it,
    /// or the original itself (`None`, or `app`).
    pub fn set_play_choice(&self, app: &str, build: Option<&str>) -> Result<()> {
        if !valid_slug(app) {
            return Err(Error::Install(format!("not an app: {app:?}")));
        }
        let mut choices = self.play_choices();
        match build.filter(|b| *b != app) {
            Some(build) => {
                let ok = self
                    .installed()
                    .iter()
                    .any(|a| a.slug == build && a.build_of.as_deref() == Some(app));
                if !ok {
                    return Err(Error::Install(format!(
                        "{build} isn't an installed build of {app}"
                    )));
                }
                choices.insert(app.to_string(), build.to_string());
            }
            None => {
                choices.remove(app);
            }
        }
        fs::create_dir_all(self.root.join("settings"))?;
        fs::write(
            self.play_choices_path(),
            serde_json::to_vec_pretty(&choices).expect("serialises"),
        )?;
        Ok(())
    }

    fn sandbox_path(&self) -> PathBuf {
        self.root.join("settings").join("sandbox.json")
    }

    pub fn sandbox_settings(&self) -> SandboxSettings {
        fs::read(self.sandbox_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Changes and saves the player's sandbox settings.
    pub fn set_sandbox(
        &self,
        change: impl FnOnce(&mut SandboxSettings),
    ) -> Result<SandboxSettings> {
        let mut settings = self.sandbox_settings();
        change(&mut settings);
        settings.apps.retain(|slug, _| valid_slug(slug));
        fs::write(
            self.sandbox_path(),
            serde_json::to_vec_pretty(&settings).expect("serialises"),
        )?;
        Ok(settings)
    }

    /// The verified installer of an app that ships one, for the user to run.
    pub fn installer_path(
        &self,
        slug: &str,
        revocations: Option<&RevocationList>,
    ) -> Result<PathBuf> {
        let (record, manifest) = self.record(slug)?;
        if let Some(revoked) = revocations.and_then(|r| r.is_revoked(&manifest.release_id)) {
            return Err(Error::Revoked(revoked.reason.clone()));
        }
        let file = manifest
            .file_for(record.platform)
            .filter(|f| f.kind == FileKind::Installer)
            .ok_or_else(|| Error::Install("this app has no installer".into()))?;
        let path = self
            .app_dir(slug)?
            .join("installer")
            .join(safe_file_name(&file.file_name));
        if sha256_file(&path).ok().as_deref() != Some(file.sha256.as_str()) {
            return Err(Error::Verify(
                "the installer changed since it was downloaded; reinstall".into(),
            ));
        }
        Ok(path)
    }

    // -----------------------------------------------------------------------------------------
    // Revocations

    fn revocations_path(&self) -> PathBuf {
        self.root.join("revocations.json")
    }

    /// The last revocation list saved, if its signature still checks out (it may be expired).
    pub fn cached_revocations(&self) -> Option<RevocationList> {
        let signed: Signed =
            serde_json::from_slice(&fs::read(self.revocations_path()).ok()?).ok()?;
        slop_manifest::verify(&signed, Purpose::RevocationList, &self.trusted).ok()
    }

    /// Fetches and verifies the current revocation list, and saves it for offline use.
    pub async fn refresh_revocations(&self, api: &ApiClient) -> Result<RevocationList> {
        let signed: Signed = api.get_json("/v1/revocations").await?;
        let list: RevocationList =
            slop_manifest::verify(&signed, Purpose::RevocationList, &self.trusted)
                .map_err(|e| Error::Verify(format!("revocation list: {e}")))?;
        if list.is_expired(OffsetDateTime::now_utc()) {
            return Err(Error::Verify("the revocation list is out of date".into()));
        }
        // Never go back to an older list than the one already saved.
        if let Some(cached) = self.cached_revocations()
            && cached.issued_at > list.issued_at
        {
            return Err(Error::Verify(
                "got an older revocation list than the saved one".into(),
            ));
        }
        fs::write(
            self.revocations_path(),
            serde_json::to_vec(&signed).expect("serialises"),
        )?;
        Ok(list)
    }

    /// The revocation list to check at launch: fresh when online, otherwise the saved one, so
    /// people can still play offline (SECURITY §5.5).
    pub async fn revocations_for_launch(&self, api: &ApiClient) -> Option<RevocationList> {
        match self.refresh_revocations(api).await {
            Ok(list) => Some(list),
            Err(err) => {
                tracing::warn!(error = %err, "using the saved revocation list");
                self.cached_revocations()
            }
        }
    }
}

fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "._- ()".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "installer".into()
    } else {
        cleaned
    }
}

/// Unpacks a zip into `dest`. Entries that would land outside `dest`, symlinks and absurd sizes
/// are refused (zip-slip and zip-bomb protection).
fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    const MAX_TOTAL: u64 = 16 * 1024 * 1024 * 1024;
    const MAX_ENTRIES: usize = 200_000;
    let bad = |msg: &str| Error::Verify(format!("the archive is unsafe: {msg}"));

    let mut archive =
        zip::ZipArchive::new(fs::File::open(zip_path)?).map_err(|_| bad("not a zip file"))?;
    if archive.len() > MAX_ENTRIES {
        return Err(bad("too many files"));
    }
    fs::create_dir_all(dest)?;
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|_| bad("damaged entry"))?;
        if entry.is_symlink() {
            return Err(bad("contains a symbolic link"));
        }
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| bad("a path points outside the app's folder"))?;
        let out = dest.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out)?;
        // Count real bytes, not the size the archive claims.
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry.read(&mut buf).map_err(|_| bad("damaged data"))?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > MAX_TOTAL {
                return Err(bad("unpacks to more than 16 GB"));
            }
            file.write_all(&buf[..n])?;
        }
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            // Keep the exec bits, drop setuid/setgid/sticky and group/world write.
            fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o755))?;
        }
    }
    Ok(())
}

/// Archives are zips, except `.tar.gz`/`.tgz` (the usual Linux format, which keeps the
/// program's execute permission).
fn is_tar_gz(file_name: &str) -> bool {
    let name = file_name.to_ascii_lowercase();
    name.ends_with(".tar.gz") || name.ends_with(".tgz")
}

/// Unpacks a `.tar.gz` with the same rules as `extract_zip`: plain files and folders only,
/// every path inside `dest`, and a cap on the unpacked size.
fn extract_tar_gz(path: &Path, dest: &Path) -> Result<()> {
    const MAX_TOTAL: u64 = 16 * 1024 * 1024 * 1024;
    const MAX_ENTRIES: usize = 200_000;
    let bad = |msg: &str| Error::Verify(format!("the archive is unsafe: {msg}"));

    let gz = flate2::read::GzDecoder::new(io::BufReader::new(fs::File::open(path)?));
    let mut archive = tar::Archive::new(gz);
    fs::create_dir_all(dest)?;
    let mut total: u64 = 0;
    let entries = archive.entries().map_err(|_| bad("not a .tar.gz file"))?;
    for (count, entry) in entries.enumerate() {
        if count >= MAX_ENTRIES {
            return Err(bad("too many files"));
        }
        let mut entry = entry.map_err(|_| bad("damaged entry"))?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() {
            continue;
        }
        if !(kind.is_file() || kind.is_dir()) {
            return Err(bad("contains a link or special file"));
        }
        let rel = entry
            .path()
            .map_err(|_| bad("a path isn't valid"))?
            .into_owned();
        let mut clean = PathBuf::new();
        for part in rel.components() {
            match part {
                std::path::Component::Normal(p) => clean.push(p),
                std::path::Component::CurDir => {}
                _ => return Err(bad("a path points outside the app's folder")),
            }
        }
        if clean.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&clean);
        if kind.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out)?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry.read(&mut buf).map_err(|_| bad("damaged data"))?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > MAX_TOTAL {
                return Err(bad("unpacks to more than 16 GB"));
            }
            file.write_all(&buf[..n])?;
        }
        #[cfg(unix)]
        if let Ok(mode) = entry.header().mode() {
            use std::os::unix::fs::PermissionsExt;
            // Keep the exec bits, drop setuid/setgid/sticky and group/world write.
            fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o755))?;
        }
    }
    Ok(())
}

fn make_executable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)?.permissions().mode();
        fs::set_permissions(path, fs::Permissions::from_mode((mode | 0o500) & 0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Who signed a build, for keeping updates to the same signer (`Library::check_maker_signature`).
#[derive(Debug, PartialEq, Eq)]
enum Signer {
    /// The maker's SSH key (`ssh-ed25519 <base64>`).
    Key(String),
    /// A GitHub build attestation from this repository: (repository ID, owner ID), when the
    /// certificate names them (attestations made before GitHub added them don't).
    Repository(Option<(String, String)>),
    /// No maker signature (only on test setups that allow it).
    Unsigned,
}

impl Signer {
    /// Why an update signed by `now` can't replace one signed by `was`, if it can't.
    fn changed(was: &Signer, now: &Signer) -> Option<&'static str> {
        match (was, now) {
            (Signer::Unsigned, _) => None,
            (Signer::Key(a), Signer::Key(b)) if a == b => None,
            (Signer::Key(_), Signer::Key(_)) => {
                Some("this update is signed with a different maker key than the installed app")
            }
            (Signer::Repository(Some(a)), Signer::Repository(Some(b))) if a != b => Some(
                "this update was built in a different repository than the installed app, though it has the same name",
            ),
            (Signer::Repository(_), Signer::Repository(_)) => None,
            (_, Signer::Unsigned) => {
                Some("the installed app was signed by its maker, but this update isn't")
            }
            _ => Some("this update is signed differently than the installed app"),
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn updates_keep_the_installed_signer() {
        let key = |k: &str| Signer::Key(k.into());
        let repo = |r: &str| Signer::Repository(Some((r.into(), "42".into())));
        assert_eq!(
            Signer::changed(&key("ssh-ed25519 AAA"), &key("ssh-ed25519 AAA")),
            None
        );
        assert!(Signer::changed(&key("ssh-ed25519 AAA"), &key("ssh-ed25519 BBB")).is_some());
        assert_eq!(Signer::changed(&repo("1001"), &repo("1001")), None);
        assert!(
            Signer::changed(&repo("1001"), &repo("9999")).is_some(),
            "repojacking"
        );
        assert_eq!(
            Signer::changed(&Signer::Repository(None), &repo("1001")),
            None
        );
        assert!(Signer::changed(&repo("1001"), &key("ssh-ed25519 AAA")).is_some());
        assert!(Signer::changed(&key("ssh-ed25519 AAA"), &Signer::Unsigned).is_some());
        assert_eq!(
            Signer::changed(&Signer::Unsigned, &key("ssh-ed25519 AAA")),
            None
        );
    }
    use super::*;

    #[test]
    fn slugs_and_names() {
        assert!(valid_slug("pixel-dungeon"));
        assert!(!valid_slug("../etc"));
        assert!(!valid_slug("A"));
        assert_eq!(safe_file_name("Setup 1.0.exe"), "Setup 1.0.exe");
        assert_eq!(safe_file_name("../../evil.sh"), "_.._evil.sh");
        assert_eq!(safe_file_name("..."), "installer");
    }
}
