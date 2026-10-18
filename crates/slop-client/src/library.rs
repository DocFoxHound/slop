//! Installing, verifying and launching apps (ARCHITECTURE §6.3, SECURITY §5.3, §5.5, §7).
//!
//! Layout under the library root (in the user's profile, so no admin rights are needed):
//!
//! ```text
//! apps/<slug>/install.json    the signed manifest and what was installed
//! apps/<slug>/files/…         an archive's contents
//! apps/<slug>/installer/…     a maker's installer, kept for the user to run
//! staging/                    downloads and unpacking in progress
//! revocations.json            the last signed revocation list
//! ```
//!
//! Nothing is written into `apps/` until the download matches the size and SHA-256 in a manifest
//! signed by a trusted Slop key. Every launch re-checks the manifest signature, the revocation
//! list and the hash of the program being started, so tampering with the library folder can't
//! slip anything past verification.

use std::{
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

use crate::{Error, Result, api::ApiClient};

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

    fn verify_manifest(&self, signed: &Signed) -> Result<ReleaseManifest> {
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
        if manifest.app_slug != slug {
            return Err(Error::Verify(
                "install record belongs to another app".into(),
            ));
        }
        Ok((record, manifest))
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
                })
            })
            .collect();
        apps.sort_by_key(|a| a.title.to_lowercase());
        apps
    }

    /// Downloads, verifies and installs the build for `platform` from a signed manifest.
    /// `progress` gets (bytes so far, total bytes).
    pub async fn install(
        &self,
        signed: &Signed,
        platform: Platform,
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
        let provenance = self.check_maker_signature(&manifest, &file)?;
        let final_dir = self.app_dir(&manifest.app_slug)?;

        // A fresh staging folder for this attempt.
        let staging = self.root.join("staging").join(crate::random_token());
        fs::create_dir_all(&staging)?;
        let result = self
            .install_into(
                &staging, signed, &manifest, &file, platform, provenance, &progress,
            )
            .await;
        if let Err(err) = result {
            let _ = fs::remove_dir_all(&staging);
            return Err(err);
        }

        // Swap the new install into place; the old one is removed afterwards.
        let old = self
            .root
            .join("staging")
            .join(format!("old-{}", crate::random_token()));
        if final_dir.exists() {
            fs::rename(&final_dir, &old)?;
        }
        if let Err(err) = fs::rename(staging.join("app"), &final_dir) {
            if old.exists() {
                let _ = fs::rename(&old, &final_dir);
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(err.into());
        }
        let _ = fs::remove_dir_all(&old);
        let _ = fs::remove_dir_all(&staging);

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
        })
    }

    /// Checks the maker's signature on a build, before anything is downloaded.
    fn check_maker_signature(
        &self,
        manifest: &ReleaseManifest,
        file: &slop_manifest::ManifestFile,
    ) -> Result<Option<slop_provenance::Provenance>> {
        // An update must come from the same repository as what's installed: not even Slop can
        // quietly repoint an installed app at someone else's code.
        if let Ok((_, installed)) = self.record(&manifest.app_slug) {
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
        match &file.attestation {
            Some(bundle) => {
                let expected = slop_provenance::Expected {
                    repo_url: &manifest.repo_url,
                    commit: &manifest.commit,
                    slop_build_workflow: None,
                };
                self.provenance
                    .verify_sha256(bundle, &file.sha256, expected)
                    .map(Some)
                    .map_err(|e| Error::Verify(format!("maker signature: {e}")))
            }
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
        progress: &impl Fn(u64, u64),
    ) -> Result<()> {
        let download = staging.join("download");
        self.download_verified(&file.url, file.size, &file.sha256, &download, progress)
            .await?;

        let app = staging.join("app");
        fs::create_dir_all(&app)?;
        let exec_sha256 = match file.kind {
            FileKind::Archive => {
                let launch = file
                    .launch
                    .as_ref()
                    .filter(|l| is_safe_relative_path(&l.exec))
                    .ok_or_else(|| {
                        Error::Verify("the manifest has no valid program to start".into())
                    })?;
                let files = app.join("files");
                let zip_path = download.clone();
                let files_clone = files.clone();
                tokio::task::spawn_blocking(move || extract_zip(&zip_path, &files_clone))
                    .await
                    .map_err(|e| Error::Install(e.to_string()))??;
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
    async fn download_verified(
        &self,
        url: &str,
        size: u64,
        sha256: &str,
        dest: &Path,
        progress: &impl Fn(u64, u64),
    ) -> Result<()> {
        let url = url::Url::parse(url).map_err(|_| Error::Verify("bad download URL".into()))?;
        if url.scheme() != "https" && !(self.allow_http && url.scheme() == "http") {
            return Err(Error::Verify("downloads must use https".into()));
        }
        let mut res = self.http.get(url).send().await?;
        if !res.status().is_success() {
            return Err(Error::Install(format!(
                "the maker's release file is unavailable (HTTP {})",
                res.status().as_u16()
            )));
        }
        let mut out = fs::File::create(dest)?;
        let mut hasher = Sha256::new();
        let mut got: u64 = 0;
        while let Some(chunk) = res.chunk().await? {
            got += chunk.len() as u64;
            if got > size {
                return Err(Error::Verify(
                    "the file changed after Slop scanned it (it's bigger)".into(),
                ));
            }
            hasher.update(&chunk);
            out.write_all(&chunk)?;
            progress(got, size);
        }
        out.sync_all()?;
        if got != size || hex::encode(hasher.finalize()) != sha256 {
            return Err(Error::Verify(
                "the file changed after Slop scanned it; not installing".into(),
            ));
        }
        Ok(())
    }

    pub fn uninstall(&self, slug: &str) -> Result<()> {
        let dir = self.app_dir(slug)?;
        if !dir.exists() {
            return Err(Error::Install(format!("{slug} isn't installed")));
        }
        // Move aside first so a half-deleted folder never looks installed.
        let trash = self
            .root
            .join("staging")
            .join(format!("old-{}", crate::random_token()));
        fs::rename(&dir, &trash)?;
        fs::remove_dir_all(&trash)?;
        Ok(())
    }

    /// Builds the command that starts an installed app, after checking the manifest signature,
    /// the revocation list and the program's hash.
    pub fn launch_command(
        &self,
        slug: &str,
        revocations: Option<&RevocationList>,
    ) -> Result<Command> {
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
        let mut cmd = Command::new(exec);
        cmd.args(&launch.args).current_dir(files);
        Ok(cmd)
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

#[cfg(test)]
mod tests {
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
