// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rate limits per client IP (SECURITY §8.1): strictest for sign-in, then writes, then reads.
//! Anonymous repository and portfolio checks (checks.rs) have their own, per hour: each one makes
//! SLAPorSLOP download from GitHub.
//!
//! Token buckets kept in memory. Each API instance limits on its own, so with N instances behind
//! the load balancer a client gets up to N times the limit; that's acceptable at launch scale
//! and the limits are deliberately conservative. A shared limiter (Postgres or the load
//! balancer's own) replaces this when SLAPorSLOP runs many instances.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, Method, StatusCode, header::RETRY_AFTER},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    /// Sign-in, sign-up and desktop token endpoints.
    Auth,
    /// Anything that changes state.
    Write,
    /// Reads.
    Read,
    /// Anonymous repository and portfolio checks (checks.rs).
    Check,
}

/// Requests per minute (per hour for checks) for each class; each also allows a burst of that
/// many at once.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub auth_per_minute: u32,
    pub write_per_minute: u32,
    pub read_per_minute: u32,
    /// Anonymous repository and portfolio checks from one address, per hour.
    pub check_per_hour: u32,
    /// GitHub API requests the anonymous checks may make per hour, from every address together
    /// (checks.rs). They ask as nobody, so they share the server's anonymous GitHub allowance
    /// (60 an hour on github.com) with licence lookups for people who don't own the repository.
    /// 0 turns the repository and portfolio checks off.
    pub check_github_per_hour: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            auth_per_minute: 20,
            write_per_minute: 60,
            read_per_minute: 600,
            check_per_hour: 20,
            check_github_per_hour: 40,
        }
    }
}

impl Limits {
    /// (how many, per how many seconds).
    fn rate(&self, class: Class) -> (u32, f64) {
        match class {
            Class::Auth => (self.auth_per_minute, 60.0),
            Class::Write => (self.write_per_minute, 60.0),
            Class::Read => (self.read_per_minute, 60.0),
            Class::Check => (self.check_per_hour, 3600.0),
        }
    }
}

struct Bucket {
    tokens: f64,
    updated: Instant,
}

/// Above this many tracked clients, idle buckets are dropped before adding more.
const MAX_BUCKETS: usize = 100_000;

pub struct RateLimiter {
    limits: Limits,
    buckets: Mutex<HashMap<(Class, IpAddr), Bucket>>,
}

impl RateLimiter {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes one token. `Err` holds how long until the next one.
    pub fn check(&self, class: Class, ip: IpAddr) -> Result<(), Duration> {
        self.check_at(class, ip, Instant::now())
    }

    fn check_at(&self, class: Class, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let (n, period) = self.limits.rate(class);
        let capacity = f64::from(n.max(1));
        let per_second = capacity / period;
        let mut buckets = self.buckets.lock().expect("rate limiter lock");
        if buckets.len() >= MAX_BUCKETS && !buckets.contains_key(&(class, ip)) {
            prune(&mut buckets, &self.limits, now);
        }
        let bucket = buckets.entry((class, ip)).or_insert(Bucket {
            tokens: capacity,
            updated: now,
        });
        let elapsed = now.saturating_duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * per_second).min(capacity);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64(
                ((1.0 - bucket.tokens) / per_second).ceil().max(1.0),
            ))
        }
    }

    /// Drops buckets that have refilled completely (their clients went quiet).
    pub fn prune(&self) {
        let mut buckets = self.buckets.lock().expect("rate limiter lock");
        prune(&mut buckets, &self.limits, Instant::now());
    }
}

fn prune(buckets: &mut HashMap<(Class, IpAddr), Bucket>, limits: &Limits, now: Instant) {
    buckets.retain(|(class, _), b| {
        let (n, period) = limits.rate(*class);
        let capacity = f64::from(n.max(1));
        let refilled =
            b.tokens + now.saturating_duration_since(b.updated).as_secs_f64() * capacity / period;
        refilled < capacity
    });
}

fn classify(method: &Method, path: &str) -> Option<Class> {
    // GitHub's webhooks all come from a few of its addresses; each is checked by its signature.
    if path == "/healthz" || path == "/v1/github/webhook" {
        return None;
    }
    if path.starts_with("/v1/auth/") && path != "/v1/auth/providers" {
        return Some(Class::Auth);
    }
    // The store-address check only asks the database: an ordinary read.
    if path == "/v1/check/repository" || path == "/v1/check/portfolio" {
        return Some(Class::Check);
    }
    // AI tools registering, and starting an approval (agent_oauth.rs). Their token requests
    // count as writes: services such as Claude.ai refresh for all their users from a few
    // addresses.
    if path == "/v1/oauth/register" || path == "/v1/oauth/authorize" {
        return Some(Class::Auth);
    }
    if method == Method::GET || method == Method::HEAD || method == Method::OPTIONS {
        Some(Class::Read)
    } else {
        Some(Class::Write)
    }
}

