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
    /// SLAPorSLOP's scanning server (`SLOP_SCANNER=remote`, scan_server.rs): a separate machine
    /// that runs the same command, so nothing a maker sent is opened where the website and the
    /// database are. Source checks go there too (`scan_source`).
    Remote(&'static Remote),
}

/// The longest one file's scan may take.
const SCAN_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// The scanning server couldn't be reached, was busy for too long, or turned the request away
/// (a wrong token). Nothing is wrong with the file: the release waits and is tried again.
#[derive(Debug)]
pub struct Unavailable(pub String);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SLAPorSLOP's scanning server {}", self.0)
    }
}

impl std::error::Error for Unavailable {}

/// Since when the scanning server hasn't answered (None while it does), for the admins' alerts
/// (ops.rs).
static REMOTE_DOWN_SINCE: std::sync::Mutex<Option<time::OffsetDateTime>> =
    std::sync::Mutex::new(None);

pub fn remote_down_since() -> Option<time::OffsetDateTime> {
    *REMOTE_DOWN_SINCE.lock().unwrap_or_else(|e| e.into_inner())
}

fn remote_answered(up: bool) {
    let mut since = REMOTE_DOWN_SINCE.lock().unwrap_or_else(|e| e.into_inner());
    *since = match (up, *since) {
        (true, _) => None,
        (false, None) => Some(time::OffsetDateTime::now_utc()),
        (false, kept) => kept,
    };
}

/// The way to SLAPorSLOP's scanning server: its address, the token both servers share, and how
/// long to wait between tries when it doesn't answer.
pub struct Remote {
    base: reqwest::Url,
    token: String,
    http: reqwest::Client,
    pauses: &'static [u64],
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the token.
        f.debug_struct("Remote")
            .field("base", &self.base.as_str())
            .finish()
    }
}

impl PartialEq for Remote {
    fn eq(&self, other: &Self) -> bool {
        self.base == other.base
    }
}

impl Eq for Remote {}

/// Seconds between tries of one job, when the scanning server doesn't answer (a restart or a
/// deploy there takes under a minute).
const REMOTE_PAUSES: &[u64] = &[5, 30, 120];
/// How often a job's progress is asked for.
const POLL: Duration = Duration::from_secs(2);
/// The longest a scan may take on the scanning server, waiting in its queue included. Under the
/// release fetcher's lease (releases.rs), so a long queue makes the release wait, not run twice.
const REMOTE_SCAN_DEADLINE: Duration = Duration::from_secs(35 * 60);

/// How a job on the scanning server ended, when it didn't give its answer.
enum JobError {
    /// Try again later.
    Unavailable(String),
    /// The scanner itself failed on it, as a local scanner can.
    Failed(String),
}

impl Remote {
    fn job_url(&self, path: &str) -> reqwest::Url {
        let mut url = self.base.clone();
        url.set_path(&format!("{}{path}", self.base.path().trim_end_matches('/')));
        url
    }

    /// Sends `body` to the scanning server as a job of `kind` and waits for its answer, trying
    /// again when the server doesn't answer.
    async fn job(
        &self,
        kind: &str,
        name: Option<&str>,
        sha256: Option<&str>,
        body: &Path,
        deadline: Duration,
    ) -> Result<Value, JobError> {
        let until = tokio::time::Instant::now() + deadline;
        let mut why = self.once(kind, name, sha256, body, until).await;
        for &pause in self.pauses {
            match why {
                Err(JobError::Unavailable(_)) => {}
                _ => break,
            }
            if tokio::time::Instant::now() + Duration::from_secs(pause) >= until {
                break;
            }
            tokio::time::sleep(Duration::from_secs(pause)).await;
            why = self.once(kind, name, sha256, body, until).await;
        }
        remote_answered(!matches!(why, Err(JobError::Unavailable(_))));
        why
    }

