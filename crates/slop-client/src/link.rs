// SPDX-License-Identifier: AGPL-3.0-or-later
//! Slop Link (DESIGN §9): how a running game asks the Slop app for mods. When a server or lobby
//! needs mods, the game names them (by Slop address, and version if it likes); the Slop app asks
//! the player, downloads and checks each one against Slop's signature, adds them, and tells the
//! game where they are. Servers only ever name mods; their files always come from Slop.
//!
//! It's a pair of folders, so it works the same everywhere, sandbox mode included (an
//! AppContainer can't reach the loopback interface, and a sandbox with the internet blocked has
//! no network at all). The game finds them in `SLOP_LINK`, a folder for this run only:
//!
//! - it writes a request as `requests/<name>.json` (write `<name>.tmp`, then rename it), with
//!   `<name>` made of letters, digits, `-` and `_`: `{"mods": [{"slug": "dragons", "version":
//!   "1.2.0"}], "for": "Castle Rock server"}`;
//! - Slop takes it, and writes `answers/<name>.json`: `{"status": "waiting"}`, then `"done"`
//!   (with `mods`, each `added`, `already_added`, `version_differs` or `unavailable`, and
//!   `folders`), `"declined"` or `"failed"` (with `message`).
//!
//! In sandbox mode the game can write only `requests/`; `answers/` is read-only to it. Slop
//! reads requests as plain files of at most 16 KiB and never follows links in either folder.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::json;

const MAX_REQUEST: u64 = 16 * 1024;
const MAX_MODS: usize = 50;

/// A mod the game asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wanted {
    pub slug: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// A request for the player to confirm.
#[derive(Clone, Debug, Serialize)]
pub struct ModRequest {
    pub id: String,
    /// The game that asked (its Slop address).
    pub game: String,
    /// Who wants the mods, as the game puts it ("Castle Rock server"). Untrusted text.
    pub from: Option<String>,
    pub mods: Vec<Wanted>,
}

/// How one requested mod went.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModResult {
    pub slug: String,
    /// `added`, `already_added`, `version_differs` (added, but another version than asked),
    /// or `unavailable` (not an approved mod for this game).
    pub result: &'static str,
    pub version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Waiting,
    Declined,
    Done {
        mods: Vec<ModResult>,
        /// Every mod folder the game has now, as in `SLOP_MODS`.
        folders: Vec<String>,
    },
    Failed {
        message: String,
    },
}

/// One run of a game: its link folder.
struct Run {
    game: String,
    dir: PathBuf,
}

struct Pending {
    request: ModRequest,
    /// The run's answers folder and the request's name.
    answers: PathBuf,
    name: String,
    waiting: bool,
}

type OnRequest = Box<dyn Fn(&ModRequest) + Send + Sync>;

struct Inner {
    root: PathBuf,
    runs: Mutex<HashMap<String, Run>>,
    requests: Mutex<HashMap<String, Pending>>,
    on_request: OnRequest,
}

/// The Slop app's end. Cloning shares it.
#[derive(Clone)]
pub struct Link(Arc<Inner>);