/// The client's IP: from the configured header when behind a load balancer that sets it, else
/// the connection's peer address.
pub(crate) fn client_ip(state: &AppState, req: &Request) -> Option<IpAddr> {
    if let Some(header) = &state.config.client_ip_header {
        return req
            .headers()
            .get(header)
            .and_then(|v| v.to_str().ok())
            // X-Forwarded-For style lists: the load balancer appends the real client last.
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse().ok());
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
}

pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let Some(class) = classify(req.method(), req.uri().path()) else {
        return next.run(req).await;
    };
    // Without a known client (tests, misconfigured proxy) everyone shares one bucket.
    let ip = client_ip(&state, &req).unwrap_or(IpAddr::from([0, 0, 0, 0]));
    match state.rate_limiter.check(class, ip) {
        Ok(()) => next.run(req).await,
        Err(wait) if class == Class::Check => crate::checks::too_many(wait.as_secs()),
        Err(wait) => {
            let body = json!({ "error": { "code": "rate_limited", "message": "too many requests; try again shortly" } });
            let mut res = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
            if let Ok(v) = HeaderValue::from_str(&wait.as_secs().to_string()) {
                res.headers_mut().insert(RETRY_AFTER, v);
            }
            res
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, n])
    }

    #[test]
    fn buckets_drain_and_refill() {
        let rl = RateLimiter::new(Limits {
            auth_per_minute: 3,
            ..Limits::default()
        });
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(rl.check_at(Class::Auth, ip(1), t0).is_ok());
        }
        let wait = rl.check_at(Class::Auth, ip(1), t0).unwrap_err();
        assert_eq!(wait, Duration::from_secs(20));
        // Other clients and classes are unaffected.
        assert!(rl.check_at(Class::Auth, ip(2), t0).is_ok());
        assert!(rl.check_at(Class::Read, ip(1), t0).is_ok());
        // One token comes back every 20 seconds.
        assert!(
            rl.check_at(Class::Auth, ip(1), t0 + Duration::from_secs(20))
                .is_ok()
        );
        assert!(
            rl.check_at(Class::Auth, ip(1), t0 + Duration::from_secs(21))
                .is_err()
        );
    }

    #[test]
    fn checks_are_counted_per_hour() {
        let rl = RateLimiter::new(Limits {
            check_per_hour: 2,
            ..Limits::default()
        });
        let t0 = Instant::now();
        assert!(rl.check_at(Class::Check, ip(1), t0).is_ok());
        assert!(rl.check_at(Class::Check, ip(1), t0).is_ok());
        assert_eq!(
            rl.check_at(Class::Check, ip(1), t0).unwrap_err(),
            Duration::from_secs(1800)
        );
        assert!(rl.check_at(Class::Read, ip(1), t0).is_ok());
    }

    #[test]
    fn idle_buckets_are_pruned() {
        let rl = RateLimiter::new(Limits::default());
        let t0 = Instant::now();
        rl.check_at(Class::Write, ip(1), t0).unwrap();
        prune(
            &mut rl.buckets.lock().unwrap(),
            &rl.limits,
            t0 + Duration::from_secs(120),
        );
        assert!(rl.buckets.lock().unwrap().is_empty());
    }

    #[test]
    fn classes() {
        assert_eq!(
            classify(&Method::POST, "/v1/auth/desktop/token"),
            Some(Class::Auth)
        );
        assert_eq!(
            classify(&Method::GET, "/v1/auth/github/start"),
            Some(Class::Auth)
        );
        assert_eq!(
            classify(&Method::GET, "/v1/auth/providers"),
            Some(Class::Read)
        );
        assert_eq!(classify(&Method::PATCH, "/v1/apps/x"), Some(Class::Write));
        assert_eq!(classify(&Method::GET, "/v1/apps"), Some(Class::Read));
        assert_eq!(classify(&Method::GET, "/healthz"), None);
        assert_eq!(
            classify(&Method::GET, "/v1/check/repository"),
            Some(Class::Check)
        );
        assert_eq!(
            classify(&Method::GET, "/v1/check/store-address"),
            Some(Class::Read)
        );
    }
}