    async fn once(
        &self,
        kind: &str,
        name: Option<&str>,
        sha256: Option<&str>,
        body: &Path,
        until: tokio::time::Instant,
    ) -> Result<Value, JobError> {
        let unavailable = |e: reqwest::Error| JobError::Unavailable(e.without_url().to_string());
        let file = tokio::fs::File::open(body)
            .await
            .map_err(|e| JobError::Failed(format!("reading the file to send: {e}")))?;
        let mut url = self.job_url("/v1/jobs");
        url.query_pairs_mut().append_pair("kind", kind);
        if let Some(name) = name {
            url.query_pairs_mut().append_pair("name", name);
        }
        let mut req =
            self.http
                .post(url)
                .bearer_auth(&self.token)
                .body(reqwest::Body::wrap_stream(
                    tokio_util::io::ReaderStream::new(file),
                ));
        if let Some(sha256) = sha256 {
            req = req.header("x-slop-sha256", sha256);
        }
        let res = req.send().await.map_err(unavailable)?;
        let status = res.status();
        if matches!(status.as_u16(), 400 | 413) {
            let text = res.text().await.unwrap_or_default();
            return Err(JobError::Failed(format!(
                "it refused the file ({status}): {text}"
            )));
        }
        if !status.is_success() {
            return Err(JobError::Unavailable(format!("answered {status}")));
        }
        let id = res.json::<Value>().await.map_err(unavailable)?["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| JobError::Unavailable("gave no job id".into()))?;
        let progress = self.job_url(&format!("/v1/jobs/{id}"));
        loop {
            if tokio::time::Instant::now() >= until {
                return Err(JobError::Unavailable(
                    "was busy for too long; the scan will be tried again".into(),
                ));
            }
            tokio::time::sleep(POLL).await;
            let res = self
                .http
                .get(progress.clone())
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(unavailable)?;
            if res.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(JobError::Unavailable(
                    "lost the scan (it restarted); it will be tried again".into(),
                ));
            }
            if !res.status().is_success() {
                return Err(JobError::Unavailable(format!("answered {}", res.status())));
            }
            let mut answer: Value = res.json().await.map_err(unavailable)?;
            match answer["state"].as_str() {
                Some("queued" | "running") => continue,
                Some("done") => return Ok(answer["output"].take()),
                Some("failed") => {
                    return Err(JobError::Failed(
                        answer["error"].as_str().unwrap_or("it failed").to_string(),
                    ));
                }
                _ => return Err(JobError::Unavailable("gave an answer it shouldn't".into())),
            }
        }
    }
}

