// SPDX-License-Identifier: AGPL-3.0-or-later
//! Save sync (plan phase 15): an app's save folder follows the player between their computers,
//! end-to-end encrypted, so Slop's server stores only ciphertext (slop-api `save_sync.rs`).
//!
//! - **The key** comes from the player's chat backup key (chat/mod.rs `save_sync_key`), which
//!   their approved devices share and their recovery code restores. Without it, nothing syncs.
//! - **Packing:** the save folder as a tar.gz with fixed times, encrypted with XChaCha20-Poly1305;
//!   the app's address is bound in, so one app's save can't be passed off as another's.
//! - **When:** before a launch the newest save is fetched if another computer made it and this
//!   one's saves haven't changed since the last sync; after the app exits, changed saves are sent.
//!   Before synced saves replace the folder, a copy is taken (saves.rs), so it can be undone.
//! - **Both changed:** nothing is overwritten. The player chooses this computer's saves
//!   (`push` with `force`) or the other's (`pull` with `force`; this computer's go to the copies).

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Error, Method, Result,
    api::{ApiClient, RawBody},
};

/// Largest save Slop syncs (packed and encrypted).
pub const MAX_BYTES: usize = 20 * 1024 * 1024;
const MAGIC: &[u8] = b"SLOPSV1";

/// The key for saves, from the chat backup key: a different key for a different use.
pub fn derive_key(backup_key: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"slop/save-sync/v1\n");
    h.update(backup_key);
    h.finalize().into()
}

/// What this computer knows about each app's synced saves.
#[derive(Default, Serialize, Deserialize)]
pub struct State {
    /// This device's name, sent with uploads.
    pub device: String,
    /// The player turned save sync off on this computer.
    #[serde(default)]
    pub off: bool,
    pub apps: BTreeMap<String, Synced>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Synced {
    /// The generation this computer last sent or fetched.
    pub generation: i32,
    /// The save folder's fingerprint then.
    pub fingerprint: String,
}

impl State {
    pub fn load(path: &Path) -> State {
        let mut s: State = fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if s.device.is_empty() {
            s.device = format!(
                "d-{}",
                crate::random_token().chars().take(16).collect::<String>()
            );
        }
        s
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(io::Error::other)?,
        )?;
        fs::rename(tmp, path)
    }
}

/// Every file under `dir` (relative path, with `/`), sorted.
fn files(dir: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries {
            let e = e?;
            let t = e.file_type()?;
            if t.is_dir() {
                stack.push(e.path());
            } else if t.is_file() {
                let rel = e
                    .path()
                    .strip_prefix(dir)
                    .map_err(io::Error::other)?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, e.path()));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// A fingerprint of the folder's contents: changes when any file's name or bytes do.
pub fn fingerprint(dir: &Path) -> io::Result<String> {
    let mut h = Sha256::new();
    for (rel, path) in files(dir)? {
        h.update(rel.as_bytes());
        h.update([0]);
        h.update(Sha256::digest(fs::read(path)?));
    }
    Ok(hex::encode(h.finalize()))
}

/// The folder as a tar.gz, with fixed times and permissions so equal folders pack equally.
pub fn pack(dir: &Path) -> io::Result<Vec<u8>> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    for (rel, path) in files(dir)? {
        let data = fs::read(&path)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_cksum();
        tar.append_data(&mut header, &rel, data.as_slice())?;
    }
    tar.into_inner()?.finish()
}

/// Unpacks into `dest` (which must not exist): plain files and folders only, nothing outside.
pub fn unpack(packed: &[u8], dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(packed));
    let mut total = 0u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let rel = entry.path()?.into_owned();
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a path outside the save folder",
            ));
        }
        let mut data = Vec::new();
        entry
            .by_ref()
            .take(MAX_BYTES as u64 * 4 + 1)
            .read_to_end(&mut data)?;
        total += data.len() as u64;
        if total > MAX_BYTES as u64 * 4 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "too big"));
        }
        let path = dest.join(&rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, data)?;
    }
    Ok(())
}

/// Encrypts a packed save for `slug`.
pub fn seal(key: &[u8; 32], slug: &str, packed: &[u8]) -> Vec<u8> {
    let nonce: [u8; 24] = rand::random();
    let cipher = XChaCha20Poly1305::new(key.into());
    let sealed = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: packed,
                aad: slug.as_bytes(),
            },
        )
        .expect("encrypting in memory");
    [MAGIC, &nonce, &sealed].concat()
}

/// Decrypts a save for `slug`; fails if the key, the app or a byte is wrong.
pub fn open(key: &[u8; 32], slug: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    let rest = sealed
        .strip_prefix(MAGIC)
        .filter(|r| r.len() > 24)
        .ok_or_else(|| Error::Verify("that isn't a Slop save".into()))?;
    let (nonce, body) = rest.split_at(24);
    let nonce: [u8; 24] = nonce.try_into().expect("24 bytes");
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: body,
                aad: slug.as_bytes(),
            },
        )
        .map_err(|_| Error::Verify("the synced save doesn't open with this computer's key".into()))
}

/// What a sync did.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    UpToDate,
    /// The other computer's saves are here now.
    Fetched {
        generation: i32,
    },
    Sent {
        generation: i32,
    },
    /// Both this computer's and another's saves changed: the player chooses.
    BothChanged {
        other_device: String,
    },
    /// Over 20 MB packed: not synced.
    TooBig,
}

fn path_for(slug: &str) -> String {
    // App addresses are lowercase letters, digits and dashes: nothing to escape.
    format!("/v1/me/saves/{slug}")
}

