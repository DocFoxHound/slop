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
    #[error("accept the current {} first", crate::legal::title(.0))]
    TermsRequired(&'static str),
    #[error("your account is suspended: see the decision on your account page")]
    Suspended,
    /// A sensitive action (SECURITY §4.3) needs a sign-in from the last few minutes.
    #[error("sign in again to confirm it's you: this protects where your money goes")]
    StepUpRequired,
    /// Too many unfinished checkouts: solve a proof-of-work challenge first (challenge.rs).
    #[error("one more step before trying again: your browser needs to do a quick check")]
    ChallengeRequired,
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
            Self::ChallengeRequired => (StatusCode::CONFLICT, "challenge_required"),
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
        // The document to accept, so the UI can show it and try again.
        if let Self::TermsRequired(kind) = &self {
            body["error"]["field"] = json!(kind);
        }
        (status, Json(body)).into_response()
    }
}

/// True when a database error is a unique-constraint violation.
pub fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}
