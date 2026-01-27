//! Database layer for usenet-dl
//!
//! Handles SQLite persistence for downloads, articles, passwords, and history.
//!
//! ## Submodules
//!
//! Methods on [`Database`] are organized by domain:
//! - [`migrations`] — Database lifecycle, schema migrations
//! - [`downloads`] — Download queue CRUD
//! - [`articles`] — Article-level tracking for resume support
//! - [`passwords`] — Password cache for archive extraction
//! - [`duplicates`] — Duplicate detection queries
//! - [`history`] — History management
//! - [`state`] — Runtime state (shutdown tracking, NZB processing, RSS seen)
//! - [`rss`] — RSS feed CRUD

use crate::types::{HistoryEntry, Status};
use sqlx::{sqlite::SqlitePool, FromRow};
use std::path::PathBuf;

mod articles;
mod downloads;
mod duplicates;
mod history;
mod migrations;
mod passwords;
mod rss;
mod state;

/// New download to be inserted into the database
#[derive(Debug, Clone)]
pub struct NewDownload {
    pub name: String,
    pub nzb_path: String,
    pub nzb_meta_name: Option<String>,
    pub nzb_hash: Option<String>,
    pub job_name: Option<String>,
    pub category: Option<String>,
    pub destination: String,
    pub post_process: i32,
    pub priority: i32,
    pub status: i32,
    pub size_bytes: i64,
}

/// Download record from database
#[derive(Debug, Clone, FromRow)]
pub struct Download {
    pub id: i64,
    pub name: String,
    pub nzb_path: String,
    pub nzb_meta_name: Option<String>,
    pub nzb_hash: Option<String>,
    pub job_name: Option<String>,
    pub category: Option<String>,
    pub destination: String,
    pub post_process: i32,
    pub priority: i32,
    pub status: i32,
    pub progress: f32,
    pub speed_bps: i64,
    pub size_bytes: i64,
    pub downloaded_bytes: i64,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
}

/// New article to be inserted into the database
#[derive(Debug, Clone)]
pub struct NewArticle {
    pub download_id: crate::types::DownloadId,
    pub message_id: String,
    pub segment_number: i32,
    pub size_bytes: i64,
}

/// Article record from database
#[derive(Debug, Clone, FromRow)]
pub struct Article {
    pub id: i64,
    pub download_id: i64,
    pub message_id: String,
    pub segment_number: i32,
    pub size_bytes: i64,
    pub status: i32,
    pub downloaded_at: Option<i64>,
}

/// Article status constants
pub mod article_status {
    pub const PENDING: i32 = 0;
    pub const DOWNLOADED: i32 = 1;
    pub const FAILED: i32 = 2;
}

/// New history entry to be inserted into the database
#[derive(Debug, Clone)]
pub struct NewHistoryEntry {
    pub name: String,
    pub category: Option<String>,
    pub destination: Option<PathBuf>,
    pub status: i32,
    pub size_bytes: u64,
    pub download_time_secs: i64,
    pub completed_at: i64,
}

/// History record from database (raw from SQLite)
#[derive(Debug, Clone, FromRow)]
pub struct HistoryRow {
    pub id: i64,
    pub name: String,
    pub category: Option<String>,
    pub destination: Option<String>,
    pub status: i32,
    pub size_bytes: i64,
    pub download_time_secs: i64,
    pub completed_at: i64,
}

impl From<HistoryRow> for HistoryEntry {
    fn from(row: HistoryRow) -> Self {
        use chrono::{TimeZone, Utc};
        use std::time::Duration;

        HistoryEntry {
            id: row.id,
            name: row.name,
            category: row.category,
            destination: row.destination.map(PathBuf::from),
            status: Status::from_i32(row.status),
            size_bytes: row.size_bytes as u64,
            download_time: Duration::from_secs(row.download_time_secs as u64),
            completed_at: Utc
                .timestamp_opt(row.completed_at, 0)
                .single()
                .unwrap_or_else(Utc::now),
        }
    }
}

/// RSS feed record from database
#[derive(Debug, Clone, FromRow)]
pub struct RssFeed {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub check_interval_secs: i64,
    pub category: Option<String>,
    pub auto_download: i32,
    pub priority: i32,
    pub enabled: i32,
    pub last_check: Option<i64>,
    pub last_error: Option<String>,
    pub created_at: i64,
}

/// RSS filter record from database
#[derive(Debug, Clone, FromRow)]
pub struct RssFilterRow {
    pub id: i64,
    pub feed_id: i64,
    pub name: String,
    pub include_patterns: Option<String>,
    pub exclude_patterns: Option<String>,
    pub min_size: Option<i64>,
    pub max_size: Option<i64>,
    pub max_age_secs: Option<i64>,
}

/// Database handle for usenet-dl
pub struct Database {
    pool: SqlitePool,
}

#[cfg(test)]
mod tests;
