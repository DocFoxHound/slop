// SPDX-License-Identifier: AGPL-3.0-or-later
//! A checked-out source tree, read like a repository archive: the same files, in the same order
//! (Git's: by path, byte by byte, as `git archive` and the hosts' archives list them), with the
//! same limits. `.git` is left out (archives don't have
//! it); symbolic links, devices and anything else that isn't a plain file or folder are skipped,
//! so nothing outside the tree is ever read.

use std::{
    ffi::OsString,
    fs::{self, File, FileType},
    io::{self, Read},
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::{Entry, RawScan, scan_entries};

/// Reads a checked-out repository at `root`.
pub fn scan_dir(root: &Path) -> anyhow::Result<RawScan> {
    scan_entries(Walk::new(root)?)
}

type Listing = std::vec::IntoIter<(OsString, PathBuf, FileType)>;

/// The files under a folder, depth first, in Git's order.
struct Walk {
    /// Folders being read: their path inside the tree, and what's left of them.
    stack: Vec<(String, Listing)>,
}

impl Walk {
    fn new(root: &Path) -> anyhow::Result<Self> {
        let listing = listing(root).with_context(|| format!("reading {}", root.display()))?;
        Ok(Self {
            stack: vec![(String::new(), listing)],
        })
    }
}

fn listing(dir: &Path) -> io::Result<Listing> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        // The entry's own type: a symbolic link stays one, never what it points at.
        out.push((entry.file_name(), entry.path(), entry.file_type()?));
    }
    // Git sorts a folder's entries by name with a `/` after folders' names, which keeps whole
    // paths in byte order (`a.txt` before `a/b`, `a/b` before `a0`).
    out.sort_by_cached_key(|(name, _, kind)| {
        let mut key = name.as_encoded_bytes().to_vec();
        if kind.is_dir() {
            key.push(b'/');
        }
        key
    });
    Ok(out.into_iter())
}

impl Iterator for Walk {
    type Item = anyhow::Result<Entry<LazyFile>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (prefix, listing) = self.stack.last_mut()?;
            let Some((name, path, kind)) = listing.next() else {
                self.stack.pop();
                continue;
            };
            if name == ".git" {
                continue;
            }
            let name = name.to_string_lossy();
            let rel = if prefix.is_empty() {
                name.into_owned()
            } else {
                format!("{prefix}/{name}")
            };
            if kind.is_dir() {
                match listing_of(&path, &rel) {
                    Ok(inner) => self.stack.push((rel, inner)),
                    Err(err) => return Some(Err(err)),
                }
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let size = match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_file() => meta.len(),
                Ok(_) => continue,
                Err(err) => {
                    return Some(Err(
                        anyhow::Error::from(err).context(format!("reading {rel}"))
                    ));
                }
            };
            return Some(Ok(Entry {
                path: rel,
                size,
                reader: LazyFile { path, file: None },
            }));
        }
    }
}

fn listing_of(path: &Path, rel: &str) -> anyhow::Result<Listing> {
    listing(path).with_context(|| format!("reading {rel}"))
}

/// A file opened only when it's read: most files are only counted.
pub struct LazyFile {
    path: PathBuf,
    file: Option<File>,
}

impl Read for LazyFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let file = match &mut self.file {
            Some(file) => file,
            None => self.file.insert(open_plain(&self.path)?),
        };
        file.read(buf)
    }
}

/// Opens a plain file, refusing a symbolic link or anything else put in its place since the
/// folder was listed (and never waiting on a pipe).
fn open_plain(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other(format!(
            "{} isn't a plain file any more",
            path.display()
        )));
    }
    Ok(file)
}
