//! The Slop client's core: talking to the API, signing in through the system browser, and
//! installing, verifying and launching apps (docs/ARCHITECTURE.md §3, §6.3; docs/SECURITY.md §5).
//!
//! It has no UI and no Tauri dependency, so it can be tested on its own; `slop-desktop` exposes
//! it to the UI as a small set of commands.

pub mod api;
pub mod chat;
pub mod library;
pub mod sandbox;
pub mod saves;
pub mod signin;

pub use reqwest::Method;
pub use slop_manifest;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("can't reach Slop: {0}")]
    Network(String),
    #[error("{message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("you're signed out")]
    SignedOut,
    #[error("verification failed: {0}")]
    Verify(String),
    #[error("{0}")]
    Install(String),
    #[error("this version was withdrawn by Slop: {0}")]
    Revoked(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Network(err.without_url().to_string())
    }
}

/// A random URL-safe token (256 bits).
pub(crate) fn random_token() -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}
