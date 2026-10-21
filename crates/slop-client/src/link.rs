//! Slop Link (DESIGN §9): how a running game asks the Slop app for mods. When a server or lobby
//! needs mods, the game names them (by Slop address, and version if it likes); the Slop app asks
//! the player, downloads and checks each one against Slop's signature, adds them, and tells the
//! game where they are. Servers only ever name mods; their files always come from Slop.
//!
//! The game finds it in `SLOP_LINK`: `http://127.0.0.1:<port>/<token>/`, one token per start,
//! which stops working when the game exits. It answers on the loopback interface only, and
//! checks the `Host` header, so web pages can't reach it.
//!
//! - `POST {SLOP_LINK}mods/request` with `{"mods": [{"slug": "dragons", "version": "1.2.0"}],
//!   "for": "Castle Rock server"}` → `202 {"id": "…"}`. One request waits at a time.
//! - `GET {SLOP_LINK}mods/request/<id>` → `{"status": "waiting" | "declined" | "done" |
//!   "failed", "mods": [{"slug", "result", "version"}], "folders": [...]}`; ask every second or
//!   two until it isn't `waiting`.

use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_BODY: usize = 16 * 1024;
const MAX_HEADERS: usize = 8 * 1024;
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

struct Pending {
    request: ModRequest,
    outcome: Outcome,
}

type OnRequest = Box<dyn Fn(&ModRequest) + Send + Sync>;

