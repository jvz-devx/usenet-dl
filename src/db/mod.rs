//! Database layer for usenet-dl
//!
//! Handles SQLite persistence for downloads, articles, passwords, and history.

use crate::{error::DatabaseError, types::{DownloadId, HistoryEntry, Status}, Error, Result};
use sqlx::{sqlite::SqlitePool, FromRow, SqliteConnection};
use std::path::{Path, PathBuf};

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
    pub download_id: DownloadId,
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
        use std::time::Duration;
        use chrono::{Utc, TimeZone};

        HistoryEntry {
            id: row.id,
            name: row.name,
            category: row.category,
            destination: row.destination.map(PathBuf::from),
            status: Status::from_i32(row.status),
            size_bytes: row.size_bytes as u64,
            download_time: Duration::from_secs(row.download_time_secs as u64),
            completed_at: Utc.timestamp_opt(row.completed_at, 0).unwrap(),
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

impl Database {
    /// Create a new database connection
    ///
    /// Creates the database file if it doesn't exist and runs migrations.
    pub async fn new(path: &Path) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Database(DatabaseError::ConnectionFailed(format!("Failed to create database directory: {}", e))))?;
        }

        // Connect to database
        let connection_string = format!("sqlite:{}?mode=rwc", path.display());
        let pool = SqlitePool::connect(&connection_string)
            .await
            .map_err(|e| Error::Database(DatabaseError::ConnectionFailed(format!("Failed to connect to database: {}", e))))?;

        let db = Self { pool };

        // Run migrations
        db.run_migrations().await?;

        Ok(db)
    }

    /// Run database migrations
    async fn run_migrations(&self) -> Result<()> {
        let mut conn = self.pool.acquire().await
            .map_err(|e| Error::Database(DatabaseError::ConnectionFailed(format!("Failed to acquire connection: {}", e))))?;

        // Create schema version table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create schema_version table: {}", e))))?;

        // Check current version
        let current_version: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(version) FROM schema_version"
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to query schema version: {}", e))))?;

        let current_version = current_version.unwrap_or(0);

        // Apply migrations
        if current_version < 1 {
            Self::migrate_v1(&mut conn).await?;
        }
        if current_version < 2 {
            Self::migrate_v2(&mut conn).await?;
        }
        if current_version < 3 {
            Self::migrate_v3(&mut conn).await?;
        }

        Ok(())
    }

    /// Migration v1: Create initial schema
    async fn migrate_v1(conn: &mut SqliteConnection) -> Result<()> {
        tracing::info!("Applying database migration v1");

        // Downloads table
        sqlx::query(
            r#"
            CREATE TABLE downloads (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                nzb_path TEXT NOT NULL,
                nzb_meta_name TEXT,
                nzb_hash TEXT,
                job_name TEXT,
                category TEXT,
                destination TEXT NOT NULL,
                post_process INTEGER NOT NULL,
                priority INTEGER NOT NULL DEFAULT 0,
                status INTEGER NOT NULL DEFAULT 0,
                progress REAL DEFAULT 0.0,
                speed_bps INTEGER DEFAULT 0,
                size_bytes INTEGER DEFAULT 0,
                downloaded_bytes INTEGER DEFAULT 0,
                error_message TEXT,
                created_at INTEGER NOT NULL,
                started_at INTEGER,
                completed_at INTEGER
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create downloads table: {}", e))))?;

        // Indexes for downloads
        sqlx::query("CREATE INDEX idx_downloads_status ON downloads(status)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create index: {}", e))))?;

        sqlx::query("CREATE INDEX idx_downloads_priority ON downloads(priority DESC, created_at ASC)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create index: {}", e))))?;

        sqlx::query("CREATE INDEX idx_downloads_nzb_hash ON downloads(nzb_hash)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create index: {}", e))))?;

        sqlx::query("CREATE INDEX idx_downloads_job_name ON downloads(job_name)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create index: {}", e))))?;

        // Download articles table (for resume support)
        sqlx::query(
            r#"
            CREATE TABLE download_articles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
                message_id TEXT NOT NULL,
                segment_number INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL,
                status INTEGER NOT NULL DEFAULT 0,
                downloaded_at INTEGER,
                UNIQUE(download_id, message_id)
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create download_articles table: {}", e))))?;

        // Indexes for download_articles
        sqlx::query("CREATE INDEX idx_articles_download ON download_articles(download_id)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create index: {}", e))))?;

        sqlx::query("CREATE INDEX idx_articles_status ON download_articles(download_id, status)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create index: {}", e))))?;

        // Password cache table
        sqlx::query(
            r#"
            CREATE TABLE passwords (
                download_id INTEGER PRIMARY KEY REFERENCES downloads(id) ON DELETE CASCADE,
                correct_password TEXT NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create passwords table: {}", e))))?;

        // Processed NZBs table (for watch folder tracking)
        sqlx::query(
            r#"
            CREATE TABLE processed_nzbs (
                path TEXT PRIMARY KEY,
                processed_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create processed_nzbs table: {}", e))))?;

        // History table
        sqlx::query(
            r#"
            CREATE TABLE history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                category TEXT,
                destination TEXT,
                status INTEGER NOT NULL,
                size_bytes INTEGER,
                download_time_secs INTEGER,
                completed_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create history table: {}", e))))?;

        // Index for history
        sqlx::query("CREATE INDEX idx_history_completed ON history(completed_at DESC)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create index: {}", e))))?;

        // Record migration
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (1, ?)")
            .bind(now)
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to record migration: {}", e))))?;

        tracing::info!("Database migration v1 complete");

        Ok(())
    }

    /// Migration v2: Add runtime state table for shutdown tracking
    async fn migrate_v2(conn: &mut SqliteConnection) -> Result<()> {
        tracing::info!("Applying database migration v2");

        // Runtime state table for tracking clean/unclean shutdown
        sqlx::query(
            r#"
            CREATE TABLE runtime_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create runtime_state table: {}", e))))?;

        // Initialize shutdown state as unclean (will be set to clean on proper startup)
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            r#"
            INSERT INTO runtime_state (key, value, updated_at)
            VALUES ('clean_shutdown', 'false', ?)
            "#
        )
        .bind(now)
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to initialize runtime_state: {}", e))))?;

        // Record migration
        sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (2, ?)")
            .bind(now)
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to record migration: {}", e))))?;

        tracing::info!("Database migration v2 complete");

        Ok(())
    }

    /// Migration v3: Add RSS feed tables
    async fn migrate_v3(conn: &mut SqliteConnection) -> Result<()> {
        tracing::info!("Applying database migration v3");

        // RSS feeds table
        sqlx::query(
            r#"
            CREATE TABLE rss_feeds (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                url TEXT NOT NULL,
                check_interval_secs INTEGER NOT NULL DEFAULT 900,
                category TEXT,
                auto_download INTEGER NOT NULL DEFAULT 1,
                priority INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 1,
                last_check INTEGER,
                last_error TEXT,
                created_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create rss_feeds table: {}", e))))?;

        // RSS filters table (per feed)
        sqlx::query(
            r#"
            CREATE TABLE rss_filters (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                feed_id INTEGER NOT NULL REFERENCES rss_feeds(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                include_patterns TEXT,
                exclude_patterns TEXT,
                min_size INTEGER,
                max_size INTEGER,
                max_age_secs INTEGER
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create rss_filters table: {}", e))))?;

        // RSS seen items table (prevent re-downloading)
        sqlx::query(
            r#"
            CREATE TABLE rss_seen (
                feed_id INTEGER NOT NULL REFERENCES rss_feeds(id) ON DELETE CASCADE,
                guid TEXT NOT NULL,
                seen_at INTEGER NOT NULL,
                PRIMARY KEY (feed_id, guid)
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create rss_seen table: {}", e))))?;

        // Index for rss_seen
        sqlx::query("CREATE INDEX idx_rss_seen_feed ON rss_seen(feed_id)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to create index: {}", e))))?;

        // Record migration
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (3, ?)")
            .bind(now)
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(DatabaseError::MigrationFailed(format!("Failed to record migration: {}", e))))?;

        tracing::info!("Database migration v3 complete");

        Ok(())
    }

    /// Close the database connection
    pub async fn close(self) {
        self.pool.close().await;
    }

    /// Get the underlying connection pool
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // CRUD operations for downloads table

    /// Insert a new download record
    pub async fn insert_download(&self, download: &NewDownload) -> Result<DownloadId> {
        let now = chrono::Utc::now().timestamp();

        let result = sqlx::query(
            r#"
            INSERT INTO downloads (
                name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&download.name)
        .bind(&download.nzb_path)
        .bind(&download.nzb_meta_name)
        .bind(&download.nzb_hash)
        .bind(&download.job_name)
        .bind(&download.category)
        .bind(&download.destination)
        .bind(download.post_process as i32)
        .bind(download.priority as i32)
        .bind(download.status as i32)
        .bind(0.0f32) // progress
        .bind(0i64) // speed_bps
        .bind(download.size_bytes)
        .bind(0i64) // downloaded_bytes
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to insert download: {}", e))))?;

        Ok(result.last_insert_rowid())
    }

    /// Get a download by ID
    pub async fn get_download(&self, id: DownloadId) -> Result<Option<Download>> {
        let row = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at
            FROM downloads
            WHERE id = ?
            "#
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to get download: {}", e))))?;

        Ok(row)
    }

    /// List all downloads
    pub async fn list_downloads(&self) -> Result<Vec<Download>> {
        let rows = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at
            FROM downloads
            ORDER BY priority DESC, created_at ASC
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to list downloads: {}", e))))?;

        Ok(rows)
    }

    /// List downloads with a specific status
    pub async fn list_downloads_by_status(&self, status: i32) -> Result<Vec<Download>> {
        let rows = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at
            FROM downloads
            WHERE status = ?
            ORDER BY priority DESC, created_at ASC
            "#
        )
        .bind(status)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to list downloads by status: {}", e))))?;

        Ok(rows)
    }

    /// Update download status
    pub async fn update_status(&self, id: DownloadId, status: i32) -> Result<()> {
        sqlx::query("UPDATE downloads SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to update status: {}", e))))?;

        Ok(())
    }

    /// Update download progress
    pub async fn update_progress(
        &self,
        id: DownloadId,
        progress: f32,
        speed_bps: u64,
        downloaded_bytes: u64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE downloads SET progress = ?, speed_bps = ?, downloaded_bytes = ? WHERE id = ?"
        )
        .bind(progress)
        .bind(speed_bps as i64)
        .bind(downloaded_bytes as i64)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to update progress: {}", e))))?;

        Ok(())
    }

    /// Update download priority
    pub async fn update_priority(&self, id: DownloadId, priority: i32) -> Result<()> {
        sqlx::query("UPDATE downloads SET priority = ? WHERE id = ?")
            .bind(priority)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to update priority: {}", e))))?;

        Ok(())
    }

    /// Set download error message
    pub async fn set_error(&self, id: DownloadId, error: &str) -> Result<()> {
        sqlx::query("UPDATE downloads SET error_message = ? WHERE id = ?")
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to set error: {}", e))))?;

        Ok(())
    }

    /// Set download started timestamp
    pub async fn set_started(&self, id: DownloadId) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE downloads SET started_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to set started timestamp: {}", e))))?;

        Ok(())
    }

    /// Set download completed timestamp
    pub async fn set_completed(&self, id: DownloadId) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE downloads SET completed_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to set completed timestamp: {}", e))))?;

        Ok(())
    }

    /// Delete a download
    pub async fn delete_download(&self, id: DownloadId) -> Result<()> {
        sqlx::query("DELETE FROM downloads WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to delete download: {}", e))))?;

        Ok(())
    }

    /// Get incomplete downloads (for resume on startup)
    pub async fn get_incomplete_downloads(&self) -> Result<Vec<Download>> {
        let rows = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at
            FROM downloads
            WHERE status IN (0, 1, 3)
            ORDER BY priority DESC, created_at ASC
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to get incomplete downloads: {}", e))))?;

        Ok(rows)
    }

    /// Get all downloads (for state persistence during shutdown)
    pub async fn get_all_downloads(&self) -> Result<Vec<Download>> {
        let rows = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at
            FROM downloads
            ORDER BY created_at ASC
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to get all downloads: {}", e))))?;

        Ok(rows)
    }

    // Article-level tracking operations (for download resume support)

    /// Insert a single article
    pub async fn insert_article(&self, article: &NewArticle) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO download_articles (
                download_id, message_id, segment_number, size_bytes, status
            ) VALUES (?, ?, ?, ?, 0)
            "#
        )
        .bind(article.download_id)
        .bind(&article.message_id)
        .bind(article.segment_number)
        .bind(article.size_bytes)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to insert article: {}", e))))?;

        Ok(result.last_insert_rowid())
    }

    /// Insert multiple articles in a batch (more efficient for large NZB files)
    pub async fn insert_articles_batch(&self, articles: &[NewArticle]) -> Result<()> {
        if articles.is_empty() {
            return Ok(());
        }

        // Build a multi-row insert query
        let mut query_builder = sqlx::QueryBuilder::new(
            "INSERT INTO download_articles (download_id, message_id, segment_number, size_bytes, status) "
        );

        query_builder.push_values(articles, |mut b, article| {
            b.push_bind(article.download_id)
                .push_bind(&article.message_id)
                .push_bind(article.segment_number)
                .push_bind(article.size_bytes)
                .push_bind(0); // status = PENDING
        });

        let query = query_builder.build();
        query.execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to insert articles batch: {}", e))))?;

        Ok(())
    }

    /// Update article status
    pub async fn update_article_status(&self, article_id: i64, status: i32) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            UPDATE download_articles
            SET status = ?, downloaded_at = ?
            WHERE id = ?
            "#
        )
        .bind(status)
        .bind(if status == article_status::DOWNLOADED { Some(now) } else { None })
        .bind(article_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to update article status: {}", e))))?;

        Ok(())
    }

    /// Update article status by message_id
    pub async fn update_article_status_by_message_id(
        &self,
        download_id: DownloadId,
        message_id: &str,
        status: i32,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            UPDATE download_articles
            SET status = ?, downloaded_at = ?
            WHERE download_id = ? AND message_id = ?
            "#
        )
        .bind(status)
        .bind(if status == article_status::DOWNLOADED { Some(now) } else { None })
        .bind(download_id)
        .bind(message_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to update article status: {}", e))))?;

        Ok(())
    }

    /// Update multiple article statuses in a single transaction (more efficient for batch operations)
    ///
    /// # Arguments
    /// * `updates` - Vector of tuples containing (article_id, status)
    ///
    /// # Performance
    /// This method uses a CASE-WHEN statement to update multiple rows in a single query,
    /// which is significantly faster than individual UPDATE statements. With 100 updates,
    /// this can be 50-100x faster than calling `update_article_status` 100 times.
    ///
    /// # Example
    /// ```rust,ignore
    /// let updates = vec![
    ///     (123, article_status::DOWNLOADED),
    ///     (124, article_status::DOWNLOADED),
    ///     (125, article_status::FAILED),
    /// ];
    /// db.update_articles_status_batch(&updates).await?;
    /// ```
    pub async fn update_articles_status_batch(&self, updates: &[(i64, i32)]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }

        let now = chrono::Utc::now().timestamp();

        // Build a multi-row update query using CASE-WHEN
        // UPDATE download_articles
        // SET status = CASE
        //   WHEN id = 1 THEN 1
        //   WHEN id = 2 THEN 1
        //   ...
        // END,
        // downloaded_at = CASE
        //   WHEN id = 1 AND status = 1 THEN timestamp
        //   WHEN id = 2 AND status = 1 THEN timestamp
        //   ...
        // END
        // WHERE id IN (1, 2, ...)

        let mut query_builder = sqlx::QueryBuilder::new("UPDATE download_articles SET status = CASE ");

        // Build status CASE clause
        for (article_id, status) in updates {
            query_builder.push("WHEN id = ");
            query_builder.push_bind(*article_id);
            query_builder.push(" THEN ");
            query_builder.push_bind(*status);
            query_builder.push(" ");
        }
        query_builder.push("END, downloaded_at = CASE ");

        // Build downloaded_at CASE clause (only set timestamp for DOWNLOADED status)
        for (article_id, status) in updates {
            query_builder.push("WHEN id = ");
            query_builder.push_bind(*article_id);
            if *status == article_status::DOWNLOADED {
                query_builder.push(" THEN ");
                query_builder.push_bind(now);
            } else {
                query_builder.push(" THEN downloaded_at"); // Keep existing value
            }
            query_builder.push(" ");
        }
        query_builder.push("END WHERE id IN (");

        // Build WHERE IN clause
        let mut first = true;
        for (article_id, _) in updates {
            if !first {
                query_builder.push(", ");
            }
            query_builder.push_bind(*article_id);
            first = false;
        }
        query_builder.push(")");

        let query = query_builder.build();
        query.execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to update articles status batch: {}", e))))?;

        Ok(())
    }

    /// Get all articles for a download
    pub async fn get_articles(&self, download_id: DownloadId) -> Result<Vec<Article>> {
        let rows = sqlx::query_as::<_, Article>(
            r#"
            SELECT id, download_id, message_id, segment_number, size_bytes, status, downloaded_at
            FROM download_articles
            WHERE download_id = ?
            ORDER BY segment_number ASC
            "#
        )
        .bind(download_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to get articles: {}", e))))?;

        Ok(rows)
    }

    /// Get pending articles for a download (for resume)
    pub async fn get_pending_articles(&self, download_id: DownloadId) -> Result<Vec<Article>> {
        let rows = sqlx::query_as::<_, Article>(
            r#"
            SELECT id, download_id, message_id, segment_number, size_bytes, status, downloaded_at
            FROM download_articles
            WHERE download_id = ? AND status = 0
            ORDER BY segment_number ASC
            "#
        )
        .bind(download_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to get pending articles: {}", e))))?;

        Ok(rows)
    }

    /// Get article by message_id
    pub async fn get_article_by_message_id(
        &self,
        download_id: DownloadId,
        message_id: &str,
    ) -> Result<Option<Article>> {
        let row = sqlx::query_as::<_, Article>(
            r#"
            SELECT id, download_id, message_id, segment_number, size_bytes, status, downloaded_at
            FROM download_articles
            WHERE download_id = ? AND message_id = ?
            "#
        )
        .bind(download_id)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to get article: {}", e))))?;

        Ok(row)
    }

    /// Count articles by status for a download
    pub async fn count_articles_by_status(
        &self,
        download_id: DownloadId,
        status: i32,
    ) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM download_articles WHERE download_id = ? AND status = ?"
        )
        .bind(download_id)
        .bind(status)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to count articles: {}", e))))?;

        Ok(count)
    }

    /// Get total article count for a download
    pub async fn count_articles(&self, download_id: DownloadId) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM download_articles WHERE download_id = ?"
        )
        .bind(download_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to count articles: {}", e))))?;

        Ok(count)
    }

    /// Delete all articles for a download (automatic via CASCADE, but explicit method for clarity)
    pub async fn delete_articles(&self, download_id: DownloadId) -> Result<()> {
        sqlx::query("DELETE FROM download_articles WHERE download_id = ?")
            .bind(download_id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to delete articles: {}", e))))?;

        Ok(())
    }

    // Password cache operations (for archive extraction)

    /// Cache a correct password for a download
    ///
    /// This is used after successfully extracting an archive with a password,
    /// so if the download needs to be reprocessed, we can try this password first.
    pub async fn set_correct_password(&self, download_id: DownloadId, password: &str) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO passwords (download_id, correct_password)
            VALUES (?, ?)
            ON CONFLICT(download_id) DO UPDATE SET correct_password = excluded.correct_password
            "#
        )
        .bind(download_id)
        .bind(password)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to set correct password: {}", e))))?;

        Ok(())
    }

    /// Get the cached correct password for a download
    ///
    /// Returns None if no password is cached for this download.
    pub async fn get_cached_password(&self, download_id: DownloadId) -> Result<Option<String>> {
        let password: Option<String> = sqlx::query_scalar(
            "SELECT correct_password FROM passwords WHERE download_id = ?"
        )
        .bind(download_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to get cached password: {}", e))))?;

        Ok(password)
    }

    /// Delete cached password for a download
    ///
    /// Note: This is automatically deleted via CASCADE when the download is deleted.
    pub async fn delete_cached_password(&self, download_id: DownloadId) -> Result<()> {
        sqlx::query("DELETE FROM passwords WHERE download_id = ?")
            .bind(download_id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to delete cached password: {}", e))))?;

        Ok(())
    }

    // Duplicate detection queries

    /// Find a download by NZB hash
    ///
    /// This is the most reliable duplicate detection method as it compares
    /// the actual NZB file content (via SHA-256 hash).
    pub async fn find_by_nzb_hash(&self, nzb_hash: &str) -> Result<Option<Download>> {
        let row = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at
            FROM downloads
            WHERE nzb_hash = ?
            LIMIT 1
            "#
        )
        .bind(nzb_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to find download by nzb_hash: {}", e))))?;

        Ok(row)
    }

    /// Find a download by exact name match
    ///
    /// This is useful for detecting duplicates when the NZB filename is used
    /// as the download name. Case-sensitive match.
    pub async fn find_by_name(&self, name: &str) -> Result<Option<Download>> {
        let row = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at
            FROM downloads
            WHERE name = ?
            LIMIT 1
            "#
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to find download by name: {}", e))))?;

        Ok(row)
    }

    /// Find a download by job name
    ///
    /// This detects duplicates using the deobfuscated job name, which catches
    /// cases where the same content is uploaded with different NZB filenames.
    pub async fn find_by_job_name(&self, job_name: &str) -> Result<Option<Download>> {
        let row = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at
            FROM downloads
            WHERE job_name = ?
            LIMIT 1
            "#
        )
        .bind(job_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to find download by job_name: {}", e))))?;

        Ok(row)
    }

    // ==================== History Operations ====================

    /// Insert a download into history
    ///
    /// This is typically called when a download is completed (successfully or failed)
    /// to create a historical record separate from the active downloads table.
    pub async fn insert_history(&self, entry: &NewHistoryEntry) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO history (
                name, category, destination, status, size_bytes,
                download_time_secs, completed_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&entry.name)
        .bind(&entry.category)
        .bind(entry.destination.as_ref().map(|p| p.to_string_lossy().into_owned()))
        .bind(entry.status)
        .bind(entry.size_bytes as i64)
        .bind(entry.download_time_secs)
        .bind(entry.completed_at)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Sqlx(e))?;

        Ok(result.last_insert_rowid())
    }

    /// Query history with pagination and optional status filter
    ///
    /// Returns history entries ordered by completion time (most recent first).
    /// Use limit and offset for pagination.
    pub async fn query_history(
        &self,
        status_filter: Option<i32>,
        limit: usize,
        offset: usize
    ) -> Result<Vec<HistoryEntry>> {
        let query = if let Some(status) = status_filter {
            sqlx::query_as::<_, HistoryRow>(
                r#"
                SELECT id, name, category, destination, status, size_bytes,
                       download_time_secs, completed_at
                FROM history
                WHERE status = ?
                ORDER BY completed_at DESC
                LIMIT ? OFFSET ?
                "#
            )
            .bind(status)
            .bind(limit as i64)
            .bind(offset as i64)
        } else {
            sqlx::query_as::<_, HistoryRow>(
                r#"
                SELECT id, name, category, destination, status, size_bytes,
                       download_time_secs, completed_at
                FROM history
                ORDER BY completed_at DESC
                LIMIT ? OFFSET ?
                "#
            )
            .bind(limit as i64)
            .bind(offset as i64)
        };

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::Sqlx(e))?;

        Ok(rows.into_iter().map(HistoryEntry::from).collect())
    }

    /// Count history entries (optionally filtered by status)
    ///
    /// Useful for pagination - returns total count of records matching the filter.
    pub async fn count_history(&self, status_filter: Option<i32>) -> Result<i64> {
        let count = if let Some(status) = status_filter {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM history WHERE status = ?")
                .bind(status)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| Error::Sqlx(e))?
        } else {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM history")
                .fetch_one(&self.pool)
                .await
                .map_err(|e| Error::Sqlx(e))?
        };

        Ok(count)
    }

    /// Delete history entries older than the specified timestamp
    ///
    /// Returns the number of records deleted.
    /// This is useful for cleanup - e.g., delete history older than 30 days.
    pub async fn delete_history_before(&self, before_timestamp: i64) -> Result<u64> {
        let result = sqlx::query("DELETE FROM history WHERE completed_at < ?")
            .bind(before_timestamp)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Sqlx(e))?;

        Ok(result.rows_affected())
    }

    /// Delete history entries with a specific status
    ///
    /// Returns the number of records deleted.
    /// This is useful for cleanup - e.g., delete all failed downloads from history.
    pub async fn delete_history_by_status(&self, status: i32) -> Result<u64> {
        let result = sqlx::query("DELETE FROM history WHERE status = ?")
            .bind(status)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Sqlx(e))?;

        Ok(result.rows_affected())
    }

    /// Clear all history
    ///
    /// Returns the number of records deleted.
    /// This is a destructive operation - use with caution.
    pub async fn clear_history(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM history")
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Sqlx(e))?;

        Ok(result.rows_affected())
    }

    /// Delete history entries with optional filters
    ///
    /// Returns the number of records deleted.
    /// Supports filtering by:
    /// - before_timestamp: Delete entries completed before this timestamp
    /// - status: Delete only entries with this status
    ///
    /// If both filters are None, deletes all history (same as clear_history).
    pub async fn delete_history_filtered(
        &self,
        before_timestamp: Option<i64>,
        status: Option<i32>,
    ) -> Result<u64> {
        match (before_timestamp, status) {
            (None, None) => {
                // No filters - delete all
                self.clear_history().await
            }
            (Some(before), None) => {
                // Only timestamp filter
                self.delete_history_before(before).await
            }
            (None, Some(status_val)) => {
                // Only status filter
                self.delete_history_by_status(status_val).await
            }
            (Some(before), Some(status_val)) => {
                // Both filters
                let result = sqlx::query("DELETE FROM history WHERE completed_at < ? AND status = ?")
                    .bind(before)
                    .bind(status_val)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| Error::Sqlx(e))?;

                Ok(result.rows_affected())
            }
        }
    }

    /// Get a single history entry by ID
    pub async fn get_history_entry(&self, id: i64) -> Result<Option<HistoryEntry>> {
        let row = sqlx::query_as::<_, HistoryRow>(
            r#"
            SELECT id, name, category, destination, status, size_bytes,
                   download_time_secs, completed_at
            FROM history
            WHERE id = ?
            "#
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Sqlx(e))?;

        Ok(row.map(HistoryEntry::from))
    }

    // Shutdown state tracking methods

    /// Check if the last shutdown was unclean
    ///
    /// Returns true if the previous session did not call set_clean_shutdown(),
    /// indicating a crash or forced termination.
    ///
    /// This method is called on startup to determine if state recovery is needed.
    pub async fn was_unclean_shutdown(&self) -> Result<bool> {
        let value: Option<String> = sqlx::query_scalar(
            r#"
            SELECT value FROM runtime_state WHERE key = 'clean_shutdown'
            "#
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to check shutdown state: {}", e))))?;

        // If the value is missing or "false", it was an unclean shutdown
        Ok(value.map_or(true, |v| v != "true"))
    }

    /// Mark that the application has started cleanly
    ///
    /// This should be called during UsenetDownloader::new() to indicate that
    /// the application is running. If shutdown() is not called before the next
    /// startup, was_unclean_shutdown() will return true.
    pub async fn set_clean_start(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            r#"
            INSERT INTO runtime_state (key, value, updated_at)
            VALUES ('clean_shutdown', 'false', ?)
            ON CONFLICT(key) DO UPDATE SET value = 'false', updated_at = ?
            "#
        )
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to set clean start: {}", e))))?;

        Ok(())
    }

    /// Mark that the application is shutting down cleanly
    ///
    /// This should be called during UsenetDownloader::shutdown() to indicate
    /// a graceful shutdown. If this is not called before the process exits,
    /// the next startup will detect an unclean shutdown.
    pub async fn set_clean_shutdown(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            r#"
            INSERT INTO runtime_state (key, value, updated_at)
            VALUES ('clean_shutdown', 'true', ?)
            ON CONFLICT(key) DO UPDATE SET value = 'true', updated_at = ?
            "#
        )
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to set clean shutdown: {}", e))))?;

        Ok(())
    }

    /// Mark an NZB file as processed
    ///
    /// This is used by the folder watcher with WatchFolderAction::Keep to track
    /// which NZB files have already been processed to avoid re-adding them.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the NZB file
    ///
    /// # Returns
    ///
    /// Returns Ok(()) on success, or an error if the database operation fails.
    pub async fn mark_nzb_processed(&self, path: &std::path::Path) -> Result<()> {
        let path_str = path.to_string_lossy().to_string();
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            INSERT INTO processed_nzbs (path, processed_at)
            VALUES (?, ?)
            ON CONFLICT(path) DO UPDATE SET processed_at = ?
            "#
        )
        .bind(&path_str)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to mark NZB as processed: {}", e))))?;

        Ok(())
    }

    /// Check if an NZB file has been processed
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the NZB file
    ///
    /// # Returns
    ///
    /// Returns true if the NZB has been processed before, false otherwise.
    pub async fn is_nzb_processed(&self, path: &std::path::Path) -> Result<bool> {
        let path_str = path.to_string_lossy().to_string();

        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM processed_nzbs WHERE path = ?
            "#
        )
        .bind(&path_str)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to check if NZB is processed: {}", e))))?;

        Ok(count > 0)
    }

    /// Check if an RSS feed item has been seen before
    ///
    /// # Arguments
    ///
    /// * `feed_id` - RSS feed ID
    /// * `guid` - Unique identifier for the RSS item (GUID or link)
    ///
    /// # Returns
    ///
    /// Returns true if the item has been seen before, false otherwise.
    pub async fn is_rss_item_seen(&self, feed_id: i64, guid: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM rss_seen WHERE feed_id = ? AND guid = ?
            "#
        )
        .bind(feed_id)
        .bind(guid)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to check if RSS item is seen: {}", e))))?;

        Ok(count > 0)
    }

    /// Mark an RSS feed item as seen
    ///
    /// # Arguments
    ///
    /// * `feed_id` - RSS feed ID
    /// * `guid` - Unique identifier for the RSS item (GUID or link)
    ///
    /// # Returns
    ///
    /// Returns Ok(()) on success.
    pub async fn mark_rss_item_seen(&self, feed_id: i64, guid: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            INSERT INTO rss_seen (feed_id, guid, seen_at)
            VALUES (?, ?, ?)
            ON CONFLICT(feed_id, guid) DO UPDATE SET seen_at = ?
            "#
        )
        .bind(feed_id)
        .bind(guid)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to mark RSS item as seen: {}", e))))?;

        Ok(())
    }

    // =========================================================================
    // RSS Feed CRUD Operations
    // =========================================================================

    /// Get all RSS feeds
    pub async fn get_all_rss_feeds(&self) -> Result<Vec<RssFeed>> {
        let feeds = sqlx::query_as::<_, RssFeed>(
            r#"
            SELECT id, name, url, check_interval_secs, category, auto_download,
                   priority, enabled, last_check, last_error, created_at
            FROM rss_feeds
            ORDER BY id ASC
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to get RSS feeds: {}", e))))?;

        Ok(feeds)
    }

    /// Get RSS feed by ID
    pub async fn get_rss_feed(&self, id: i64) -> Result<Option<RssFeed>> {
        let feed = sqlx::query_as::<_, RssFeed>(
            r#"
            SELECT id, name, url, check_interval_secs, category, auto_download,
                   priority, enabled, last_check, last_error, created_at
            FROM rss_feeds
            WHERE id = ?
            "#
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to get RSS feed: {}", e))))?;

        Ok(feed)
    }

    /// Insert a new RSS feed
    pub async fn insert_rss_feed(
        &self,
        name: &str,
        url: &str,
        check_interval_secs: i64,
        category: Option<&str>,
        auto_download: bool,
        priority: i32,
        enabled: bool,
    ) -> Result<i64> {
        let now = chrono::Utc::now().timestamp();

        let result = sqlx::query(
            r#"
            INSERT INTO rss_feeds (name, url, check_interval_secs, category, auto_download,
                                  priority, enabled, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(name)
        .bind(url)
        .bind(check_interval_secs)
        .bind(category)
        .bind(auto_download as i32)
        .bind(priority)
        .bind(enabled as i32)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to insert RSS feed: {}", e))))?;

        Ok(result.last_insert_rowid())
    }

    /// Update an existing RSS feed
    pub async fn update_rss_feed(
        &self,
        id: i64,
        name: &str,
        url: &str,
        check_interval_secs: i64,
        category: Option<&str>,
        auto_download: bool,
        priority: i32,
        enabled: bool,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE rss_feeds
            SET name = ?, url = ?, check_interval_secs = ?, category = ?,
                auto_download = ?, priority = ?, enabled = ?
            WHERE id = ?
            "#
        )
        .bind(name)
        .bind(url)
        .bind(check_interval_secs)
        .bind(category)
        .bind(auto_download as i32)
        .bind(priority)
        .bind(enabled as i32)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to update RSS feed: {}", e))))?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete an RSS feed (cascades to filters and seen items)
    pub async fn delete_rss_feed(&self, id: i64) -> Result<bool> {
        let result = sqlx::query("DELETE FROM rss_feeds WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to delete RSS feed: {}", e))))?;

        Ok(result.rows_affected() > 0)
    }

    /// Get all filters for a specific RSS feed
    pub async fn get_rss_filters(&self, feed_id: i64) -> Result<Vec<RssFilterRow>> {
        let filters = sqlx::query_as::<_, RssFilterRow>(
            r#"
            SELECT id, feed_id, name, include_patterns, exclude_patterns,
                   min_size, max_size, max_age_secs
            FROM rss_filters
            WHERE feed_id = ?
            ORDER BY id ASC
            "#
        )
        .bind(feed_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to get RSS filters: {}", e))))?;

        Ok(filters)
    }

    /// Insert a new RSS filter
    pub async fn insert_rss_filter(
        &self,
        feed_id: i64,
        name: &str,
        include_patterns: Option<&str>,
        exclude_patterns: Option<&str>,
        min_size: Option<i64>,
        max_size: Option<i64>,
        max_age_secs: Option<i64>,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO rss_filters (feed_id, name, include_patterns, exclude_patterns,
                                    min_size, max_size, max_age_secs)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(feed_id)
        .bind(name)
        .bind(include_patterns)
        .bind(exclude_patterns)
        .bind(min_size)
        .bind(max_size)
        .bind(max_age_secs)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to insert RSS filter: {}", e))))?;

        Ok(result.last_insert_rowid())
    }

    /// Delete all filters for a feed (used during update)
    pub async fn delete_rss_filters(&self, feed_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM rss_filters WHERE feed_id = ?")
            .bind(feed_id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to delete RSS filters: {}", e))))?;

        Ok(())
    }

    /// Update last check time and error for an RSS feed
    pub async fn update_rss_feed_check_status(
        &self,
        id: i64,
        last_error: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            UPDATE rss_feeds
            SET last_check = ?, last_error = ?
            WHERE id = ?
            "#
        )
        .bind(now)
        .bind(last_error)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(format!("Failed to update RSS feed status: {}", e))))?;

        Ok(())
    }
}

#[cfg(test)]

#[cfg(test)]
mod tests;
