//! Save folders (DESIGN §6, SECURITY §6.2). Each app gets one, outside its installed files, so
//! updates never touch it; the app finds it in `SLOP_SAVE_DIR`. It's the same folder in normal
//! and sandbox mode, so switching loses nothing.
//!
//! That makes it the one bridge between a sandboxed session and a normal one, so before every
//! launch Slop sweeps it: links are removed, programs and libraries are moved out to a
//! quarantine folder, and execute permission is taken off. A sandboxed session can only leave
//! data behind, and before each one Slop keeps a copy of the folder so the player can go back.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339, macros::format_description};

/// Copies kept per app; the oldest goes first.
pub const KEEP_COPIES: usize = 3;
/// Bigger save folders aren't copied before a sandboxed session (it'd take too long and too
/// much disk); the player is told.
pub const MAX_COPY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// A saved copy of an app's save folder.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SaveCopy {
    /// Its folder name, a UTC timestamp.
    pub id: String,
    /// RFC 3339.
    pub taken_at: String,
    pub bytes: u64,
}

/// Whether a file starts like a program or library: ELF, Windows PE, Mach-O.
fn is_native_code(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    let Ok(mut f) = fs::File::open(path) else {
        return false;
    };
    if f.read_exact(&mut head).is_err() {
        return false;
    }
    head == *b"\x7fELF"
        || head[..2] == *b"MZ"
        || matches!(
            u32::from_be_bytes(head),
            0xfeedface | 0xfeedfacf | 0xcefaedfe | 0xcffaedfe | 0xcafebabe
        )
}

/// Sweeps a save folder before a launch. Returns the paths (relative to the folder) of the
/// programs and libraries moved into `quarantine`. `skip` is a mods folder inside it, which
/// Slop fills with approved mods (code included) and checks itself (mods_folder.rs).
pub fn sweep(dir: &Path, quarantine: &Path, skip: Option<&Path>) -> io::Result<Vec<String>> {
    let mut moved = Vec::new();
    if dir.exists() {
        sweep_in(dir, dir, quarantine, skip, &mut moved)?;
    }
    Ok(moved)
}

fn sweep_in(
    root: &Path,
    dir: &Path,
    quarantine: &Path,
    skip: Option<&Path>,
    moved: &mut Vec<String>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if skip == Some(path.as_path()) && kind.is_dir() {
            continue;
        }
        if kind.is_symlink() {
            // A link could point Slop or the app outside the folder.
            fs::remove_file(&path)?;
        } else if kind.is_dir() {
            sweep_in(root, &path, quarantine, skip, moved)?;
        } else if kind.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            if is_native_code(&path) {
                let to = quarantine.join(&rel);
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(&path, &to)
                    .or_else(|_| fs::copy(&path, &to).and_then(|_| fs::remove_file(&path)))?;
                moved.push(rel.to_string_lossy().replace('\\', "/"));
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = entry.metadata()?.permissions().mode();
                if mode & 0o111 != 0 {
                    fs::set_permissions(&path, fs::Permissions::from_mode(mode & !0o111 & 0o777))?;
                }
            }
        } else {
            // Sockets, pipes and devices have no place in saves.
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Size of a folder's plain files, not following links.
pub fn size(dir: &Path) -> io::Result<u64> {
    let mut total = 0;
    if !dir.exists() {
        return Ok(0);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            total += size(&entry.path())?;
        } else if kind.is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

/// Copies plain files and folders; links and anything else are left out.
pub fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    if !from.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

fn valid_copy_id(id: &str) -> bool {
    id.len() == 16
        && id.bytes().enumerate().all(|(i, b)| match i {
            8 => b == b'T',
            15 => b == b'Z',
            _ => b.is_ascii_digit(),
        })
}

/// The copies of one app's save folder, newest first.
pub fn copies(dir: &Path) -> Vec<SaveCopy> {
    let mut out: Vec<SaveCopy> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().into_string().ok()?;
            if !valid_copy_id(&id) {
                return None;
            }
            let at = time::PrimitiveDateTime::parse(
                &id,
                format_description!("[year][month][day]T[hour][minute][second]Z"),
            )
            .ok()?
            .assume_utc();
            Some(SaveCopy {
                taken_at: at.format(&Rfc3339).ok()?,
                bytes: size(&e.path()).unwrap_or(0),
                id,
            })
        })
        .collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

/// Copies `saves` into `copies_dir` as a new copy, keeping the newest few. Returns `None`,
/// without copying, when the folder is empty or too big.
pub fn take_copy(saves: &Path, copies_dir: &Path) -> io::Result<Option<SaveCopy>> {
    let bytes = size(saves)?;
    if bytes == 0 || bytes > MAX_COPY_BYTES {
        return Ok(None);
    }
    let now = OffsetDateTime::now_utc();
    let id = now
        .format(format_description!(
            "[year][month][day]T[hour][minute][second]Z"
        ))
        .map_err(io::Error::other)?;
    // Two in the same second: the later one wins the name.
    let dest = copies_dir.join(&id);
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    let staging = copies_dir.join(format!(".new-{id}"));
    let _ = fs::remove_dir_all(&staging);
    copy_tree(saves, &staging)?;
    fs::rename(&staging, &dest)?;
    for old in copies(copies_dir).into_iter().skip(KEEP_COPIES) {
        let _ = fs::remove_dir_all(copies_dir.join(&old.id));
    }
    Ok(copies(copies_dir).into_iter().find(|c| c.id == id))
}

/// Puts a copy back. The current saves are copied first, so a restore can be undone.
pub fn restore(saves: &Path, copies_dir: &Path, id: &str, scratch: &Path) -> io::Result<()> {
    if !valid_copy_id(id) || !copies_dir.join(id).is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no such copy"));
    }
    let incoming: PathBuf = scratch.join(format!("restore-{id}"));
    let _ = fs::remove_dir_all(&incoming);
    copy_tree(&copies_dir.join(id), &incoming)?;
    take_copy(saves, copies_dir)?;
    replace_dir(saves, &incoming, scratch)
}