/// The verdict when the scanner itself failed on a file: on hold, for staff (ops.rs alerts them).
fn couldnt_finish(why: impl std::fmt::Display, sha256: &str) -> Verdict {
    tracing::error!(error = %why, sha256, "scan failed");
    Verdict {
        result: ScanResult::OnHold,
        detail: Some(format!(
            "the scanner couldn't finish ({why}); running the scans again usually fixes this"
        )),
        report: None,
        fingerprints: Vec::new(),
    }
}

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
            "remote" => {
                anyhow::bail!("SLOP_SCANNER=remote needs SLOP_SCAN_URL and SLOP_SCAN_TOKEN")
            }
            other => anyhow::bail!("SLOP_SCANNER={other:?}: use \"local\", \"remote\" or \"off\""),
        }
    }

    /// The scanning server at `url` (https, or http on this machine for tests), with the token in
    /// its SLOP_SCAN_TOKEN.
    pub fn remote(url: &str, token: &str) -> anyhow::Result<Self> {
        Self::remote_with_pauses(url, token, REMOTE_PAUSES)
    }

    /// `remote`, with other pauses between tries (tests).
    pub fn remote_with_pauses(
        url: &str,
        token: &str,
        pauses: &'static [u64],
    ) -> anyhow::Result<Self> {
        let base: reqwest::Url = url.trim().parse().map_err(|_| {
            anyhow::anyhow!("SLOP_SCAN_URL isn't an address (https://scan.example/)")
        })?;
        let local = matches!(base.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        anyhow::ensure!(
            base.scheme() == "https" || (base.scheme() == "http" && local),
            "SLOP_SCAN_URL must be https"
        );
        let token = token.trim();
        anyhow::ensure!(
            token.len() >= 32,
            "SLOP_SCAN_TOKEN must be at least 32 characters (openssl rand -hex 32)"
        );
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self::Remote(Box::leak(Box::new(Remote {
            base,
            token: token.to_string(),
            http,
            pauses,
        }))))
    }

    pub fn is_on(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Scans a file. A scanning server that can't be reached gives a hold, as a scanner that
    /// failed does; callers that can wait use `try_scan` instead.
    pub async fn scan(self, target: &Target<'_>) -> Verdict {
        match self.try_scan(target).await {
            Ok(v) => v,
            Err(why) => couldnt_finish(why, target.sha256),
        }
    }

    /// Scans a file, or says the scanning server can't be reached right now.
    pub async fn try_scan(self, target: &Target<'_>) -> Result<Verdict, Unavailable> {
        match self {
            Self::Off => Ok(Verdict {
                result: ScanResult::Skipped,
                detail: Some("not scanned: scanning is off on this server".into()),
                report: None,
                fingerprints: Vec::new(),
            }),
            Self::Local(command) => Ok(match run_local(command, target).await {
                Ok(v) => v,
                Err(e) => couldnt_finish(e, target.sha256),
            }),
            Self::Remote(remote) => match remote
                .job(
                    "file",
                    Some(target.file_name),
                    Some(target.sha256),
                    target.path,
                    REMOTE_SCAN_DEADLINE,
                )
                .await
            {
                Ok(report) => {
                    Ok(verdict_of(report).unwrap_or_else(|e| couldnt_finish(e, target.sha256)))
                }
                Err(JobError::Failed(e)) => Ok(couldnt_finish(e, target.sha256)),
                Err(JobError::Unavailable(e)) => Err(Unavailable(e)),
            },
        }
    }

    /// Reads a repository archive (a .tar.gz) on the scanning server. None when this server
    /// reads source itself (source_scan.rs).
    pub async fn scan_source(
        self,
        archive: &Path,
    ) -> Option<anyhow::Result<slop_source_scan::RawScan>> {
        let Self::Remote(remote) = self else {
            return None;
        };
        Some(
            match remote
                .job("source", None, None, archive, REMOTE_SCAN_DEADLINE)
                .await
            {
                Ok(out) => serde_json::from_value::<slop_source_scan::SourceCheck>(out)
                    .map_err(anyhow::Error::from)
                    .and_then(|c| {
                        c.raw.validate().map_err(|why| anyhow::anyhow!("{why}"))?;
                        Ok(c.raw)
                    }),
                Err(JobError::Failed(e)) => Err(anyhow::anyhow!("the source check failed: {e}")),
                Err(JobError::Unavailable(e)) => Err(Unavailable(e).into()),
            },
        )
    }

    /// Checks fingerprints (JSON lines written by the caller) against today's ClamAV
    /// signatures: `slop-scan --match-hashes` (deploy/scanner/hashes.py), here or on the scanning
    /// server. None with the scanner off.
    pub async fn match_fingerprints<F, Fut>(self, write: F) -> anyhow::Result<Option<Matches>>
    where
        F: FnOnce(std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send>>) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<()>>,
    {
        let report: Value = match self {
            Self::Off => return Ok(None),
            Self::Local(command) => {
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
                    write(Box::pin(stdin)).await?;
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
                serde_json::from_slice(&out.stdout)?
            }
            Self::Remote(remote) => {
                // Written to a file first, then sent (the same prefix as re-scans' copies, so a
                // left-over one is swept, releases.rs).
                let path =
                    std::env::temp_dir().join(format!("slop-rescan-hashes-{}", Uuid::now_v7()));
                let sent = async {
                    write(Box::pin(tokio::fs::File::create(&path).await?)).await?;
                    match remote.job("hashes", None, None, &path, MATCH_TIMEOUT).await {
                        Ok(report) => Ok(report),
                        Err(JobError::Failed(e)) => Err(anyhow::anyhow!("{e}")),
                        Err(JobError::Unavailable(e)) => Err(Unavailable(e).into()),
                    }
                }
                .await;
                let _ = tokio::fs::remove_file(&path).await;
                sent?
            }
        };
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
    let file_name = std::ffi::OsStr::new(target.file_name);
    verdict_of(
        run_command(
            command,
            &[target.path.as_os_str(), file_name],
            None,
            SCAN_TIMEOUT,
        )
        .await?,
    )
}

/// Runs the scanner command with `args` (and `stdin` from a file) and reads the JSON it prints:
/// here, or on the scanning server for a job (scan_server.rs).
pub(crate) async fn run_command(
    command: &str,
    args: &[&std::ffi::OsStr],
    stdin: Option<&Path>,
    timeout: Duration,
) -> anyhow::Result<Value> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args).kill_on_drop(true);
    match stdin {
        Some(path) => cmd.stdin(std::fs::File::open(path)?),
        None => cmd.stdin(std::process::Stdio::null()),
    };
    let out = tokio::time::timeout(timeout, cmd.output())
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
    Ok(serde_json::from_slice(&out.stdout)?)
}

