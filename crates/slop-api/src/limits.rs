// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rate limits per account (SECURITY §8.1), on top of the per-IP limits in ratelimit.rs: someone
//! with many addresses still can't vote, report, message or invite faster than a person would.
//!
//! Token buckets in memory, like ratelimit.rs (each API instance limits on its own). The limits
//! are generous: a person using Slop normally never meets them. Hitting one answers 429 with
//! `Retry-After`, and repeated hits are a signal for abuse checks (abuse.rs).

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use uuid::Uuid;

use crate::{AppState, error::ApiError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Vote,
    Review,
    Report,
    ChatMessage,
    FriendRequest,
    Donation,
    Share,
    Interest,
    TeamInvite,
    NewApp,
    Release,
    Hire,
    JobMessage,
    Offer,
    Lookup,
}

impl Action {
    /// (how many, per how long). The bucket also allows all of them at once.
    pub fn limit(self) -> (u32, Duration) {
        const MINUTE: u64 = 60;
        const HOUR: u64 = 3600;
        const DAY: u64 = 86_400;
        let (n, secs) = match self {
            Self::Vote => (60, HOUR),
            Self::Review => (10, HOUR),
            Self::Report => (10, HOUR),
            Self::ChatMessage => (60, MINUTE),
            Self::FriendRequest => (40, DAY),
            Self::Donation => (10, HOUR),
            Self::Share => (60, HOUR),
            Self::Interest => (120, HOUR),
            Self::TeamInvite => (50, DAY),
            Self::NewApp => (10, DAY),
            Self::Release => (30, HOUR),
            Self::Hire => (20, DAY),
            Self::JobMessage => (60, MINUTE),
            // Each one notifies (and emails) the original's maker.
            Self::Offer => (5, DAY),
            // Lookups that cost Slop a GitHub request.
            Self::Lookup => (30, HOUR),
        };
        (n, Duration::from_secs(secs))
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Vote => "votes",
            Self::Review => "reviews",
            Self::Report => "reports",
            Self::ChatMessage => "chat messages",
            Self::FriendRequest => "friend requests",
            Self::Donation => "donations",
            Self::Share => "shares",
            Self::Interest => "Interested marks",
            Self::TeamInvite => "team invitations",
            Self::NewApp => "new apps",
            Self::Release => "releases",
            Self::Hire => "hiring requests",
            Self::JobMessage => "job messages",
            Self::Offer => "offers to other makers",
            Self::Lookup => "repository lookups",
        }
    }
}

struct Bucket {
    tokens: f64,
    updated: Instant,
}

const MAX_BUCKETS: usize = 200_000;

#[derive(Default)]
pub struct AccountLimiter {
    buckets: Mutex<HashMap<(Action, Uuid), Bucket>>,
    /// How often each account hit a limit recently (for abuse checks).
    hits: Mutex<HashMap<Uuid, (u32, Instant)>>,
}

impl AccountLimiter {
    /// Takes one. `Err` is how long until the next.
    pub fn take(&self, user: Uuid, action: Action, now: Instant) -> Result<(), Duration> {
        let (n, per) = action.limit();
        let capacity = f64::from(n.max(1));
        let per_second = capacity / per.as_secs_f64();
        let mut buckets = self.buckets.lock().expect("account limiter lock");
        if buckets.len() >= MAX_BUCKETS {
            buckets.retain(|(a, _), b| {
                let (n, per) = a.limit();
                b.tokens
                    + now.saturating_duration_since(b.updated).as_secs_f64() * f64::from(n)
                        / per.as_secs_f64()
                    < f64::from(n)
            });
        }
        let b = buckets.entry((action, user)).or_insert(Bucket {
            tokens: capacity,
            updated: now,
        });
        b.tokens = (b.tokens + now.saturating_duration_since(b.updated).as_secs_f64() * per_second)
            .min(capacity);
        b.updated = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            return Ok(());
        }
        drop(buckets);
        let mut hits = self.hits.lock().expect("account limiter lock");
        let e = hits.entry(user).or_insert((0, now));
        if now.saturating_duration_since(e.1) > Duration::from_secs(86_400) {
            *e = (0, now);
        }
        e.0 += 1;
        Err(Duration::from_secs_f64((1.0 / per_second).ceil().max(1.0)))
    }

    /// Accounts that hit limits at least `min` times in the last day, most first.
    pub fn frequent(&self, min: u32) -> Vec<(Uuid, u32)> {
        let now = Instant::now();
        let hits = self.hits.lock().expect("account limiter lock");
        let mut out: Vec<(Uuid, u32)> = hits
            .iter()
            .filter(|(_, (n, since))| {
                *n >= min && now.saturating_duration_since(*since) <= Duration::from_secs(86_400)
            })
            .map(|(u, (n, _))| (*u, *n))
            .collect();
        out.sort_by_key(|e| std::cmp::Reverse(e.1));
        out
    }
}

/// Refuses (429) an account doing `action` faster than a person would.
pub fn check(state: &AppState, user: Uuid, action: Action) -> Result<(), ApiError> {
    state
        .account_limiter
        .take(user, action, Instant::now())
        .map_err(|wait| ApiError::TooFast {
            what: action.name(),
            retry_after: wait.as_secs(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_empties_then_refills() {
        let l = AccountLimiter::default();
        let user = Uuid::now_v7();
        let t0 = Instant::now();
        for _ in 0..10 {
            assert!(l.take(user, Action::Review, t0).is_ok());
        }
        let wait = l.take(user, Action::Review, t0).unwrap_err();
        assert!(wait >= Duration::from_secs(300), "{wait:?}");
        assert!(
            l.take(Uuid::now_v7(), Action::Review, t0).is_ok(),
            "per account"
        );
        assert!(l.take(user, Action::Vote, t0).is_ok(), "per action");
        assert!(
            l.take(user, Action::Review, t0 + Duration::from_secs(361))
                .is_ok()
        );
        assert_eq!(l.frequent(1), vec![(user, 1)]);
    }
}
