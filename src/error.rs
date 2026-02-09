//! Error types for usenet-dl
//!
//! This module provides comprehensive error handling for the library, including:
//! - Domain-specific error types (Download, PostProcess, Config, etc.)
//! - HTTP status code mapping for API integration
//! - Structured error responses with machine-readable error codes
//! - Context information (stage, file path, download ID, etc.)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use utoipa::ToSchema;

/// Result type alias for usenet-dl operations
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for usenet-dl
///
/// This is the primary error type used throughout the library. Each variant includes
/// contextual information to help diagnose issues.
#[derive(Debug, Error)]
pub enum Error {
    /// Configuration error with context about which setting is invalid
    #[error("configuration error: {message}")]
    Config {
        /// Human-readable error message describing the configuration issue
        message: String,
        /// The configuration key that caused the error (e.g., "download_dir")
        key: Option<String>,
    },

    /// Database operation failed
    #[error("database error: {0}")]
    Database(#[from] DatabaseError),

    /// SQLx database error
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// NNTP protocol or connection error
    #[error("NNTP error: {0}")]
    Nntp(String),

    /// Download-related error
    #[error("download error: {0}")]
    Download(#[from] DownloadError),

    /// Post-processing error (verify, repair, extract, etc.)
    #[error("post-processing error: {0}")]
    PostProcess(#[from] PostProcessError),

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

    /// Network error
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// API server error
    #[error("API server error: {0}")]
    ApiServerError(String),

    /// Folder watching error
    #[error("folder watch error: {0}")]
    FolderWatch(String),

    /// Duplicate download detected
    #[error("duplicate download: {0}")]
    Duplicate(String),

    /// Insufficient disk space
    #[error("insufficient disk space: need {required} bytes, have {available} bytes")]
    InsufficientSpace {
        /// Number of bytes required for the operation
        required: u64,
        /// Number of bytes currently available on disk
        available: u64,
    },

    /// Failed to check disk space
    #[error("failed to check disk space: {0}")]
    DiskSpaceCheckFailed(String),

    /// External tool execution failed (par2, unrar, etc.)
    #[error("external tool error: {0}")]
    ExternalTool(String),

    /// Operation not supported (missing binary, not implemented, etc.)
    #[error("not supported: {0}")]
    NotSupported(String),

    /// Other error
    #[error("{0}")]
    Other(String),
}

/// Database-related errors
#[derive(Debug, Error)]
pub enum DatabaseError {
    /// Failed to connect to database
    #[error("failed to connect to database: {0}")]
    ConnectionFailed(String),

    /// Failed to run migrations
    #[error("failed to run migrations: {0}")]
    MigrationFailed(String),

    /// Query failed
    #[error("query failed: {0}")]
    QueryFailed(String),

    /// Record not found
    #[error("record not found: {0}")]
    NotFound(String),

    /// Constraint violation (e.g., duplicate key)
    #[error("constraint violation: {0}")]
    ConstraintViolation(String),
}

/// Download-related errors
#[derive(Debug, Error)]
pub enum DownloadError {
    /// Download not found in queue or database
    #[error("download {id} not found")]
    NotFound {
        /// The download ID that was not found
        id: i64,
    },

    /// Download files not found on disk
    #[error("download {id} files not found at {path}")]
    FilesNotFound {
        /// The download ID whose files were not found
        id: i64,
        /// The path where the files were expected to be
        path: PathBuf,
    },

    /// Download already in requested state
    #[error("download {id} is already {state}")]
    AlreadyInState {
        /// The download ID that is already in the requested state
        id: i64,
        /// The current state (e.g., "paused", "completed")
        state: String,
    },

    /// Cannot perform operation in current state
    #[error("cannot {operation} download {id} in state {current_state}")]
    InvalidState {
        /// The download ID that is in an invalid state for the operation
        id: i64,
        /// The operation that was attempted (e.g., "pause", "resume", "retry")
        operation: String,
        /// The current state that prevents the operation (e.g., "downloading", "completed")
        current_state: String,
    },

    /// Insufficient disk space to start download
    #[error("insufficient disk space: need {required} bytes, have {available} bytes")]
    InsufficientSpace {
        /// Number of bytes required for the download
        required: u64,
        /// Number of bytes currently available on disk
        available: u64,
    },

}

/// Post-processing errors (PAR2 verify, repair, extraction, etc.)
#[derive(Debug, Error)]
pub enum PostProcessError {
    /// PAR2 verification failed
    #[error("PAR2 verification failed for download {id}: {reason}")]
    VerificationFailed {
        /// The download ID for which verification failed
        id: i64,
        /// The reason verification failed
        reason: String,
    },

    /// PAR2 repair failed
    #[error("PAR2 repair failed for download {id}: {reason}")]
    RepairFailed {
        /// The download ID for which repair failed
        id: i64,
        /// The reason repair failed
        reason: String,
    },

    /// Archive extraction failed
    #[error("extraction failed for {archive}: {reason}")]
    ExtractionFailed {
        /// The archive file that failed to extract
        archive: PathBuf,
        /// The reason extraction failed
        reason: String,
    },

    /// Wrong password for encrypted archive
    #[error("wrong password for encrypted archive {archive}")]
    WrongPassword {
        /// The encrypted archive that could not be opened
        archive: PathBuf,
    },

    /// All passwords failed for archive extraction
    #[error("all {count} passwords failed for archive {archive}")]
    AllPasswordsFailed {
        /// The encrypted archive that could not be opened
        archive: PathBuf,
        /// The number of passwords that were tried
        count: usize,
    },

    /// No passwords available for encrypted archive
    #[error("no passwords available for encrypted archive {archive}")]
    NoPasswordsAvailable {
        /// The encrypted archive that requires a password
        archive: PathBuf,
    },

    /// File move/rename failed
    #[error("failed to move {source_path} to {dest_path}: {reason}")]
    MoveFailed {
        /// The source path of the file being moved
        source_path: PathBuf,
        /// The destination path where the file should be moved
        dest_path: PathBuf,
        /// The reason the move failed
        reason: String,
    },

    /// File collision at destination
    #[error("file collision at {path}: {reason}")]
    FileCollision {
        /// The path where the collision occurred
        path: PathBuf,
        /// The reason for the collision (e.g., "file already exists")
        reason: String,
    },

    /// Cleanup failed (non-fatal, usually logged as warning)
    #[error("cleanup failed for download {id}: {reason}")]
    CleanupFailed {
        /// The download ID for which cleanup failed
        id: i64,
        /// The reason cleanup failed
        reason: String,
    },

    /// Invalid path encountered during post-processing
    #[error("invalid path {path}: {reason}")]
    InvalidPath {
        /// The invalid path that was encountered
        path: PathBuf,
        /// The reason the path is invalid
        reason: String,
    },
}

/// API error response format
///
/// This structure is returned by API endpoints when an error occurs.
/// It follows a standard format with machine-readable error codes,
/// human-readable messages, and optional contextual details.
///
/// # Example JSON Response
///
/// ```json
/// {
///   "error": {
///     "code": "not_found",
///     "message": "Download 123 not found",
///     "details": {
///       "download_id": 123
///     }
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiError {
    /// The error details
    pub error: ErrorDetail,
}

/// Detailed error information for API responses
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorDetail {
    /// Machine-readable error code (e.g., "not_found", "validation_error")
    ///
    /// Clients can use this for programmatic error handling.
    pub code: String,

    /// Human-readable error message
    ///
    /// This is suitable for displaying to end users.
    pub message: String,

    /// Optional additional context about the error
    ///
    /// This can include fields like download_id, file paths, validation errors, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    /// Create a new API error with code and message
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
                details: None,
            },
        }
    }