struct Inner {
    port: u16,
    /// token → game
    games: Mutex<HashMap<String, String>>,
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

/// Plain text a person will read: no control characters, at most `max` characters.
fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

impl Link {
    /// Starts listening on a free loopback port. `on_request` is called for each request the
    /// player needs to answer (`finish` it later).
    pub fn start(on_request: impl Fn(&ModRequest) + Send + Sync + 'static) -> io::Result<Link> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let link = Link(Arc::new(Inner {
            port,
            games: Mutex::default(),
            requests: Mutex::default(),
            on_request: Box::new(on_request),
        }));
        let server = link.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let server = server.clone();
                std::thread::spawn(move || {
                    let _ = server.serve(stream);
                });
            }
        });
        Ok(link)
    }

    /// A new `SLOP_LINK` for one start of `game`. `close` it when the game exits.
    pub fn open(&self, game: &str) -> String {
        let token = crate::random_token();
        self.0
            .games
            .lock()
            .unwrap()
            .insert(token.clone(), game.to_string());
        format!("http://127.0.0.1:{}/{token}/", self.0.port)
    }

    /// Stops a `SLOP_LINK` from working, and forgets its requests.
    pub fn close(&self, url: &str) {
        let token = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        let game = self.0.games.lock().unwrap().remove(token);
        if let Some(game) = game {
            let still_running = self.0.games.lock().unwrap().values().any(|g| *g == game);
            if !still_running {
                self.0
                    .requests
                    .lock()
                    .unwrap()
                    .retain(|_, p| p.request.game != game);
            }
        }
    }

    /// A request still waiting for the player.
    pub fn waiting(&self, id: &str) -> Option<ModRequest> {
        self.0
            .requests
            .lock()
            .unwrap()
            .get(id)
            .filter(|p| matches!(p.outcome, Outcome::Waiting))
            .map(|p| p.request.clone())
    }

    /// Records the player's answer, for the game to read.
    pub fn finish(&self, id: &str, outcome: Outcome) {
        if let Some(p) = self.0.requests.lock().unwrap().get_mut(id) {
            p.outcome = outcome;
        }
    }

    fn serve(&self, mut stream: TcpStream) -> io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let (status, body) = match self.handle(&stream) {
            Ok(r) => r,
            Err(e) => (400, json!({ "error": e })),
        };
        let body = body.to_string();
        let reason = match status {
            200 => "OK",
            202 => "Accepted",
            404 => "Not Found",
            409 => "Conflict",
            413 => "Payload Too Large",
            _ => "Bad Request",
        };
        write!(
            stream,
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        stream.flush()
    }

    fn handle(&self, stream: &TcpStream) -> Result<(u16, Value), String> {
        let mut reader = BufReader::new(stream.take((MAX_HEADERS + MAX_BODY) as u64));
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;
        let mut parts = line.split_whitespace();
        let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let (mut length, mut host, mut read) = (0usize, String::new(), line.len());
        loop {
            let mut h = String::new();
            let n = reader.read_line(&mut h).map_err(|e| e.to_string())?;
            read += n;
            if n == 0 || read > MAX_HEADERS {
                return Err("headers too long".into());
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((name, value)) = h.split_once(':') {
                match name.trim().to_ascii_lowercase().as_str() {
                    "content-length" => length = value.trim().parse().unwrap_or(usize::MAX),
                    "host" => host = value.trim().to_string(),
                    _ => {}
                }
            }
        }
        // Only for programs on this computer that were given the address, never web pages.
        let port = self.0.port;
        if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
            return Ok((404, json!({ "error": "not found" })));
        }
        if length > MAX_BODY {
            return Ok((413, json!({ "error": "too large" })));
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).map_err(|e| e.to_string())?;

        let rest = path.strip_prefix('/').unwrap_or("");
        let (token, route) = rest.split_once('/').unwrap_or((rest, ""));
        let Some(game) = self.0.games.lock().unwrap().get(token).cloned() else {
            return Ok((404, json!({ "error": "not found" })));
        };
        match (method, route) {
            ("POST", "mods/request") => self.new_request(&game, &body),
            ("GET", r) if r.starts_with("mods/request/") => {
                let id = &r["mods/request/".len()..];
                let requests = self.0.requests.lock().unwrap();
                match requests.get(id).filter(|p| p.request.game == game) {
                    Some(p) => Ok((200, serde_json::to_value(&p.outcome).unwrap())),
                    None => Ok((404, json!({ "error": "no such request" }))),
                }
            }
            _ => Ok((404, json!({ "error": "not found" }))),
        }
    }

    fn new_request(&self, game: &str, body: &[u8]) -> Result<(u16, Value), String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Body {
            mods: Vec<Wanted>,
            #[serde(default, rename = "for")]
            from: Option<String>,
        }
        let b: Body = serde_json::from_slice(body).map_err(|e| e.to_string())?;
        if b.mods.is_empty() || b.mods.len() > MAX_MODS {
            return Err(format!("name 1 to {MAX_MODS} mods"));
        }
        let mut mods: Vec<Wanted> = Vec::new();
        for m in b.mods {
            if !valid_slug(&m.slug) {
                return Err(format!("{:?} isn't a Slop address", m.slug));
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
            .any(|p| p.request.game == game && matches!(p.outcome, Outcome::Waiting))
        {
            return Ok((
                409,
                json!({ "error": "a request is already waiting for the player" }),
            ));
        }
        // Keep the last few answers per game for the game to read.
        let mut done: Vec<String> = requests
            .iter()
            .filter(|(_, p)| p.request.game == game)
            .map(|(id, _)| id.clone())
            .collect();
        done.sort();
        for id in done.iter().rev().skip(10) {
            requests.remove(id);
        }
        let request = ModRequest {
            id: format!("{:020}-{}", now_ms(), crate::random_token()),
            game: game.to_string(),
            from: b.from.map(|f| clean(&f, 100)).filter(|f| !f.is_empty()),
            mods,
        };
        requests.insert(
            request.id.clone(),
            Pending {
                request: request.clone(),
                outcome: Outcome::Waiting,
            },
        );
        drop(requests);
        (self.0.on_request)(&request);
        Ok((202, json!({ "id": request.id })))
    }

    pub fn local_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.0.port))
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(addr: SocketAddr, host: &str, method: &str, path: &str, body: &str) -> (u16, Value) {
        let mut s = TcpStream::connect(addr).unwrap();
        write!(
            s,
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let status = out[9..12].parse().unwrap();
        let body = out.split_once("\r\n\r\n").unwrap().1;
        (status, serde_json::from_str(body).unwrap())
    }

    #[test]
    fn a_game_asks_and_reads_the_players_answer() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let link = Link::start(move |r| s2.lock().unwrap().push(r.clone())).unwrap();
        let addr = link.local_addr();
        let host = format!("127.0.0.1:{}", addr.port());
        let url = link.open("castle");
        let base = url
            .trim_start_matches(&format!("http://{host}"))
            .to_string();

        // Web pages (another Host) and wrong tokens get nothing.
        let body = r#"{"mods": [{"slug": "dragons", "version": "1.2.0"}, {"slug": "dragons"}], "for": "Rock\u0007 server"}"#;
        assert_eq!(
            call(
                addr,
                "evil.example",
                "POST",
                &format!("{base}mods/request"),
                body
            )
            .0,
            404
        );
        assert_eq!(call(addr, &host, "POST", "/nope/mods/request", body).0, 404);
        assert_eq!(
            call(
                addr,
                &host,
                "POST",
                &format!("{base}mods/request"),
                r#"{"mods": [{"slug": "../x"}]}"#
            )
            .0,
            400
        );

        let (status, res) = call(addr, &host, "POST", &format!("{base}mods/request"), body);
        assert_eq!(status, 202);
        let id = res["id"].as_str().unwrap().to_string();
        let asked = seen.lock().unwrap()[0].clone();
        assert_eq!(asked.game, "castle");
        assert_eq!(asked.from.as_deref(), Some("Rock server"));
        assert_eq!(asked.mods.len(), 1, "named once");
        assert_eq!(
            call(addr, &host, "POST", &format!("{base}mods/request"), body).0,
            409,
            "one at a time"
        );
        let status_path = format!("{base}mods/request/{id}");
        assert_eq!(
            call(addr, &host, "GET", &status_path, "").1["status"],
            "waiting"
        );

        assert!(link.waiting(&id).is_some());
        link.finish(
            &id,
            Outcome::Done {
                mods: vec![ModResult {
                    slug: "dragons".into(),
                    result: "added",
                    version: Some("1.2.0".into()),
                }],
                folders: vec!["/x/mods/dragons/files".into()],
            },
        );
        let (_, res) = call(addr, &host, "GET", &status_path, "");
        assert_eq!(res["status"], "done");
        assert_eq!(res["mods"][0]["result"], "added");

        // Closed when the game exits.
        link.close(&url);
        assert_eq!(call(addr, &host, "GET", &status_path, "").0, 404);
    }
}
