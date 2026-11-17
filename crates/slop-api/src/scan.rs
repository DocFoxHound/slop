// SPDX-License-Identifier: AGPL-3.0-or-later
//! The scan step of the release pipeline (DESIGN §10, SECURITY §5.2).
//!
//! Two parts, both free:
//!
//! - **SLAPorSLOP's scanner** (`SLOP_SCANNER=local`): every release file is downloaded once and handed
//!   to `SLOP_SCAN_COMMAND` (`deploy/scanner/slop-scan`), which scans it in a container with no
//!   network: ClamAV, YARA with YARA-Forge's core rules, and Mandiant's capa. Static only;
//!   nothing in a build is run. It prints a verdict as JSON.
//! - **Microsoft Defender** for Windows builds, which SLAPorSLOP can't run on Linux: Slop Build (and
//!   Slop Check, for builds made elsewhere) scans them on GitHub's Windows runners in the
//!   maker's own repository, with today's definitions, and reports to SLAPorSLOP with the run's OIDC
//!   token (slop_build.rs). That catches what would stop players' Windows from running the app
//!   before it's published.
//!
//! Findings put a file **on hold** (holds.rs): scanners have false positives, so SLAPorSLOP staff
//! can clear one, with or without a note for the maker (admin.rs), which is audit-logged, and
//! the maker can have the scans run again with newer signatures. With the scanner off
//! (test and development servers) files are `skipped`, and nothing in the UI claims a scan.
//!
//! The scanner also reports **fingerprints** of the programs and scripts inside a file (whole-file
//! and Windows section hashes, deploy/scanner/hashes.py). SLAPorSLOP keeps them
//! (`release_file_fingerprints`), and rescan.rs checks them against ClamAV's newer signatures
//! (`Scanner::match_fingerprints`), so a release doesn't have to be downloaded and scanned again
//! to learn that a program in it has become known malware.

use std::{path::Path, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// Result scale from DESIGN §10, plus `Skipped` for servers without a scanner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanResult {
    Passed,
    Note,
    OnHold,
    Rejected,
    Blocked,
    Skipped,
}

impl ScanResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Note => "note",
            Self::OnHold => "on_hold",
            Self::Rejected => "rejected",
            Self::Blocked => "blocked",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [
            Self::Passed,
            Self::Note,
            Self::OnHold,
            Self::Rejected,
            Self::Blocked,
            Self::Skipped,
        ]
        .into_iter()
        .find(|r| r.as_str() == value)
    }
}

#[derive(Clone, Debug)]
pub struct Verdict {
    pub result: ScanResult,
    pub detail: Option<String>,
    /// The scanner's report: engines and versions, findings, capabilities.
    pub report: Option<Value>,
    /// The programs and scripts inside, by their fingerprints (kept apart from the report).
    pub fingerprints: Vec<Fingerprint>,
}

/// A whole file's or a section's hashes, as ClamAV's fingerprint signatures take them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Digests {
    pub size: i64,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}

impl Digests {
    fn valid(&self) -> bool {
        let hex =
            |s: &str, n| s.len() == n && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        self.size >= 0 && hex(&self.md5, 32) && hex(&self.sha1, 40) && hex(&self.sha256, 64)
    }
}

/// A program or script inside a release file: where it is, its hashes, and its sections' if
/// it's a Windows program.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Fingerprint {
    pub file: String,
    #[serde(flatten)]
    pub digests: Digests,
    #[serde(default)]
    pub sections: Vec<Digests>,
}

/// Most fingerprints kept per file, and sections per program (deploy/scanner/hashes.py's limits).
const MAX_FINGERPRINTS: usize = 3000;
const MAX_SECTIONS: usize = 32;

/// The fingerprints in a scanner report, leaving out any that aren't well-formed.
fn fingerprints(value: Option<Value>) -> Vec<Fingerprint> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Fingerprint>(v).ok())
        .filter(|f| f.file.chars().count() <= 1000 && f.digests.valid())
        .map(|mut f| {
            f.sections.retain(Digests::valid);
            f.sections.truncate(MAX_SECTIONS);
            f
        })
        .take(MAX_FINGERPRINTS)
        .collect()
}

