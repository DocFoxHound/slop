//! What's inside an encrypted chat message, and the small crypto around it: franking tags,
//! attachment encryption, the local store's key and the backup key.

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{Error, Result};

/// One message, as encrypted. `fk` is the franking key (SECURITY §8.5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Content {
    pub v: u8,
    pub id: Uuid,
    #[serde(flatten)]
    pub body: Body,
    pub fk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Body {
    /// Text (Markdown-lite), with any pictures, GIFs, videos or files, a link preview, and what
    /// it replies to.
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<AttachmentRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<LinkPreview>,
    },
    Reaction {
        target: Uuid,
        emoji: String,
        on: bool,
    },
    Edit {
        target: Uuid,
        text: String,
    },
    Delete {
        target: Uuid,
    },
    /// A group chat's name. Slop never sees it.
    Rename {
        name: String,
    },
    /// Between a user's own devices (the self chat): the backup key, and a copy of the history
    /// for a newly approved device.
    DeviceSync {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backup_key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history: Option<AttachmentRef>,
    },
}

/// An attachment, encrypted on the sending device with its own key (SECURITY §8.5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttachmentRef {
    /// Slop's ID for the encrypted bytes.
    pub id: Uuid,
    /// The key and nonce, base64.
    pub key: String,
    pub nonce: String,
    /// SHA-256 of the plaintext, hex.
    pub sha256: String,
    pub mime: String,
    pub name: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// A link the sender pasted that this was fetched from (a meme's address).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

/// A link preview made by the sender's app, so recipients' apps never contact the site
/// (SECURITY §8.5: link warnings run on the recipient's side, previews on the sender's).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkPreview {
    pub url: String,
    pub site: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<AttachmentRef>,
}

pub fn random<const N: usize>() -> [u8; N] {
    rand::random::<[u8; N]>()
}

impl Content {
    pub fn new(body: Body) -> Self {
        Self {
            v: 1,
            id: Uuid::now_v7(),
            body,
            fk: B64.encode(random::<32>()),
        }
    }

    /// The plaintext that's encrypted, and its franking tag for Slop.
    pub fn seal(&self) -> Result<(Vec<u8>, [u8; 32])> {
        let plaintext = serde_json::to_vec(self).map_err(|e| Error::Verify(e.to_string()))?;
        let tag = franking_tag(&self.fk, &plaintext)?;
        Ok((plaintext, tag))
    }

    pub fn parse(plaintext: &[u8]) -> Result<Self> {
        serde_json::from_slice(plaintext)
            .map_err(|e| Error::Verify(format!("unreadable message: {e}")))
    }
}

/// HMAC-SHA256(franking key, plaintext): Slop stores it, and checks a reported message against it.
pub fn franking_tag(key_b64: &str, plaintext: &[u8]) -> Result<[u8; 32]> {
    let key = B64
        .decode(key_b64)
        .map_err(|_| Error::Verify("bad franking key".into()))?;
    let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(&key)
        .map_err(|_| Error::Verify("bad franking key".into()))?;
    mac.update(plaintext);
    Ok(mac.finalize().into_bytes().into())
}

/// Encrypts with XChaCha20-Poly1305 under a fresh nonce; returns (nonce, ciphertext).
pub fn seal(key: &[u8; 32], plaintext: &[u8]) -> Result<([u8; 24], Vec<u8>)> {
    let nonce = random::<24>();
    let cipher = XChaCha20Poly1305::new(key.into());
    let ciphertext = cipher
        .encrypt(&XNonce::from(nonce), plaintext)
        .map_err(|_| Error::Verify("encryption failed".into()))?;
    Ok((nonce, ciphertext))
}

pub fn open(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    let nonce: [u8; 24] = nonce
        .try_into()
        .map_err(|_| Error::Verify("bad nonce".into()))?;
    XChaCha20Poly1305::new(key.into())
        .decrypt(&XNonce::from(nonce), ciphertext)
        .map_err(|_| Error::Verify("couldn't decrypt: wrong key, or the data was changed".into()))
}

/// Seals with the nonce in front, for files (the local store, backups).
pub fn seal_blob(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let (nonce, mut ciphertext) = seal(key, plaintext)?;
    let mut out = nonce.to_vec();
    out.append(&mut ciphertext);
    Ok(out)
}

pub fn open_blob(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < 24 {
        return Err(Error::Verify("too short".into()));
    }
    open(key, &blob[..24], &blob[24..])
}

