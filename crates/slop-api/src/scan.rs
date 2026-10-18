//! The scan step of the release pipeline (DESIGN §10, SECURITY §5.2).
//!
//! Not built yet. The plan is a separately hosted scan service (in its own cloud account, with
//! no route back to the core) that returns verdicts signed with its own key. Until then the
//! server runs with scanning off: every file is recorded as `skipped`, reviewers and makers see
//! that, and nothing in the UI says a file was scanned.

use serde::Serialize;

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
}

/// What to scan: one release file, by where it lives and its hash.
pub struct Target<'a> {
    pub url: &'a str,
    pub sha256: &'a str,
    pub repo_url: &'a str,
    pub commit: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scanner {
    /// No scanning: test and development servers only.
    Off,
}

impl Scanner {
    pub fn from_setting(value: &str) -> anyhow::Result<Self> {
        match value.trim() {
            "" | "off" => Ok(Self::Off),
            other => anyhow::bail!("SLOP_SCANNER={other:?}: only \"off\" exists so far"),
        }
    }

    pub fn is_on(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub async fn scan(self, _target: &Target<'_>) -> Verdict {
        match self {
            Self::Off => Verdict {
                result: ScanResult::Skipped,
                detail: Some("not scanned: scanning is off on this server".into()),
            },
        }
    }

    /// Whether a file with this result may be approved.
    pub fn approvable(self, result: ScanResult) -> bool {
        match result {
            ScanResult::Passed | ScanResult::Note => true,
            // Only a server that can't scan accepts unscanned files.
            ScanResult::Skipped => !self.is_on(),
            ScanResult::OnHold | ScanResult::Rejected | ScanResult::Blocked => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_off_exists_and_it_skips() {
        assert_eq!(Scanner::from_setting("off").unwrap(), Scanner::Off);
        assert_eq!(Scanner::from_setting("").unwrap(), Scanner::Off);
        assert!(Scanner::from_setting("clamav").is_err());
        assert!(Scanner::Off.approvable(ScanResult::Skipped));
        assert!(!Scanner::Off.approvable(ScanResult::Rejected));
        assert!(!Scanner::Off.approvable(ScanResult::OnHold));
    }
}
