//! SLAPorSLOP for Rust games: saves, mods and asking the SLAPorSLOP app for a server's mods. Needs
//! `serde_json`. Outside the SLAPorSLOP app it falls back: your own save folder, no mods, and
//! `request_mods` answers "no_link".
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Where saves, settings and logs go: SLAPorSLOP's save folder, else `fallback`.
pub fn save_dir(fallback: &Path) -> PathBuf {
    env::var_os("SLOP_SAVE_DIR")
        .filter(|d| !d.is_empty())
        .map_or_else(|| fallback.to_path_buf(), PathBuf::from)
}

/// Saves safely: a new file first, renamed over the old one.
pub fn save(fallback: &Path, name: &str, data: &[u8]) -> io::Result<()> {
    let dir = save_dir(fallback);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{name}.tmp"));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, dir.join(name))
}

/// A save, or `None` when it's missing or over 1 MiB.
pub fn load(fallback: &Path, name: &str) -> Option<Vec<u8>> {
    let path = save_dir(fallback).join(name);
    (fs::metadata(&path).ok()?.len() <= 1024 * 1024).then(|| fs::read(&path).ok())?
}

/// Every mod folder: your game's own mods folder (if you named one on your store page), then
/// `SLOP_MODS`. Each holds one mod from SLAPorSLOP.
pub fn mod_dirs(own_folder: Option<&Path>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    if let Some(own) = own_folder {
        let mut subs: Vec<PathBuf> = fs::read_dir(own)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        subs.sort();
        found.extend(subs);
    }
    let address = |p: &Path| -> Option<String> {
        let p = if p.file_name()? == "files" { p.parent()? } else { p };
        Some(p.file_name()?.to_string_lossy().into_owned())
    };
    for dir in env::split_paths(&env::var_os("SLOP_MODS").unwrap_or_default()) {
        if !dir.as_os_str().is_empty() && !found.iter().any(|f| address(f) == address(&dir)) {
            found.push(dir);
        }
    }
    found
}

/// Asks the SLAPorSLOP app for mods by address, e.g. `json!([{"slug": "dragons", "version": "1.2.0"}])`.
/// The player confirms. Returns the answer: "done" (with "mods" and "folders"), "declined",
/// "failed" (with "message"), "no_link" or "timeout". Blocks: call it off your main loop.
pub fn request_mods(mods: serde_json::Value, for_whom: &str, give_up_after: Duration) -> serde_json::Value {
    let Some(link) = env::var_os("SLOP_LINK").filter(|l| !l.is_empty()).map(PathBuf::from) else {
        return serde_json::json!({ "status": "no_link" });
    };
    let name = format!("r{}", now_ns());
    let tmp = link.join("requests").join(format!("{name}.tmp"));
    let body = serde_json::json!({ "mods": mods, "for": for_whom });
    if fs::write(&tmp, body.to_string())
        .and_then(|_| fs::rename(&tmp, link.join("requests").join(format!("{name}.json"))))
        .is_err()
    {
        return serde_json::json!({ "status": "failed", "message": "couldn't write the request" });
    }
    let answer_path = link.join("answers").join(format!("{name}.json"));
    let until = Instant::now() + give_up_after;
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(500));
        let answer = fs::read(&answer_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        if let Some(answer) = answer.filter(|a| a["status"] != "waiting") {
            return answer;
        }
    }
    serde_json::json!({ "status": "timeout" })
}

fn now_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}