/// Before a launch: fetch another computer's newer saves, if this one's haven't changed.
#[allow(clippy::too_many_arguments)]
pub async fn pull(
    api: &ApiClient,
    key: &[u8; 32],
    slug: &str,
    saves: &Path,
    copies: &Path,
    scratch: &Path,
    state: &mut State,
    force: bool,
) -> Result<Outcome> {
    let res = api
        .request_raw(Method::GET, &path_for(slug), RawBody::None, &[])
        .await?;
    if res.status == 404 {
        return Ok(Outcome::UpToDate);
    }
    if res.status != 200 {
        return Err(Error::Install(format!(
            "Slop answered {} for the synced save",
            res.status
        )));
    }
    let generation: i32 = res
        .header("x-slop-generation")
        .and_then(|g| g.parse().ok())
        .unwrap_or(0);
    let device = res.header("x-slop-device").unwrap_or_default();
    let known = state.apps.get(slug).cloned().unwrap_or_default();
    if generation <= known.generation && !force {
        return Ok(Outcome::UpToDate);
    }
    let here = fingerprint(saves)?;
    let changed_here = !known.fingerprint.is_empty() && here != known.fingerprint
        || known.fingerprint.is_empty() && !files(saves)?.is_empty();
    if changed_here && !force {
        return Ok(Outcome::BothChanged {
            other_device: device,
        });
    }
    let packed = open(key, slug, &res.bytes)?;
    let incoming = scratch.join(format!("synced-{}", crate::random_token()));
    unpack(&packed, &incoming)?;
    fs::create_dir_all(copies)?;
    crate::saves::take_copy(saves, copies)?;
    crate::saves::replace_dir(saves, &incoming, scratch)?;
    state.apps.insert(
        slug.to_string(),
        Synced {
            generation,
            fingerprint: fingerprint(saves)?,
        },
    );
    Ok(Outcome::Fetched { generation })
}

/// After the app exits: send this computer's saves if they changed since the last sync.
pub async fn push(
    api: &ApiClient,
    key: &[u8; 32],
    slug: &str,
    saves: &Path,
    state: &mut State,
    force: bool,
) -> Result<Outcome> {
    let here = fingerprint(saves)?;
    let known = state.apps.get(slug).cloned().unwrap_or_default();
    if here == known.fingerprint && !force {
        return Ok(Outcome::UpToDate);
    }
    let packed = pack(saves)?;
    if packed.len() > MAX_BYTES {
        return Ok(Outcome::TooBig);
    }
    // Keeping this computer's saves over another's: build on the newest there.
    let base = if force {
        let res = api
            .request_raw(Method::GET, &path_for(slug), RawBody::None, &[])
            .await?;
        res.header("x-slop-generation")
            .and_then(|g| g.parse().ok())
            .unwrap_or(0)
    } else {
        known.generation
    };
    let sealed = seal(key, slug, &packed);
    let res = api
        .request_raw(
            Method::PUT,
            &path_for(slug),
            RawBody::Bytes(sealed),
            &[
                ("x-slop-base", base.to_string()),
                ("x-slop-device", state.device.clone()),
            ],
        )
        .await?;
    match res.status {
        200 => {
            let generation = serde_json::from_slice::<serde_json::Value>(&res.bytes)
                .ok()
                .and_then(|v| v["generation"].as_i64())
                .unwrap_or(0) as i32;
            state.apps.insert(
                slug.to_string(),
                Synced {
                    generation,
                    fingerprint: here,
                },
            );
            Ok(Outcome::Sent { generation })
        }
        409 => Ok(Outcome::BothChanged {
            other_device: String::new(),
        }),
        s => Err(Error::Install(format!(
            "Slop answered {s} when sending the save"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("slop-sync-{tag}-{}", rand::random::<u64>()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn packs_seals_and_unpacks_the_same_folder() {
        let dir = tempdir("a");
        fs::create_dir_all(dir.join("slots")).unwrap();
        fs::write(dir.join("game.json"), r#"{"level":3}"#).unwrap();
        fs::write(dir.join("slots/1.sav"), [0u8, 1, 2]).unwrap();
        let key = derive_key(&[7u8; 32]);
        let packed = pack(&dir).unwrap();
        assert_eq!(
            packed,
            pack(&dir).unwrap(),
            "the same folder packs the same"
        );
        let sealed = seal(&key, "pong", &packed);
        assert!(open(&key, "tetris", &sealed).is_err(), "bound to its app");
        assert!(
            open(&derive_key(&[8u8; 32]), "pong", &sealed).is_err(),
            "another key"
        );
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(open(&key, "pong", &tampered).is_err());
        let out = tempdir("b").join("saves");
        unpack(&open(&key, "pong", &sealed).unwrap(), &out).unwrap();
        assert_eq!(fingerprint(&out).unwrap(), fingerprint(&dir).unwrap());
        fs::write(dir.join("game.json"), r#"{"level":4}"#).unwrap();
        assert_ne!(fingerprint(&out).unwrap(), fingerprint(&dir).unwrap());
    }

    #[test]
    fn nothing_lands_outside_the_folder() {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        let mut h = tar::Header::new_gnu();
        h.set_size(1);
        h.set_cksum();
        // tar refuses ".." in append_data, so write the name by hand.
        h.as_old_mut().name[..9].copy_from_slice(b"../escape");
        h.set_cksum();
        tar.append(&h, &b"x"[..]).unwrap();
        let bytes = tar.into_inner().unwrap().finish().unwrap();
        let dest = tempdir("c").join("saves");
        assert!(unpack(&bytes, &dest).is_err());
        assert!(!dest.parent().unwrap().join("escape").exists());
    }
}
