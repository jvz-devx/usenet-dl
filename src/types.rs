//! Core types for usenet-dl

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Unique identifier for a download
pub type DownloadId = i64;

/// Download status
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Queued and waiting to start
    Queued,
    /// Currently downloading
    Downloading,
    /// Paused by user
    Paused,
    /// Post-processing (verify/repair/extract)
    Processing,
    /// Successfully completed
    Complete,
    /// Failed with error
    Failed,
}

/// Download priority
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Low priority (-1)
    Low = -1,
    /// Normal priority (0)
    Normal = 0,
    /// High priority (1)
    High = 1,
    /// Force start immediately (2)
    Force = 2,
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

/// Post-processing stage
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// Download stage
    Download,
    /// PAR2 verification
    Verify,
    /// PAR2 repair
    Repair,
    /// Archive extraction
    Extract,
    /// Move to final destination
    Move,
    /// Cleanup intermediate files
    Cleanup,
}

/// Event emitted during download lifecycle
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Download added to queue
    Queued {
        /// Download ID
        id: DownloadId,
        /// Download name
        name: String,
    },

    /// Download removed from queue
    Removed {
        /// Download ID
        id: DownloadId,
    },

    /// Download progress update
    Downloading {
        /// Download ID
        id: DownloadId,
        /// Progress percentage (0.0 to 100.0)
        percent: f32,
        /// Current speed in bytes per second
        speed_bps: u64,
    },

    /// Download completed (starting post-processing)
    DownloadComplete {
        /// Download ID
        id: DownloadId,
    },

    /// Download failed
    DownloadFailed {
        /// Download ID
        id: DownloadId,
        /// Error message
        error: String,
    },

    /// PAR2 verification started
    Verifying {
        /// Download ID
        id: DownloadId,
    },

    /// PAR2 verification completed
    VerifyComplete {
        /// Download ID
        id: DownloadId,
        /// Whether files are damaged
        damaged: bool,
    },

    /// PAR2 repair started
    Repairing {
        /// Download ID
        id: DownloadId,
        /// Blocks needed for repair
        blocks_needed: u32,
        /// Blocks available
        blocks_available: u32,
    },

    /// PAR2 repair completed
    RepairComplete {
        /// Download ID
        id: DownloadId,
        /// Whether repair was successful
        success: bool,
    },

    /// Archive extraction started
    Extracting {
        /// Download ID
        id: DownloadId,
        /// Archive filename
        archive: String,
        /// Extraction progress (0.0 to 100.0)
        percent: f32,
    },

    /// Archive extraction completed
    ExtractComplete {
        /// Download ID
        id: DownloadId,
    },

    /// Moving files to destination
    Moving {
        /// Download ID
        id: DownloadId,
        /// Destination path
        destination: PathBuf,
    },

    /// Cleaning up intermediate files
    Cleaning {
        /// Download ID
        id: DownloadId,
    },

    /// Download fully complete
    Complete {
        /// Download ID
        id: DownloadId,
        /// Final path
        path: PathBuf,
    },

    /// Download failed at some stage
    Failed {
        /// Download ID
        id: DownloadId,
        /// Stage where failure occurred
        stage: Stage,
        /// Error message
        error: String,
        /// Whether files were kept
        files_kept: bool,
    },

    /// Speed limit changed
    SpeedLimitChanged {
        /// New limit in bytes per second (None = unlimited)
        limit_bps: Option<u64>,
    },

    /// Queue paused
    QueuePaused,

    /// Queue resumed
    QueueResumed,

    /// Webhook delivery failed
    WebhookFailed {
        /// Webhook URL
        url: String,
        /// Error message
        error: String,
    },

    /// Script execution failed
    ScriptFailed {
        /// Script path
        script: PathBuf,
        /// Exit code (if available)
        exit_code: Option<i32>,
    },
}