/// Swaps `incoming` into `target`'s place; the old contents are deleted.
pub fn replace_dir(target: &Path, incoming: &Path, scratch: &Path) -> io::Result<()> {
    let old = scratch.join(format!("old-saves-{}", crate::random_token()));
    if target.exists() {
        fs::rename(target, &old)?;
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(incoming, target)?;
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("slop-saves-{}", rand::random::<u64>()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(unix)]
    #[test]
    fn sweeping_leaves_only_data() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempdir();
        let saves = root.join("saves");
        fs::create_dir_all(saves.join("slot1")).unwrap();
        fs::write(saves.join("slot1/save.json"), b"{}").unwrap();
        fs::write(saves.join("run.sh"), b"#!/bin/sh\n").unwrap();
        fs::set_permissions(saves.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(saves.join("slot1/evil.so"), b"\x7fELF\x02\x01").unwrap();
        fs::write(saves.join("evil.dll"), b"MZ\x90\x00").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", saves.join("link")).unwrap();

        let mut moved = sweep(&saves, &root.join("quarantine"), None).unwrap();
        moved.sort();
        assert_eq!(moved, ["evil.dll", "slot1/evil.so"]);
        assert!(root.join("quarantine/slot1/evil.so").exists());
        assert!(!saves.join("link").exists());
        assert_eq!(
            fs::metadata(saves.join("run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
        assert!(saves.join("slot1/save.json").exists());
        assert!(saves.join("slot1").is_dir(), "folders stay usable");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copies_are_kept_and_restored() {
        let root = tempdir();
        let (saves, copies_dir, scratch) = (
            root.join("saves"),
            root.join("copies"),
            root.join("scratch"),
        );
        fs::create_dir_all(&saves).unwrap();
        fs::create_dir_all(&scratch).unwrap();
        assert_eq!(
            take_copy(&saves, &copies_dir).unwrap(),
            None,
            "nothing to copy"
        );
        fs::write(saves.join("save.json"), b"level 1").unwrap();
        let first = take_copy(&saves, &copies_dir).unwrap().unwrap();
        assert_eq!(first.bytes, 7);
        fs::write(saves.join("save.json"), b"ruined").unwrap();
        // Make the next copy land in a later second.
        fs::rename(
            copies_dir.join(&first.id),
            copies_dir.join("20200101T000000Z"),
        )
        .unwrap();
        restore(&saves, &copies_dir, "20200101T000000Z", &scratch).unwrap();
        assert_eq!(fs::read(saves.join("save.json")).unwrap(), b"level 1");
        let all = copies(&copies_dir);
        assert_eq!(
            all.len(),
            2,
            "the ruined saves were copied before the restore"
        );
        assert!(restore(&saves, &copies_dir, "../../etc", &scratch).is_err());
        for i in 0..5 {
            fs::create_dir_all(copies_dir.join(format!("2019010{i}T000000Z"))).unwrap();
            fs::write(copies_dir.join(format!("2019010{i}T000000Z/x")), b"x").unwrap();
        }
        take_copy(&saves, &copies_dir).unwrap();
        assert_eq!(copies(&copies_dir).len(), KEEP_COPIES);
        fs::remove_dir_all(root).unwrap();
    }
}