    /// Create an API error with additional details
    pub fn with_details(
        code: impl Into<String>,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self {
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
                details: Some(details),
            },
        }
    }

    /// Create a "not found" error
    pub fn not_found(resource: impl Into<String>) -> Self {
        Self::new("not_found", format!("{} not found", resource.into()))
    }

    /// Create a "validation error" error
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new("validation_error", message)
    }

    /// Create a "conflict" error
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new("conflict", message)
    }

    /// Create an "internal server error"
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal_error", message)
    }

    /// Create an "unauthorized" error
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new("unauthorized", message)
    }

    /// Create a "service unavailable" error
    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new("service_unavailable", message)
    }
}

/// Convert errors to HTTP status codes for API responses
///
/// This trait maps domain errors to appropriate HTTP status codes.
pub trait ToHttpStatus {
    /// Get the HTTP status code for this error
    fn status_code(&self) -> u16;

    /// Get the machine-readable error code
    fn error_code(&self) -> &str;
}

impl ToHttpStatus for Error {
    fn status_code(&self) -> u16 {
        match self {
            // 400 Bad Request - Client error (invalid input)
            Error::Config { .. } => 400,
            Error::InvalidNzb(_) => 422, // Unprocessable Entity
            Error::Duplicate(_) => 409,  // Conflict

            // 404 Not Found
            Error::NotFound(_) => 404,
            Error::Download(DownloadError::NotFound { .. }) => 404,
            Error::Download(DownloadError::FilesNotFound { .. }) => 404,

            // 409 Conflict - Resource already in desired state
            Error::Download(DownloadError::AlreadyInState { .. }) => 409,
            Error::Download(DownloadError::InvalidState { .. }) => 409,

            // 422 Unprocessable Entity - Semantic errors
            Error::PostProcess(_) => 422,
            Error::Download(DownloadError::InsufficientSpace { .. }) => 422,
            Error::InsufficientSpace { .. } => 422,

            // 500 Internal Server Error - Server-side issues
            Error::Database(_) => 500,
            Error::Sqlx(_) => 500,
            Error::Io(_) => 500,
            Error::ApiServerError(_) => 500,
            Error::FolderWatch(_) => 500,
            Error::DiskSpaceCheckFailed(_) => 500,
            Error::Other(_) => 500,

            // 502 Bad Gateway - External service errors
            Error::Nntp(_) => 502,
            Error::Network(_) => 502,

            // 503 Service Unavailable
            Error::ShuttingDown => 503,
            Error::ExternalTool(_) => 503,

            // 501 Not Implemented - Feature not supported
            Error::NotSupported(_) => 501,

            // 500 for serialization errors
            Error::Serialization(_) => 500,
        }
    }

