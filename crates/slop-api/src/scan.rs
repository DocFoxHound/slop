//! The scan step of the release pipeline (DESIGN §10, SECURITY §5.2).
//!
//! Two parts, both free:
//!
//! - **Slop's scanner** (`SLOP_SCANNER=local`): every release file is downloaded once and handed
//!   to `SLOP_SCAN_COMMAND` (`deploy/scanner/slop-scan`), which scans it in a container with no
//!   network: ClamAV, YARA with YARA-Forge's core rules, and Mandiant's capa. Static only;
//!   nothing in a build is run. It prints a verdict as JSON.
//! - **Microsoft Defender** for Windows builds, which Slop can't run on Linux: Slop Build (and
//!   Slop Check, for builds made elsewhere) scans them on GitHub's Windows runners in the
//!   maker's own repository, with today's definitions, and reports to Slop with the run's OIDC
//!   token (slop_build.rs). That catches what would stop players' Windows from running the app
//!   before it's published.
//!
//! Findings put a file **on hold** until a person looks: scanners have false positives, so a
//! reviewer can clear one with a reason (admin.rs), which is audit-logged. With the scanner off
//! (test and development servers) files are `skipped`, and nothing in the UI claims a scan.

use std::{path::Path, time::Duration};

use serde::Serialize;
use serde_json::Value;

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
}

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
            },
            Self::Local(command) => match run_local(command, target).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(error = %e, sha256 = target.sha256, "scan failed");
                    Verdict {
                        result: ScanResult::OnHold,
                        detail: Some(format!(
                            "the scanner couldn't finish ({e}); a person checks it, or it's scanned again"
                        )),
                        report: None,
                    }
                }
            },
        }
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
    let report: Value = serde_json::from_slice(&out.stdout)?;
    let result = report["result"]
        .as_str()
        .and_then(ScanResult::parse)
        // The scanner never decides on its own that something is malicious: a person does.
        .map(|r| match r {
            ScanResult::Passed | ScanResult::Note => r,
            _ => ScanResult::OnHold,
        })
        .ok_or_else(|| anyhow::anyhow!("no result in the report"))?;
    Ok(Verdict {
        result,
        detail: report["detail"].as_str().map(String::from),
        report: Some(report),
    })
}

/// Microsoft Defender's check of a Windows build, as reported by Slop Build or Slop Check.
#[derive(Clone, Debug, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DefenderCheck {
    /// The hash of the file that was checked (the release file, as Slop downloads it).
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