/// Keeps a release file's fingerprints, in place of its earlier ones. A scan that reported none
/// (it couldn't finish, or the scanner is off) leaves the earlier ones.
pub async fn save_fingerprints(
    db: &PgPool,
    release_id: Uuid,
    platform: &str,
    prints: &[Fingerprint],
) -> sqlx::Result<()> {
    if prints.is_empty() {
        return Ok(());
    }
    let (mut file, mut section, mut size, mut md5, mut sha1, mut sha256) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    for p in prints {
        let whole = std::iter::once((None, &p.digests));
        let parts = p
            .sections
            .iter()
            .enumerate()
            .map(|(i, d)| (Some(i as i16), d));
        for (index, d) in whole.chain(parts) {
            file.push(p.file.clone());
            section.push(index);
            size.push(d.size);
            md5.push(d.md5.clone());
            sha1.push(d.sha1.clone());
            sha256.push(d.sha256.clone());
        }
    }
    let mut tx = db.begin().await?;
    sqlx::query("delete from release_file_fingerprints where release_id = $1 and platform = $2")
        .bind(release_id)
        .bind(platform)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "insert into release_file_fingerprints (release_id, platform, file, section, size, md5, sha1, sha256)
         select $1, $2, * from unnest($3::text[], $4::smallint[], $5::bigint[], $6::text[], $7::text[], $8::text[])",
    )
    .bind(release_id)
    .bind(platform)
    .bind(&file)
    .bind(&section)
    .bind(&size)
    .bind(&md5)
    .bind(&sha1)
    .bind(&sha256)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// One fingerprint to check, for `Scanner::match_fingerprints`.
#[derive(Serialize)]
pub struct Query<'a> {
    /// The caller's name for it, given back with a match.
    pub id: &'a str,
    /// "file" or "section".
    pub kind: &'static str,
    pub size: i64,
    pub md5: &'a str,
    pub sha1: &'a str,
    pub sha256: &'a str,
}

/// What `Scanner::match_fingerprints` found.
#[derive(Debug, Deserialize)]
pub struct Matches {
    /// The signatures' versions ("bytecode 339 daily 28186 main 63").
    pub signatures: String,
    /// (the query's id, the signature's name)
    pub found: Vec<(String, String)>,
}

/// The longest checking every fingerprint against the signatures may take.
const MATCH_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// What to scan: one release file, downloaded to `path`.
pub struct Target<'a> {
    pub path: &'a Path,
    pub file_name: &'a str,
    pub sha256: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scanner {
    /// No scanning: test and development servers only.
    Off,
    /// The command that scans a file and prints a verdict (deploy/scanner/slop-scan).
    Local(&'static str),
}

/// The longest one file's scan may take.
const SCAN_TIMEOUT: Duration = Duration::from_secs(30 * 60);

impl Scanner {
    pub fn from_setting(value: &str, command: &str) -> anyhow::Result<Self> {
        match value.trim() {
            "" | "off" => Ok(Self::Off),
            "local" => {
                anyhow::ensure!(!command.trim().is_empty(), "SLOP_SCAN_COMMAND is empty");
                Ok(Self::Local(Box::leak(
                    command.trim().to_string().into_boxed_str(),
                )))
            }
            other => anyhow::bail!("SLOP_SCANNER={other:?}: use \"local\" or \"off\""),
        }
    }

    pub fn is_on(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub async fn scan(self, target: &Target<'_>) -> Verdict {
        match self {
            Self::Off => Verdict {
                result: ScanResult::Skipped,
                detail: Some("not scanned: scanning is off on this server".into()),
                report: None,
                fingerprints: Vec::new(),
            },
            Self::Local(command) => match run_local(command, target).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(error = %e, sha256 = target.sha256, "scan failed");
                    Verdict {
                        result: ScanResult::OnHold,
                        detail: Some(format!(
                            "the scanner couldn't finish ({e}); running the scans again usually fixes this"
                        )),
                        report: None,
                        fingerprints: Vec::new(),
                    }
                }
            },
        }
    }

    /// Checks fingerprints (JSON lines written to `queries` by the caller) against today's
    /// ClamAV signatures: `slop-scan --match-hashes` (deploy/scanner/hashes.py). None with the
    /// scanner off.
    pub async fn match_fingerprints<F, Fut>(self, write: F) -> anyhow::Result<Option<Matches>>
    where
        F: FnOnce(tokio::process::ChildStdin) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<()>>,
    {
        let Self::Local(command) = self else {
            return Ok(None);
        };
        let mut child = tokio::process::Command::new(command)
            .arg("--match-hashes")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().expect("piped");
        let run = async {
            // The scanner reads every line before it answers, so writing first can't block.
            write(stdin).await?;
            Ok::<_, anyhow::Error>(child.wait_with_output().await?)
        };
        let out = tokio::time::timeout(MATCH_TIMEOUT, run)
            .await
            .map_err(|_| anyhow::anyhow!("checking fingerprints took too long"))??;
        anyhow::ensure!(
            out.status.success(),
            "exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("")
        );
        let report: Value = serde_json::from_slice(&out.stdout)?;
        let signatures = report["engines"]["clamav"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("no signature versions in the answer"))?
            .to_string();
        let found = report["matches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some((
                    m["id"].as_str()?.to_string(),
                    m["name"].as_str()?.to_string(),
                ))
            })
            .collect();
        Ok(Some(Matches { signatures, found }))
    }

    /// Whether a file with this result may be approved (unless a reviewer cleared it).
    pub fn approvable(self, result: ScanResult) -> bool {
        match result {
            ScanResult::Passed | ScanResult::Note => true,
            // Only a server that can't scan accepts unscanned files.
            ScanResult::Skipped => !self.is_on(),
            ScanResult::OnHold | ScanResult::Rejected | ScanResult::Blocked => false,
        }
    }
}

