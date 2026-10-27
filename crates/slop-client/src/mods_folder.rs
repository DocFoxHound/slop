// SPDX-License-Identifier: AGPL-3.0-or-later
//! A game's own mods folder (DESIGN §9). Its maker can name the folder the game already reads
//! mods from (`mods/` next to the game, `Content/Paks/~mods`, a folder in its saves). Before each
//! start, and when the player adds mods while it runs, SLAPorSLOP makes that folder hold exactly the
//! mods the player added from SLAPorSLOP: their files are copied in, files that changed are put back,
//! and anything else, such as a mod someone dropped in by hand, is moved out to the quarantine
//! folder and the player is told. In sandbox mode the folder is read-only, so the game can't
//! put anything there itself.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};
use slop_manifest::ModsLayout;

/// What SLAPorSLOP last put in the folder: path (with `/`) → (size, modified time in ns), to tell
/// its own files apart from changed ones.
type Record = BTreeMap<String, (u64, u128)>;

fn stamp(meta: &fs::Metadata) -> (u64, u128) {
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    (meta.len(), modified)
}

/// Every plain file under `dir`, relative, with `/`; links and anything else are left out.
fn files_in(dir: &Path) -> io::Result<Vec<String>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(root, &entry.path(), out)?;
            } else if kind.is_file() {
                let rel = entry.path();
                let rel = rel.strip_prefix(root).unwrap_or(&rel);
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if dir.is_dir() {
        walk(dir, dir, &mut out)?;
    }
    Ok(out)
}

/// What a sync found.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Synced {
    /// Names (the first part of the path) of things in the folder that aren't mods from SLAPorSLOP;
    /// they were moved to the quarantine folder.
    pub unapproved: Vec<String>,
    /// Files two mods both have, in the `flat` layout: the first mod's copy is kept.
    pub clashes: Vec<String>,
}

/// Makes `target` hold exactly `mods` (address, unpacked files), laid out as `layout`.
/// `record` is SLAPorSLOP's note of what it put there; unapproved files go under `quarantine`.
pub fn sync(
    target: &Path,
    layout: ModsLayout,
    mods: &[(String, PathBuf)],
    record: &Path,
    quarantine: &Path,
) -> io::Result<Synced> {
    let mut synced = Synced::default();
    // Where each file should come from.
    let mut wanted: BTreeMap<String, PathBuf> = BTreeMap::new();
    for (slug, files) in mods {
        for rel in files_in(files)? {
            let at = match layout {
                ModsLayout::Folder => format!("{slug}/{rel}"),
                ModsLayout::Flat => rel.clone(),
            };
            if wanted.contains_key(&at) {
                synced.clashes.push(at);
                continue;
            }
            wanted.insert(at, files.join(&rel));
        }
    }
    let before: Record = fs::read(record)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    fs::create_dir_all(target)?;
    let mut keep: Record = Record::new();
    let mut unapproved = BTreeSet::new();
    let mut walk = vec![target.to_path_buf()];
    while let Some(dir) = walk.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            let rel = path
                .strip_prefix(target)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if kind.is_dir() {
                walk.push(path);
            } else if kind.is_file() {
                let now = stamp(&entry.metadata()?);
                if wanted.contains_key(&rel) {
                    // SLAPorSLOP's own copy, unchanged since it was put there: keep it.
                    if before.get(&rel) == Some(&now) {
                        keep.insert(rel, now);
                    }
                } else if before.contains_key(&rel) {
                    // A mod the player removed (or SLAPorSLOP pulled).
                    fs::remove_file(&path)?;
                } else {
                    let to = quarantine.join(&rel);
                    if let Some(parent) = to.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::rename(&path, &to)
                        .or_else(|_| fs::copy(&path, &to).and_then(|_| fs::remove_file(&path)))?;
                    unapproved.insert(rel.split('/').next().unwrap_or(&rel).to_string());
                }
            } else {
                // Links and special files: a link could point anywhere.
                fs::remove_file(&path)?;
            }
        }
    }
    for (rel, from) in &wanted {
        if keep.contains_key(rel) {
            continue;
        }
        let to = target.join(rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        if to.is_dir() {
            // A folder by hand where a mod's file goes.
            fs::remove_dir_all(&to)?;
        }
        fs::copy(from, &to)?;
        keep.insert(rel.clone(), stamp(&fs::metadata(&to)?));
    }
    prune(target, target)?;
    if let Some(parent) = record.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(record, serde_json::to_vec(&keep).expect("serialises"))?;
    synced.unapproved = unapproved.into_iter().collect();
    Ok(synced)
}

/// Removes empty folders below `dir`. Returns whether `dir` itself is empty now.
fn prune(root: &Path, dir: &Path) -> io::Result<bool> {
    let mut empty = true;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && prune(root, &entry.path())? {
            fs::remove_dir(entry.path())?;
        } else {
            empty = false;
        }
    }
    Ok(empty && dir != root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("slop-modsdir-{}", rand::random::<u64>()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_folder_holds_only_mods_from_slop() {
        let root = tempdir();
        let (target, record, quarantine) = (
            root.join("game/mods"),
            root.join("record.json"),
            root.join("quarantine"),
        );
        let dragons = root.join("m/dragons");
        fs::create_dir_all(dragons.join("data")).unwrap();
        fs::write(dragons.join("data/dragon.json"), b"{}").unwrap();
        fs::write(dragons.join("dragons.pck"), b"pack").unwrap();
        let mods = vec![("dragons".to_string(), dragons.clone())];

        // Something put there by hand is moved out; SLAPorSLOP's mods go in, each in its own folder.
        fs::create_dir_all(target.join("my-hack")).unwrap();
        fs::write(target.join("my-hack/hack.gd"), b"extends Node").unwrap();
        fs::write(target.join("loose.zip"), b"zip").unwrap();
        let got = sync(&target, ModsLayout::Folder, &mods, &record, &quarantine).unwrap();
        assert_eq!(got.unapproved, ["loose.zip", "my-hack"]);
        assert!(quarantine.join("my-hack/hack.gd").exists());
        assert!(!target.join("my-hack").exists());
        assert_eq!(
            fs::read(target.join("dragons/data/dragon.json")).unwrap(),
            b"{}"
        );

        // A changed file is put back; nothing is reported for SLAPorSLOP's own files.
        fs::write(target.join("dragons/dragons.pck"), b"tampered!").unwrap();
        let got = sync(&target, ModsLayout::Folder, &mods, &record, &quarantine).unwrap();
        assert_eq!(got, Synced::default());
        assert_eq!(
            fs::read(target.join("dragons/dragons.pck")).unwrap(),
            b"pack"
        );

        // A removed mod goes quietly.
        let got = sync(&target, ModsLayout::Folder, &[], &record, &quarantine).unwrap();
        assert!(got.unapproved.is_empty());
        assert!(!target.join("dragons").exists());
        assert!(target.is_dir());

        // Flat: files straight in the folder; the first mod wins a clash.
        let other = root.join("m/other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("dragons.pck"), b"other").unwrap();
        let both = vec![mods[0].clone(), ("other".to_string(), other)];
        let got = sync(&target, ModsLayout::Flat, &both, &record, &quarantine).unwrap();
        assert_eq!(got.clashes, ["dragons.pck"]);
        assert_eq!(fs::read(target.join("dragons.pck")).unwrap(), b"pack");
        assert!(target.join("data/dragon.json").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
