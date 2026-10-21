// SPDX-License-Identifier: AGPL-3.0-or-later
//! The last lines a game wrote, kept while it runs, for a bug report if it crashes (plan phase
//! 13). Only the tail is kept, in memory; nothing is sent unless the player chooses to, after
//! seeing it. The player's home folder is blanked out first.

use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read},
    sync::{Arc, Mutex},
};

/// Lines and characters kept: enough to see a crash, small enough to read before sending.
pub const MAX_LINES: usize = 200;
pub const MAX_CHARS: usize = 16_000;

/// The tail of a program's output, shared by the threads reading it.
#[derive(Clone, Default)]
pub struct Tail(Arc<Mutex<VecDeque<String>>>);

impl Tail {
    pub fn push(&self, line: String) {
        let mut lines = self.0.lock().unwrap();
        lines.push_back(line.chars().take(1000).collect());
        while lines.len() > MAX_LINES {
            lines.pop_front();
        }
    }

    /// Reads `from` line by line on a thread of its own until it closes (stdout, stderr).
    pub fn follow(&self, from: impl Read + Send + 'static) {
        let tail = self.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(from).lines() {
                match line {
                    Ok(l) => tail.push(l),
                    Err(_) => break,
                }
            }
        });
    }

    /// The tail as text, with the home folder blanked out, at most `MAX_CHARS` (the end kept).
    pub fn text(&self, home: Option<&str>) -> String {
        let joined = self
            .0
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let scrubbed = scrub_home(&joined, home);
        let chars: Vec<char> = scrubbed.chars().collect();
        chars[chars.len().saturating_sub(MAX_CHARS)..]
            .iter()
            .collect()
    }
}

/// Replaces the user's home folder in paths with `~`, so the name in it isn't sent.
pub fn scrub_home(text: &str, home: Option<&str>) -> String {
    let Some(home) = home
        .map(|h| h.trim_end_matches(['/', '\\']))
        .filter(|h| h.len() > 1)
    else {
        return text.to_string();
    };
    let mut out = text.replace(home, "~");
    // Windows paths also appear with forward slashes.
    if home.contains('\\') {
        out = out.replace(&home.replace('\\', "/"), "~");
    }
    out
}

/// The user's home folder, for `scrub_home`.
pub fn home_dir() -> Option<String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .filter(|h| !h.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_end_and_hides_the_home_folder() {
        let tail = Tail::default();
        for i in 0..250 {
            tail.push(format!("line {i}"));
        }
        tail.push("panic at /home/mia/games/pong/main.gd:12".into());
        let text = tail.text(Some("/home/mia/"));
        assert!(!text.contains("line 0\n") && text.contains("line 249"));
        assert!(text.ends_with("panic at ~/games/pong/main.gd:12"));
        assert_eq!(text.lines().count(), MAX_LINES);
        assert_eq!(
            scrub_home(
                r"C:\Users\Mia\AppData\x and C:/Users/Mia/y",
                Some(r"C:\Users\Mia")
            ),
            r"~\AppData\x and ~/y"
        );
        assert_eq!(scrub_home("/ is root", Some("/")), "/ is root");
    }

    #[test]
    fn follows_a_stream() {
        let tail = Tail::default();
        tail.follow(std::io::Cursor::new(b"one\ntwo\nthree\n".to_vec()));
        for _ in 0..100 {
            if tail.text(None).contains("three") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(tail.text(None), "one\ntwo\nthree");
    }
}