fn valid_slug(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_name(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Plain text a person will read: no control characters, at most `max` characters.
fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Writes an answer without following links: a new file beside it, renamed over it.
fn write_answer(answers: &Path, name: &str, outcome: &Outcome) -> io::Result<()> {
    if fs::symlink_metadata(answers)?.file_type().is_symlink() {
        return Err(io::Error::other("the answers folder is a link"));
    }
    let tmp = answers.join(format!(".{name}-{}.tmp", crate::random_token()));
    fs::write(&tmp, serde_json::to_vec(outcome).expect("serialises"))?;
    fs::rename(&tmp, answers.join(format!("{name}.json")))
}

impl Link {
    /// Starts watching run folders under `root` (emptied first). `on_request` is called for
    /// each request the player needs to answer (`finish` it later).
    pub fn start(
        root: PathBuf,
        on_request: impl Fn(&ModRequest) + Send + Sync + 'static,
    ) -> io::Result<Link> {
        Self::start_every(root, Duration::from_millis(500), on_request)
    }

    pub fn start_every(
        root: PathBuf,
        every: Duration,
        on_request: impl Fn(&ModRequest) + Send + Sync + 'static,
    ) -> io::Result<Link> {
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        let link = Link(Arc::new(Inner {
            root,
            runs: Mutex::default(),
            requests: Mutex::default(),
            on_request: Box::new(on_request),
        }));
        let watcher = Arc::downgrade(&link.0);
        std::thread::spawn(move || {
            while let Some(inner) = watcher.upgrade() {
                Link(inner).look();
                std::thread::sleep(every);
            }
        });
        Ok(link)
    }

    /// A new link folder for one run of `game`, with `requests/` and `answers/` in it (the
    /// sandbox makes only `requests/` writable). `close` it when the game exits.
    pub fn open(&self, game: &str) -> io::Result<PathBuf> {
        let run = crate::random_token();
        let dir = self.0.root.join(&run);
        fs::create_dir_all(dir.join("requests"))?;
        fs::create_dir_all(dir.join("answers"))?;
        self.0.runs.lock().unwrap().insert(
            run,
            Run {
                game: game.to_string(),
                dir: dir.clone(),
            },
        );
        Ok(dir)
    }

    /// Stops a run's link and removes its folder.
    pub fn close(&self, dir: &Path) {
        let mut runs = self.0.runs.lock().unwrap();
        runs.retain(|_, r| r.dir != dir);
        drop(runs);
        self.0
            .requests
            .lock()
            .unwrap()
            .retain(|_, p| !p.answers.starts_with(dir));
        let _ = fs::remove_dir_all(dir);
    }

    /// A request still waiting for the player.
    pub fn waiting(&self, id: &str) -> Option<ModRequest> {
        self.0
            .requests
            .lock()
            .unwrap()
            .get(id)
            .filter(|p| p.waiting)
            .map(|p| p.request.clone())
    }

    /// Records the player's answer, for the game to read.
    pub fn finish(&self, id: &str, outcome: Outcome) {
        let mut requests = self.0.requests.lock().unwrap();
        if let Some(p) = requests.get_mut(id) {
            p.waiting = false;
            if let Err(e) = write_answer(&p.answers, &p.name, &outcome) {
                tracing::warn!(error = %e, "couldn't answer a game's mod request");
            }
        }
    }

    /// Takes new requests from every run's folder.
    fn look(&self) {
        let runs: Vec<(String, PathBuf)> = self
            .0
            .runs
            .lock()
            .unwrap()
            .values()
            .map(|r| (r.game.clone(), r.dir.clone()))
            .collect();
        for (game, dir) in runs {
            let Ok(entries) = fs::read_dir(dir.join("requests")) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(".json"))
                    .map(str::to_string)
                else {
                    continue;
                };
                // Plain files only, small, read once.
                let Ok(meta) = fs::symlink_metadata(&path) else {
                    continue;
                };
                let body = (meta.is_file() && meta.len() <= MAX_REQUEST)
                    .then(|| read_plain(&path, &meta))
                    .flatten();
                let _ = fs::remove_file(&path);
                if !valid_name(&name) {
                    continue;
                }
                let answers = dir.join("answers");
                match body.ok_or_else(|| "not a plain file of at most 16 KiB".to_string()) {
                    Ok(body) => self.take(&game, &answers, &name, &body),
                    Err(message) => {
                        let _ = write_answer(&answers, &name, &Outcome::Failed { message });
                    }
                }
            }
        }
    }

    fn take(&self, game: &str, answers: &Path, name: &str, body: &[u8]) {
        let fail = |message: String| {
            let _ = write_answer(answers, name, &Outcome::Failed { message });
        };
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Body {
            mods: Vec<Wanted>,
            #[serde(default, rename = "for")]
            from: Option<String>,
        }
        let b: Body = match serde_json::from_slice(body) {
            Ok(b) => b,
            Err(e) => return fail(format!("not a request: {e}")),
        };
        if b.mods.is_empty() || b.mods.len() > MAX_MODS {
            return fail(format!("name 1 to {MAX_MODS} mods"));
        }
        let mut mods: Vec<Wanted> = Vec::new();
        for m in b.mods {
            if !valid_slug(&m.slug) {
                return fail(format!("{:?} isn't a Slop address", m.slug));
            }
            if !mods.iter().any(|x| x.slug == m.slug) {
                mods.push(Wanted {
                    slug: m.slug,
                    version: m.version.map(|v| clean(&v, 40)).filter(|v| !v.is_empty()),
                });
            }
        }
        let mut requests = self.0.requests.lock().unwrap();
        if requests
            .values()
            .any(|p| p.request.game == game && p.waiting)
        {
            drop(requests);
            return fail("a request is already waiting for the player".into());
        }
        let request = ModRequest {
            id: crate::random_token(),
            game: game.to_string(),
            from: b.from.map(|f| clean(&f, 100)).filter(|f| !f.is_empty()),
            mods,
        };
        let _ = write_answer(answers, name, &Outcome::Waiting);
        requests.insert(
            request.id.clone(),
            Pending {
                request: request.clone(),
                answers: answers.to_path_buf(),
                name: name.to_string(),
                waiting: true,
            },
        );
        drop(requests);
        (self.0.on_request)(&request);
    }
}

