//! Library-wide errors, independent of search orchestration.
use crate::model::Engine;
use thiserror::Error;

/// Library-wide failures, also re-exported from the legacy `search` module.
#[derive(Debug, Error)]
pub enum KestrelError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    Search(String),
    #[error("search deadline exceeded")]
    SearchDeadline,
    #[error("{engine} response exceeds {limit_bytes} decoded bytes (HTTP {status})")]
    ProviderResponseTooLarge {
        engine: Engine,
        limit_bytes: usize,
        status: u16,
    },
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Yahoo request failed: {0}")]
    Yahoo(#[from] primp::Error),
    #[error("Bing request failed: {0}")]
    Bing(primp::Error),
    #[error("failed to initialize HTTP client: {0}")]
    Client(String),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot start blocking search from an asynchronous runtime")]
    NestedRuntime,
}
