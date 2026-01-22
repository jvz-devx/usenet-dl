//! Error types for usenet-dl

use thiserror::Error;

/// Result type alias for usenet-dl operations
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for usenet-dl
#[derive(Debug, Error)]
pub enum Error {
    /// Configuration error
    #[error("configuration error: {0}")]
    Config(String),

    /// Database error
    #[error("database error: {0}")]
    Database(String),

    /// SQLx database error
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// NNTP error
    #[error("NNTP error: {0}")]
    Nntp(String),

    /// Invalid NZB file
    #[error("invalid NZB: {0}")]
    InvalidNzb(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Download not found
    #[error("download not found: {0}")]
    NotFound(String),

    /// Shutdown in progress - not accepting new downloads
    #[error("shutdown in progress: not accepting new downloads")]
    ShuttingDown,

    /// Archive extraction error
    #[error("extraction error: {0}")]
    Extraction(String),

    /// Wrong password for encrypted archive
    #[error("wrong password for encrypted archive")]
    WrongPassword,

    /// All passwords failed for archive extraction
    #[error("all passwords failed for archive extraction")]
    AllPasswordsFailed,

    /// No passwords available for encrypted archive
    #[error("no passwords available for encrypted archive")]
    NoPasswordsAvailable,

    /// Extraction failed with specific reason
    #[error("extraction failed: {0}")]
    ExtractionFailed(String),

    /// Network error
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Other error
    #[error("{0}")]
    Other(String),
}
