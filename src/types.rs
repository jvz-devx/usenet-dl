//! Core types for usenet-dl

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

use crate::config::PostProcess;

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

impl Status {
    /// Convert integer status code to Status enum
    pub fn from_i32(status: i32) -> Self {
        match status {
            0 => Status::Queued,
            1 => Status::Downloading,
            2 => Status::Paused,
            3 => Status::Processing,
            4 => Status::Complete,
            5 => Status::Failed,
            _ => Status::Failed, // Default to Failed for unknown status
        }
    }

    /// Convert Status enum to integer status code
    pub fn to_i32(&self) -> i32 {
        match self {
            Status::Queued => 0,
            Status::Downloading => 1,
            Status::Paused => 2,
            Status::Processing => 3,
            Status::Complete => 4,
            Status::Failed => 5,
        }
    }
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

/// Information about a download in the queue
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadInfo {
    /// Unique download identifier
    pub id: DownloadId,

    /// Download name (from NZB filename)
    pub name: String,

    /// Category (if assigned)
    pub category: Option<String>,

    /// Current status
    pub status: Status,

    /// Progress percentage (0.0 to 100.0)
    pub progress: f32,

    /// Current download speed in bytes per second
    pub speed_bps: u64,

    /// Total size in bytes
    pub size_bytes: u64,

    /// Downloaded bytes so far
    pub downloaded_bytes: u64,

    /// Estimated time to completion in seconds (None if unknown)
    pub eta_seconds: Option<u64>,

    /// Download priority
    pub priority: Priority,

    /// When the download was added to the queue
    pub created_at: DateTime<Utc>,

    /// When the download started (None if not started yet)
    pub started_at: Option<DateTime<Utc>>,
}

/// Options for adding a download to the queue
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DownloadOptions {
    /// Category to assign (None = use default category)
    #[serde(default)]
    pub category: Option<String>,

    /// Override default destination directory
    #[serde(default)]
    pub destination: Option<PathBuf>,

    /// Override default post-processing mode
    #[serde(default)]
    pub post_process: Option<PostProcess>,

    /// Download priority
    #[serde(default)]
    pub priority: Priority,

    /// Password for this specific download (high priority)
    #[serde(default)]
    pub password: Option<String>,
}

/// Historical download record
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unique download identifier
    pub id: i64,

    /// Download name
    pub name: String,

    /// Category (if assigned)
    pub category: Option<String>,

    /// Final destination path (if completed successfully)
    pub destination: Option<PathBuf>,

    /// Final status (Complete or Failed)
    pub status: Status,

    /// Total size in bytes
    pub size_bytes: u64,

    /// Time spent downloading (not including queue wait time)
    pub download_time: Duration,

    /// When the download completed (successfully or failed)
    pub completed_at: DateTime<Utc>,
}
