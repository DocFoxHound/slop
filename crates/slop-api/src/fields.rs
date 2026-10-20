//! Personal fields encrypted at rest (SECURITY §8.1, §10): ChaCha20-Poly1305 under
//! `SLOP_FIELD_KEY`, with the column's name as associated data so a value can't be moved to
//! another column. Stored as `enc1:<base64 of nonce and ciphertext>`.
//!
//! Without the key (development, tests) values are stored as they are, and `open` passes plain
//! values through, so turning the key on later works: `seal_existing` encrypts what's there.
//! Today this covers `users.vat_number`.

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};

use crate::AppState;

const PREFIX: &str = "enc1:";

pub fn seal(key: Option<&[u8; 32]>, column: &str, plain: &str) -> anyhow::Result<String> {
    let Some(key) = key else {
        return Ok(plain.to_string());
    };
    let nonce: [u8; 12] = rand::random();
    let sealed = ChaCha20Poly1305::new(key.into())
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: plain.as_bytes(),
                aad: column.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("couldn't encrypt {column}"))?;
    Ok(format!(
        "{PREFIX}{}",
        B64.encode([nonce.as_slice(), &sealed].concat())
    ))
}

pub fn open(key: Option<&[u8; 32]>, column: &str, stored: &str) -> anyhow::Result<String> {
    let Some(body) = stored.strip_prefix(PREFIX) else {
        return Ok(stored.to_string());
    };
    let key =
        key.ok_or_else(|| anyhow::anyhow!("{column} is encrypted but SLOP_FIELD_KEY isn't set"))?;
    let bytes = B64.decode(body)?;
    anyhow::ensure!(bytes.len() > 12, "{column} is damaged");
    let (nonce, ct) = bytes.split_at(12);
    let nonce: [u8; 12] = nonce.try_into()?;
    let plain = ChaCha20Poly1305::new(key.into())
        .decrypt(
            &Nonce::from(nonce),
            Payload {
                msg: ct,
                aad: column.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("{column} doesn't decrypt (was SLOP_FIELD_KEY changed?)"))?;
    Ok(String::from_utf8(plain)?)
}

/// Encrypts values stored before the key was set. Run at start-up.
pub async fn seal_existing(state: &AppState) -> anyhow::Result<usize> {
    let Some(key) = state.config.field_key.as_ref() else {
        return Ok(0);
    };
    let rows: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        "select id, vat_number from users where vat_number is not null and vat_number not like 'enc1:%'",
    )
    .fetch_all(&state.db)
    .await?;
    for (id, plain) in &rows {
        sqlx::query("update users set vat_number = $2 where id = $1")
            .bind(id)
            .bind(seal(Some(key), "users.vat_number", plain)?)
            .execute(&state.db)
            .await?;
    }
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_values_open_only_in_their_column() {
        let key = [5u8; 32];
        let s = seal(Some(&key), "users.vat_number", "DE123456789").unwrap();
        assert!(s.starts_with("enc1:") && !s.contains("DE123"));
        assert_eq!(
            open(Some(&key), "users.vat_number", &s).unwrap(),
            "DE123456789"
        );
        assert!(open(Some(&key), "users.email", &s).is_err());
        assert!(open(None, "users.vat_number", &s).is_err());
        assert_eq!(
            open(None, "users.vat_number", "DE1").unwrap(),
            "DE1",
            "plain passes through"
        );
        assert_eq!(seal(None, "users.vat_number", "DE1").unwrap(), "DE1");
    }
}