/// An attachment's bytes, encrypted for upload, and the reference that goes in the message.
pub fn encrypt_attachment(
    bytes: &[u8],
    mime: &str,
    name: &str,
) -> Result<(Vec<u8>, AttachmentRef)> {
    let key = random::<32>();
    let (nonce, ciphertext) = seal(&key, bytes)?;
    Ok((
        ciphertext,
        AttachmentRef {
            id: Uuid::nil(),
            key: B64.encode(key),
            nonce: B64.encode(nonce),
            sha256: hex::encode(Sha256::digest(bytes)),
            mime: mime.to_string(),
            name: name.to_string(),
            size: bytes.len() as u64,
            width: None,
            height: None,
            source_url: None,
        },
    ))
}

pub fn decrypt_attachment(a: &AttachmentRef, ciphertext: &[u8]) -> Result<Vec<u8>> {
    let key: [u8; 32] = B64
        .decode(&a.key)
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or_else(|| Error::Verify("bad attachment key".into()))?;
    let nonce = B64
        .decode(&a.nonce)
        .map_err(|_| Error::Verify("bad attachment nonce".into()))?;
    let bytes = open(&key, &nonce, ciphertext)?;
    if hex::encode(Sha256::digest(&bytes)) != a.sha256 {
        return Err(Error::Verify(
            "the attachment doesn't match what was sent".into(),
        ));
    }
    Ok(bytes)
}

/// A recovery code: 24 characters a person can write down (120 bits).
pub fn new_recovery_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let bytes = random::<24>();
    let chars: Vec<char> = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// The backup key from a recovery code (Argon2id, salted with the user's ID). Only the user
/// has the code, so Slop can't decrypt the backup.
pub fn backup_key(recovery_code: &str, user: Uuid) -> Result<[u8; 32]> {
    let normalized: String = recovery_code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if normalized.len() != 24 {
        return Err(Error::Verify(
            "a recovery code has 24 letters and numbers".into(),
        ));
    }
    let mut key = [0u8; 32];
    argon2::Argon2::default()
        .hash_password_into(
            normalized.as_bytes(),
            format!("slop-chat-backup:{user}").as_bytes(),
            &mut key,
        )
        .map_err(|e| Error::Verify(e.to_string()))?;
    Ok(key)
}

/// A user's security code, from their root device key: the same on everyone's screen, so two
/// people can compare it to check no one is in between (SECURITY §8.5).
pub fn security_code(user: Uuid, root_key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"slop-chat-security-code:v1:");
    hasher.update(user.as_bytes());
    hasher.update(root_key);
    let hash = hasher.finalize();
    hash.chunks(3)
        .take(8)
        .map(|c| {
            let n = (u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2])) % 100_000;
            format!("{n:05}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn franking_tags_bind_the_exact_plaintext() {
        let content = Content::new(Body::Text {
            text: "hello".into(),
            reply_to: None,
            attachments: vec![],
            preview: None,
        });
        let (plaintext, tag) = content.seal().unwrap();
        assert_eq!(franking_tag(&content.fk, &plaintext).unwrap(), tag);
        let mut changed = plaintext.clone();
        changed[10] ^= 1;
        assert_ne!(franking_tag(&content.fk, &changed).unwrap(), tag);
        assert_eq!(Content::parse(&plaintext).unwrap(), content);
    }

    #[test]
    fn attachments_round_trip_and_detect_changes() {
        let (mut ct, a) = encrypt_attachment(b"a meme", "image/gif", "meme.gif").unwrap();
        assert_eq!(decrypt_attachment(&a, &ct).unwrap(), b"a meme");
        ct[0] ^= 1;
        assert!(decrypt_attachment(&a, &ct).is_err());
    }

    #[test]
    fn recovery_codes_make_the_same_key_however_typed() {
        let code = new_recovery_code();
        assert_eq!(code.len(), 29);
        let user = Uuid::now_v7();
        let a = backup_key(&code, user).unwrap();
        let b = backup_key(&code.to_lowercase().replace('-', " "), user).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, backup_key(&code, Uuid::now_v7()).unwrap());
        assert!(backup_key("short", user).is_err());
    }

    #[test]
    fn security_codes_are_eight_groups_of_digits() {
        let code = security_code(Uuid::nil(), &[7; 32]);
        assert_eq!(code.split(' ').count(), 8);
        assert!(code.chars().all(|c| c.is_ascii_digit() || c == ' '));
        assert_ne!(code, security_code(Uuid::nil(), &[8; 32]));
    }
}
