// SPDX-License-Identifier: AGPL-3.0-or-later
//! Proof-of-work checks (PAYMENTS §9: card testing).
//!
//! After a few unfinished checkouts, starting another donation needs the browser to find a
//! number that, hashed with a random prefix from SLAPorSLOP, gives a SHA-256 starting with enough zero
//! bits. That takes a person's browser a second or two, but makes trying hundreds of stolen
//! cards slow and costly. Unlike a CAPTCHA service, nothing about the person goes to a third
//! party. Each challenge is for one account, works once, and expires after 10 minutes.

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::Duration;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
};

/// About a million hashes: a second or two in a browser, and enough that testing many cards
/// costs real time (raised from 18 in the security review, 2026-10-22).
pub const DIFFICULTY: i16 = 20;
const LIFETIME: Duration = Duration::minutes(10);

#[derive(Serialize)]
pub struct Challenge {
    pub id: Uuid,
    pub prefix: String,
    pub difficulty: i16,
}

/// The browser's answer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub id: Uuid,
    pub nonce: String,
}

pub async fn issue_route(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Challenge>> {
    sqlx::query("delete from donation_challenges where expires_at < now() - interval '1 day'")
        .execute(&state.db)
        .await?;
    let challenge = Challenge {
        id: Uuid::now_v7(),
        prefix: crate::tokens::new_token()[..24].to_string(),
        difficulty: DIFFICULTY,
    };
    sqlx::query(
        "insert into donation_challenges (id, user_id, prefix, difficulty, expires_at)
         values ($1, $2, $3, $4, now() + $5)",
    )
    .bind(challenge.id)
    .bind(user.id)
    .bind(&challenge.prefix)
    .bind(challenge.difficulty)
    .bind(LIFETIME)
    .execute(&state.db)
    .await?;
    Ok(Json(challenge))
}

/// Whether `nonce` solves `prefix` at `difficulty`: SHA-256 of "prefix:nonce" starts with that
/// many zero bits.
pub fn solves(prefix: &str, nonce: &str, difficulty: i16) -> bool {
    let hash = Sha256::digest(format!("{prefix}:{nonce}").as_bytes());
    let mut zeros = 0i16;
    for byte in hash {
        if byte == 0 {
            zeros += 8;
            continue;
        }
        zeros += byte.leading_zeros() as i16;
        break;
    }
    zeros >= difficulty
}

/// Checks an answer and uses up its challenge.
pub async fn check(db: &sqlx::PgPool, user_id: Uuid, answer: Option<&Answer>) -> ApiResult<()> {
    let Some(answer) = answer else {
        return Err(ApiError::ChallengeRequired);
    };
    if answer.nonce.len() > 40 {
        return Err(ApiError::ChallengeRequired);
    }
    let row: Option<(String, i16)> = sqlx::query_as(
        "update donation_challenges set used_at = now()
         where id = $1 and user_id = $2 and used_at is null and expires_at > now()
         returning prefix, difficulty",
    )
    .bind(answer.id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    match row {
        Some((prefix, difficulty)) if solves(&prefix, &answer.nonce, difficulty) => Ok(()),
        _ => Err(ApiError::ChallengeRequired),
    }
}

/// Solves a challenge (tests, and the reference for the browser's version).
pub fn solve(prefix: &str, difficulty: i16) -> String {
    (0u64..)
        .map(|n| n.to_string())
        .find(|n| solves(prefix, n, difficulty))
        .expect("a solution exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_checked_against_the_prefix() {
        let nonce = solve("abc", 10);
        assert!(solves("abc", &nonce, 10));
        // About one in 256 numbers meets 8 bits.
        let hits = (0..5000)
            .filter(|n| solves("abc", &n.to_string(), 8))
            .count();
        assert!((5..60).contains(&hits), "{hits}");
        // A difficulty of 0 is always met; 256 never is.
        assert!(solves("abc", "x", 0));
        assert!(!solves("abc", "x", 256));
    }
}
