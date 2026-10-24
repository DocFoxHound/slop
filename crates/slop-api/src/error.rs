// SPDX-License-Identifier: AGPL-3.0-or-later
//! API errors and their JSON shape: `{"error": {"code": "...", "message": "...", "field": "..."}}`.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("sign in to do this")]
    Unauthorized,
    #[error("you can't do this")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("{message}")]
    Invalid {
        field: &'static str,
        message: String,
    },
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Unavailable(&'static str),
    /// The action needs the current version of a legal document accepted first (legal.rs).
    /// The Publisher Agreement's message also says where: an agent's tools get it too, and its
    /// owner accepts it on the AI agents page (or the website shows it when they next publish).
    #[error("{}", if *.0 == "publisher" {
        "accept the current Publisher Agreement first: it's waiting on the website, under Account → AI agents".to_string()
    } else {
        format!("accept the current {} first", crate::legal::title(.0))
    })]
    TermsRequired(&'static str),
    #[error("your account is suspended: see the decision on your account page")]
    Suspended,
    /// A sensitive action (SECURITY §4.3) needs a sign-in from the last few minutes.
    #[error("confirm it's you first: sign in again, or use a passkey")]
    StepUpRequired,
    /// Publishing and payout changes need a passkey (passkeys.rs `require`): confirm with one
    /// (true: the account has one), or add one first (false).
    #[error("{}", if *.0 { "confirm with your passkey first" } else { "add a passkey to your account first (Account → Sign-ins): publishers and creatives need one" })]
    PasskeyRequired(bool),
    /// Too many unfinished checkouts: solve a proof-of-work challenge first (challenge.rs).
    #[error("one more step before trying again: your browser needs to do a quick check")]
    ChallengeRequired,
    /// An account doing something faster than a person would (limits.rs).
    #[error("too many {what} for now: try again in a while")]
    TooFast {
        what: &'static str,
        retry_after: u64,
    },
    /// Refused, saying why and what to do instead (403).
    #[error("{0}")]
    Refused(&'static str),
    /// Allowed again later, saying when (429 with `Retry-After`; admin_messages.rs).
    #[error("{message}")]
    NotYet { message: String, retry_after: u64 },
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn invalid(field: &'static str, message: impl Into<String>) -> Self {
        Self::Invalid {
            field,
            message: message.into(),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Invalid { .. } => (StatusCode::UNPROCESSABLE_ENTITY, "invalid"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            Self::TermsRequired(_) => (StatusCode::CONFLICT, "terms_required"),
            Self::Suspended => (StatusCode::FORBIDDEN, "suspended"),
            Self::StepUpRequired => (StatusCode::FORBIDDEN, "step_up_required"),
            Self::PasskeyRequired(_) => (StatusCode::FORBIDDEN, "passkey_required"),
            Self::ChallengeRequired => (StatusCode::CONFLICT, "challenge_required"),
            Self::TooFast { .. } | Self::NotYet { .. } => {
                (StatusCode::TOO_MANY_REQUESTS, "rate_limited")
            }
            Self::Refused(_) => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Internal(err) => {
                tracing::error!(error = ?err, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        };
        let message = match &self {
            Self::Internal(_) => "something went wrong".to_string(),
            other => other.to_string(),
        };
        let mut body = json!({ "error": { "code": code, "message": message } });
        if let Self::Invalid { field, .. } = &self {
            body["error"]["field"] = json!(field);
        }
        // Whether to confirm with a passkey ("confirm") or add one ("add").
        if let Self::PasskeyRequired(has) = &self {
            body["error"]["field"] = json!(if *has { "confirm" } else { "add" });
        }
        // The document to accept, so the UI can show it and try again.
        if let Self::TermsRequired(kind) = &self {
            body["error"]["field"] = json!(kind);
        }
        let mut res = (status, Json(body)).into_response();
        if let Self::TooFast { retry_after, .. } | Self::NotYet { retry_after, .. } = &self
            && let Ok(v) = axum::http::HeaderValue::from_str(&retry_after.to_string())
        {
            res.headers_mut().insert(axum::http::header::RETRY_AFTER, v);
        }
        res
    }
}

/// True when a database error is a unique-constraint violation.
pub fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}