    fn error_code(&self) -> &str {
        match self {
            Error::Config { .. } => "config_error",
            Error::Database(_) => "database_error",
            Error::Sqlx(_) => "database_error",
            Error::Nntp(_) => "nntp_error",
            Error::Download(e) => match e {
                DownloadError::NotFound { .. } => "download_not_found",
                DownloadError::FilesNotFound { .. } => "files_not_found",
                DownloadError::AlreadyInState { .. } => "already_in_state",
                DownloadError::InvalidState { .. } => "invalid_state",
                DownloadError::InsufficientSpace { .. } => "insufficient_space",
            },
            Error::PostProcess(e) => match e {
                PostProcessError::VerificationFailed { .. } => "verification_failed",
                PostProcessError::RepairFailed { .. } => "repair_failed",
                PostProcessError::ExtractionFailed { .. } => "extraction_failed",
                PostProcessError::WrongPassword { .. } => "wrong_password",
                PostProcessError::AllPasswordsFailed { .. } => "all_passwords_failed",
                PostProcessError::NoPasswordsAvailable { .. } => "no_passwords_available",
                PostProcessError::MoveFailed { .. } => "move_failed",
                PostProcessError::FileCollision { .. } => "file_collision",
                PostProcessError::CleanupFailed { .. } => "cleanup_failed",
                PostProcessError::InvalidPath { .. } => "invalid_path",
            },
            Error::InvalidNzb(_) => "invalid_nzb",
            Error::Io(_) => "io_error",
            Error::NotFound(_) => "not_found",
            Error::ShuttingDown => "shutting_down",
            Error::Network(_) => "network_error",
            Error::Serialization(_) => "serialization_error",
            Error::ApiServerError(_) => "api_server_error",
            Error::FolderWatch(_) => "folder_watch_error",
            Error::Duplicate(_) => "duplicate",
            Error::InsufficientSpace { .. } => "insufficient_space",
            Error::DiskSpaceCheckFailed(_) => "disk_space_check_failed",
            Error::ExternalTool(_) => "external_tool_error",
            Error::NotSupported(_) => "not_supported",
            Error::Other(_) => "internal_error",
        }
    }
}

impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        let code = error.error_code().to_string();
        let message = error.to_string();

        // Add contextual details for specific error types
        let details = match &error {
            Error::Download(DownloadError::NotFound { id }) => Some(serde_json::json!({
                "download_id": id,
            })),
            Error::Download(DownloadError::FilesNotFound { id, path }) => Some(serde_json::json!({
                "download_id": id,
                "path": path,
            })),
            Error::Download(DownloadError::AlreadyInState { id, state }) => {
                Some(serde_json::json!({
                    "download_id": id,
                    "state": state,
                }))
            }
            Error::Download(DownloadError::InvalidState {
                id,
                operation,
                current_state,
            }) => Some(serde_json::json!({
                "download_id": id,
                "operation": operation,
                "current_state": current_state,
            })),
            Error::Download(DownloadError::InsufficientSpace {
                required,
                available,
            }) => Some(serde_json::json!({
                "required_bytes": required,
                "available_bytes": available,
            })),
            Error::InsufficientSpace {
                required,
                available,
            } => Some(serde_json::json!({
                "required_bytes": required,
                "available_bytes": available,
            })),
            Error::PostProcess(PostProcessError::FileCollision { path, .. }) => {
                Some(serde_json::json!({
                    "path": path,
                }))
            }
            Error::PostProcess(PostProcessError::WrongPassword { archive }) => {
                Some(serde_json::json!({
                    "archive": archive,
                }))
            }
            Error::PostProcess(PostProcessError::AllPasswordsFailed { archive, count }) => {
                Some(serde_json::json!({
                    "archive": archive,
                    "password_count": count,
                }))
            }
            _ => None,
        };

        ApiError {
            error: ErrorDetail {
                code,
                message,
                details,
            },
        }
    }
}