async fn run_local(command: &str, target: &Target<'_>) -> anyhow::Result<Verdict> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.arg(target.path)
        .arg(target.file_name)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(SCAN_TIMEOUT, cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("it took too long"))??;
    anyhow::ensure!(
        out.status.success(),
        "exit {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .last()
            .unwrap_or("")
    );
    let mut report: Value = serde_json::from_slice(&out.stdout)?;
    let prints = fingerprints(report.as_object_mut().and_then(|o| o.remove("hashes")));
    let result = report["result"]
        .as_str()
        .and_then(ScanResult::parse)
        // The scanner never decides on its own that something is malicious: it holds the file,
        // and SLAPorSLOP staff judge it.
        .map(|r| match r {
            ScanResult::Passed | ScanResult::Note => r,
            _ => ScanResult::OnHold,
        })
        .ok_or_else(|| anyhow::anyhow!("no result in the report"))?;
    Ok(Verdict {
        result,
        detail: report["detail"].as_str().map(String::from),
        report: Some(report),
        fingerprints: prints,
    })
}

/// Microsoft Defender's check of a Windows build, as reported by Slop Build or Slop Check.
#[derive(Clone, Debug, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DefenderCheck {
    /// The hash of the file that was checked (the release file, as SLAPorSLOP downloads it).
    pub sha256: String,
    /// Defender's engine and definitions versions, and when the definitions were updated.
    pub engine: String,
    pub signatures: String,
    #[serde(default)]
    pub signatures_updated: Option<String>,
    /// What Defender found, if anything.
    #[serde(default)]
    pub detections: Vec<DefenderDetection>,
    /// The main program's Authenticode signer (e.g. "SignPath Foundation"), if it's signed.
    #[serde(default)]
    pub signed_by: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DefenderDetection {
    pub file: String,
    pub threat: String,
}

impl DefenderCheck {
    pub fn validate(&self) -> Result<(), &'static str> {
        let hex = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
        if !hex(&self.sha256) {
            return Err("sha256 must be the file's hex SHA-256");
        }
        let short = |s: &str, n| s.chars().count() <= n;
        if !short(&self.engine, 100)
            || !short(&self.signatures, 100)
            || self.detections.len() > 100
            || self
                .detections
                .iter()
                .any(|d| !short(&d.file, 500) || !short(&d.threat, 200))
            || self.signed_by.as_deref().is_some_and(|s| !short(s, 200))
        {
            return Err("too long");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanners_from_settings() {
        assert_eq!(Scanner::from_setting("off", "").unwrap(), Scanner::Off);
        assert_eq!(Scanner::from_setting("", "x").unwrap(), Scanner::Off);
        assert!(matches!(
            Scanner::from_setting("local", "/bin/slop-scan").unwrap(),
            Scanner::Local(_)
        ));
        assert!(Scanner::from_setting("local", " ").is_err());
        assert!(Scanner::from_setting("clamav", "x").is_err());
        assert!(Scanner::Off.approvable(ScanResult::Skipped));
        let on = Scanner::from_setting("local", "x").unwrap();
        assert!(
            !on.approvable(ScanResult::Skipped),
            "a scanning server needs a scan"
        );
        assert!(!Scanner::Off.approvable(ScanResult::Rejected));
        assert!(!Scanner::Off.approvable(ScanResult::OnHold));
    }

    #[tokio::test]
    async fn a_local_scanner_runs_the_command_and_reads_its_verdict() {
        let dir = std::env::temp_dir().join(format!("slop-scan-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("scan.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\ncase \"$2\" in *bad*) echo '{\"result\":\"on_hold\",\"detail\":\"Flagged\"}';; *evil*) echo '{\"result\":\"blocked\"}';; *) echo '{\"result\":\"passed\",\"detail\":\"clean\"}';; esac\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let scanner = Scanner::from_setting("local", script.to_str().unwrap()).unwrap();
        let file = dir.join("f");
        std::fs::write(&file, b"x").unwrap();
        let t = |name: &'static str| Target {
            path: &file,
            file_name: name,
            sha256: "00",
        };
        assert_eq!(
            scanner.scan(&t("good.zip")).await.result,
            ScanResult::Passed
        );
        let held = scanner.scan(&t("bad.zip")).await;
        assert_eq!(
            (held.result, held.detail.as_deref()),
            (ScanResult::OnHold, Some("Flagged"))
        );
        // The scanner can't decide on its own that something is malicious.
        assert_eq!(
            scanner.scan(&t("evil.zip")).await.result,
            ScanResult::OnHold
        );
        let broken = Scanner::from_setting("local", "/nonexistent/scan").unwrap();
        assert_eq!(broken.scan(&t("good.zip")).await.result, ScanResult::OnHold);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