/// The answer a game reads, for tests and tools.
pub fn read_answer(link: &Path, name: &str) -> Option<serde_json::Value> {
    let bytes = fs::read(link.join("answers").join(format!("{name}.json"))).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// A request as a game writes it, for tests and tools.
pub fn write_request(link: &Path, name: &str, mods: &[&str], from: &str) -> io::Result<()> {
    let body = json!({ "mods": mods.iter().map(|m| json!({ "slug": m })).collect::<Vec<_>>(), "for": from });
    let tmp = link.join("requests").join(format!("{name}.tmp"));
    fs::write(&tmp, body.to_string())?;
    fs::rename(&tmp, link.join("requests").join(format!("{name}.json")))
}

/// Reads a request the game left, at most `MAX_REQUEST` bytes, and only if the file opened is
/// the plain file that was checked: a link swapped in between the check and the read (to make
/// Slop read some other file) is refused.
fn read_plain(path: &Path, checked: &fs::Metadata) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = fs::File::open(path).ok()?;
    let opened = file.metadata().ok()?;
    if !opened.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (opened.dev(), opened.ino()) != (checked.dev(), checked.ino()) {
            return None;
        }
    }
    #[cfg(not(unix))]
    let _ = checked;
    let mut body = Vec::new();
    file.take(MAX_REQUEST + 1).read_to_end(&mut body).ok()?;
    (body.len() as u64 <= MAX_REQUEST).then_some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Up to 10 s: a busy CI machine can be slow to schedule the link's thread.
    fn wait_for(mut done: impl FnMut() -> bool) {
        for _ in 0..1000 {
            if done() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out");
    }

    /// A request as games are told to write one: `.tmp`, then renamed. Written straight to
    /// `.json`, the link could read it before its contents land, and answer "not a request".
    fn put(run: &Path, name: &str, body: &[u8]) {
        let tmp = run.join("requests").join(format!("{name}.tmp"));
        fs::write(&tmp, body).unwrap();
        fs::rename(&tmp, run.join("requests").join(format!("{name}.json"))).unwrap();
    }

    #[test]
    fn a_game_asks_and_reads_the_players_answer() {
        let root = std::env::temp_dir().join(format!("slop-link-{}", rand::random::<u64>()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let link = Link::start_every(root.clone(), Duration::from_millis(10), move |r| {
            s2.lock().unwrap().push(r.clone())
        })
        .unwrap();
        let run = link.open("castle").unwrap();

        // Bad requests get a "failed" answer and never reach the player.
        put(&run, "bad", br#"{"mods": [{"slug": "../x"}]}"#);
        wait_for(|| read_answer(&run, "bad").is_some());
        assert_eq!(read_answer(&run, "bad").unwrap()["status"], "failed");
        fs::write(run.join("requests/half.tmp"), b"{").unwrap();

        // A good one waits for the player; a second waits its turn.
        put(
            &run,
            "r1",
            br#"{"mods": [{"slug": "dragons", "version": "1.2.0"}, {"slug": "dragons"}], "for": "Rock\u0007 server"}"#,
        );
        wait_for(|| !seen.lock().unwrap().is_empty());
        assert_eq!(read_answer(&run, "r1").unwrap()["status"], "waiting");
        let asked = seen.lock().unwrap()[0].clone();
        assert_eq!(asked.game, "castle");
        assert_eq!(asked.from.as_deref(), Some("Rock server"));
        assert_eq!(asked.mods.len(), 1, "named once");
        write_request(&run, "r2", &["dragons"], "again").unwrap();
        wait_for(|| read_answer(&run, "r2").is_some());
        assert_eq!(read_answer(&run, "r2").unwrap()["status"], "failed");
        assert!(
            run.join("requests/half.tmp").exists(),
            "unfinished writes are left alone"
        );

        assert!(link.waiting(&asked.id).is_some());
        link.finish(
            &asked.id,
            Outcome::Done {
                mods: vec![ModResult {
                    slug: "dragons".into(),
                    result: "added",
                    version: Some("1.2.0".into()),
                }],
                folders: vec!["/x/mods/dragons/files".into()],
            },
        );
        let answer = read_answer(&run, "r1").unwrap();
        assert_eq!(answer["status"], "done");
        assert_eq!(answer["mods"][0]["result"], "added");
        assert!(link.waiting(&asked.id).is_none());

        // Closed when the game exits.
        link.close(&run);
        assert!(!run.exists());
        drop(link);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn links_are_never_followed() {
        let root = std::env::temp_dir().join(format!("slop-link-{}", rand::random::<u64>()));
        let link = Link::start_every(root.clone(), Duration::from_millis(10), |_| {}).unwrap();
        let run = link.open("castle").unwrap();
        let secret = root.join("secret.txt");
        fs::write(&secret, b"keep").unwrap();
        std::os::unix::fs::symlink(&secret, run.join("requests/peek.json")).unwrap();
        wait_for(|| read_answer(&run, "peek").is_some());
        assert_eq!(read_answer(&run, "peek").unwrap()["status"], "failed");
        assert_eq!(fs::read(&secret).unwrap(), b"keep");
        // An answer over a link replaces the link, not what it points to.
        std::os::unix::fs::symlink(&secret, run.join("answers/x.json")).unwrap();
        write_answer(&run.join("answers"), "x", &Outcome::Declined).unwrap();
        assert_eq!(fs::read(&secret).unwrap(), b"keep");
        link.close(&run);
        let _ = fs::remove_dir_all(root);
    }
}