/// The verdict in a scanner's report (deploy/scanner/scan.py), whether SLAPorSLOP's scanner
/// wrote it here or on the maker's GitHub runner (`ci_report_problem`). The fingerprints are taken
/// out of the report and kept apart.
pub fn verdict_of(mut report: Value) -> anyhow::Result<Verdict> {
    let prints = fingerprints(report.as_object_mut().and_then(|o| o.remove("hashes")));
    let found = |key: &str| report[key].as_array().is_some_and(|a| !a.is_empty());
    let result = report["result"]
        .as_str()
        .and_then(ScanResult::parse)
        // The scanner never decides on its own that something is malicious: it holds the file,
        // and SLAPorSLOP staff judge it. Nor does a report pass with findings in it.
        .map(|r| match r {
            ScanResult::Passed | ScanResult::Note if !found("findings") => r,
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

/// How old the signatures of a scan run on a maker's runner may be: the scanner image is rebuilt
/// with new ones every six hours (.github/workflows/scanner.yml).
const CI_SIGNATURES_MAX_DAYS: i64 = 3;

/// Why a scanner report sent from a maker's GitHub runner (Slop Build or Slop Check, by SLAPorSLOP's
/// own pinned workflow and scanner image) can't be used, or None when it can. It's checked like
/// any input: its shape and sizes, and that its ClamAV signatures are recent.
pub fn ci_report_problem(report: &Value, now: time::OffsetDateTime) -> Option<&'static str> {
    let Some(o) = report.as_object() else {
        return Some("a scan report is a JSON object");
    };
    if !matches!(
        o.get("result").and_then(Value::as_str),
        Some("passed" | "note" | "on_hold")
    ) {
        return Some("a scan report's result is passed, note or on_hold");
    }
    let text = |v: Option<&Value>, n: usize| {
        v.and_then(Value::as_str)
            .is_some_and(|s| s.chars().count() <= n)
    };
    if !text(o.get("detail"), 4000) {
        return Some("a scan report needs a detail of at most 4,000 characters");
    }
    let list = |key: &str, max: usize, fields: &[(&str, usize)]| {
        o.get(key).is_none_or(|v| {
            v.as_array().is_some_and(|items| {
                items.len() <= max
                    && items
                        .iter()
                        .all(|i| fields.iter().all(|(f, n)| text(i.get(*f), *n)))
            })
        })
    };
    let finding = [("engine", 50), ("file", 1000), ("name", 300)];
    if !list("findings", 500, &finding)
        || !list("noticed", 500, &finding)
        || !list(
            "capabilities",
            2000,
            &[("file", 1000), ("capability", 300), ("namespace", 300)],
        )
    {
        return Some("a scan report's findings are too many or malformed");
    }
    let Some(engines) = o.get("engines").and_then(Value::as_object) else {
        return Some("a scan report names its engines");
    };
    if engines.len() > 10 || !engines.values().all(|v| text(Some(v), 200)) {
        return Some("a scan report's engines are malformed");
    }
    if o.get("hashes")
        .is_some_and(|h| h.as_array().is_none_or(|a| a.len() > MAX_FINGERPRINTS))
    {
        return Some("a scan report's fingerprints are malformed");
    }
    // `clamscan --version`: "ClamAV 1.4.3/28186/Mon Nov 17 06:30:00 2026".
    let clamav = engines.get("clamav").and_then(Value::as_str).unwrap_or("");
    match signatures_date(clamav) {
        Some(date) if now.date() - date > time::Duration::days(CI_SIGNATURES_MAX_DAYS) => {
            Some("the scan's ClamAV signatures are out of date")
        }
        Some(_) => None,
        None => Some("the scan doesn't say which ClamAV signatures it used"),
    }
}

/// The date of ClamAV's signatures in `clamscan --version`'s answer.
fn signatures_date(version: &str) -> Option<time::Date> {
    let stamp = version
        .rsplit('/')
        .next()?
        .split_whitespace()
        .collect::<Vec<_>>();
    // "Mon Nov 17 06:30:00 2026": month, day and year.
    let [_, month, day, _, year] = stamp.as_slice() else {
        return None;
    };
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| m == month)?;
    time::Date::from_calendar_date(
        year.parse().ok()?,
        time::Month::try_from(month as u8 + 1).ok()?,
        day.parse().ok()?,
    )
    .ok()
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
    use sha2::Digest;
    use tokio::io::AsyncWriteExt;

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

    #[test]
    fn reports_from_github_are_checked_like_any_input() {
        let now = time::macros::datetime!(2026-11-17 12:00 UTC);
        let good = serde_json::json!({
            "result": "passed", "detail": "Scanned with ClamAV, YARA and capa: nothing found.",
            "findings": [], "noticed": [], "capabilities": [],
            "engines": { "clamav": "ClamAV 1.4.3/28186/Mon Nov 16 06:30:00 2026", "yara": "yara-forge 20261115" },
            "hashes": []
        });
        assert_eq!(ci_report_problem(&good, now), None);
        let with = |key: &str, value: Value| {
            let mut r = good.clone();
            r[key] = value;
            ci_report_problem(&r, now)
        };
        assert!(with("result", "blocked".into()).is_some());
        assert!(with("detail", "x".repeat(4001).into()).is_some());
        assert!(with("findings", serde_json::json!([{ "engine": "ClamAV" }])).is_some());
        assert!(
            with("engines", serde_json::json!({})).is_some(),
            "which signatures?"
        );
        assert!(
            with(
                "engines",
                serde_json::json!({ "clamav": "ClamAV 1.4.3/28100/Thu Nov  5 06:30:00 2026" })
            )
            .is_some(),
            "signatures more than three days old"
        );
        assert_eq!(
            signatures_date("ClamAV 1.4.3/28100/Thu Nov  5 06:30:00 2026"),
            Some(time::macros::date!(2026 - 11 - 05))
        );
        // A report with findings never passes, whatever it says.
        let mut found = good.clone();
        found["findings"] =
            serde_json::json!([{ "engine": "ClamAV", "file": "a.exe", "name": "Win.Test" }]);
        assert_eq!(verdict_of(found).unwrap().result, ScanResult::OnHold);
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

    #[tokio::test]
    async fn a_scanning_server_scans_for_production() {
        let dir = std::env::temp_dir().join(format!("slop-scan-remote-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        // A repository archive with one file, read as the source check reads it.
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(5);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "game-abc/README.md", &b"hello"[..])
            .unwrap();
        let archive = archive.into_inner().unwrap().finish().unwrap();
        let expected = slop_source_scan::scan_archive_raw(&archive[..]).unwrap();
        let source = slop_source_scan::SourceCheck {
            raw: expected.clone(),
            tool: slop_source_scan::tool(),
        };
        std::fs::write(
            dir.join("source.json"),
            serde_json::to_string(&source).unwrap(),
        )
        .unwrap();
        let script = dir.join("scan.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n--match-hashes) grep -q Win.Test.query && echo '{{\"engines\":{{\"clamav\":\"bytecode 1 daily 2\"}},\"matches\":[{{\"id\":\"r/windows/a.exe\",\"name\":\"Win.Test\"}}]}}';;\n--source) cat {};;\n*) case \"$2\" in *bad*) echo '{{\"result\":\"on_hold\",\"detail\":\"Flagged\"}}';; *broken*) exit 3;; *) echo '{{\"result\":\"passed\",\"detail\":\"clean\"}}';; esac;;\nesac\n",
                dir.join("source.json").display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let token = "t".repeat(40);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = crate::scan_server::router(crate::scan_server::Settings {
            listen: addr,
            token: token.clone(),
            command: script.to_str().unwrap().to_string(),
            jobs: 2,
            max_bytes: 1000,
            dir: dir.clone(),
        });
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("http://{addr}/");
        let scanner = Scanner::remote_with_pauses(&url, &token, &[]).unwrap();

        let file = dir.join("f");
        std::fs::write(&file, b"x").unwrap();
        let x_sha = hex::encode(sha2::Sha256::digest(b"x"));
        let t = |name: &'static str, sha256: &'static str| Target {
            path: &file,
            file_name: name,
            sha256,
        };
        let x: &'static str = Box::leak(x_sha.into_boxed_str());
        assert_eq!(
            scanner.try_scan(&t("good.zip", x)).await.unwrap().result,
            ScanResult::Passed
        );
        let held = scanner.try_scan(&t("bad.zip", x)).await.unwrap();
        assert_eq!(
            (held.result, held.detail.as_deref()),
            (ScanResult::OnHold, Some("Flagged"))
        );
        assert_eq!(remote_down_since(), None);
        // The scanner failing on the file holds it, as a local one does.
        let broken = scanner.try_scan(&t("broken.zip", x)).await.unwrap();
        assert_eq!(broken.result, ScanResult::OnHold);
        assert!(
            broken
                .detail
                .unwrap()
                .starts_with("the scanner couldn't finish")
        );
        // A file changed on the way is sent again, then waits.
        assert!(scanner.try_scan(&t("good.zip", "00")).await.is_err());
        // Over the size limit: held, not retried for ever.
        let big = dir.join("big");
        std::fs::write(&big, vec![0u8; 2000]).unwrap();
        let too_big = scanner
            .try_scan(&Target {
                path: &big,
                file_name: "big.zip",
                sha256: "00",
            })
            .await
            .unwrap();
        assert_eq!(too_big.result, ScanResult::OnHold);

        // Fingerprints and source checks go there too.
        let matches = scanner
            .match_fingerprints(|mut w| async move {
                w.write_all(b"{\"id\":\"r/windows/a.exe\",\"q\":\"Win.Test.query\"}\n")
                    .await?;
                w.shutdown().await?;
                Ok(())
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(matches.signatures, "bytecode 1 daily 2");
        assert_eq!(
            matches.found,
            vec![("r/windows/a.exe".to_string(), "Win.Test".to_string())]
        );
        let raw = scanner.scan_source(&file).await.unwrap().unwrap();
        assert_eq!(raw, expected);
        assert!(Scanner::Off.scan_source(&file).await.is_none(), "read here");

        // A wrong token, or nobody answering: the file waits, and admins learn of it.
        let wrong = Scanner::remote_with_pauses(&url, &"w".repeat(40), &[]).unwrap();
        assert!(wrong.try_scan(&t("good.zip", x)).await.is_err());
        let gone = Scanner::remote_with_pauses("http://127.0.0.1:9/", &token, &[]).unwrap();
        let why = gone.try_scan(&t("good.zip", x)).await.unwrap_err();
        assert!(why.to_string().starts_with("SLAPorSLOP's scanning server"));
        assert!(remote_down_since().is_some());
        assert_eq!(
            gone.scan(&t("good.zip", x)).await.result,
            ScanResult::OnHold
        );
        scanner.try_scan(&t("good.zip", x)).await.unwrap();
        assert_eq!(remote_down_since(), None, "answering again");
        // Nothing is kept there.
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("slop-job-"))
            .collect();
        assert!(left.is_empty(), "{left:?}");

        assert!(
            Scanner::remote("http://scan.example/", &token).is_err(),
            "https only"
        );
        assert!(Scanner::remote("https://scan.example/", "short").is_err());
        assert!(
            format!("{scanner:?}").contains("127.0.0.1")
                && !format!("{scanner:?}").contains(&token)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
