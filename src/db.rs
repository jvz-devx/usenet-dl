//! Database layer for usenet-dl
//!
//! Handles SQLite persistence for downloads, articles, passwords, and history.

use crate::{types::DownloadId, Error, Result};
use sqlx::{sqlite::SqlitePool, FromRow, SqliteConnection};
use std::path::Path;

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
                .map_err(|e| Error::Database(format!("Failed to create database directory: {}", e)))?;
        }

        // Connect to database
        let connection_string = format!("sqlite:{}?mode=rwc", path.display());
        let pool = SqlitePool::connect(&connection_string)
            .await
            .map_err(|e| Error::Database(format!("Failed to connect to database: {}", e)))?;

        let db = Self { pool };

        // Run migrations
        db.run_migrations().await?;

        Ok(db)
    }

    /// Run database migrations
    async fn run_migrations(&self) -> Result<()> {
        let mut conn = self.pool.acquire().await
            .map_err(|e| Error::Database(format!("Failed to acquire connection: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to create schema_version table: {}", e)))?;

        // Check current version
        let current_version: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(version) FROM schema_version"
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| Error::Database(format!("Failed to query schema version: {}", e)))?;

        let current_version = current_version.unwrap_or(0);

        // Apply migrations
        if current_version < 1 {
            Self::migrate_v1(&mut conn).await?;
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
        .map_err(|e| Error::Database(format!("Failed to create downloads table: {}", e)))?;

        // Indexes for downloads
        sqlx::query("CREATE INDEX idx_downloads_status ON downloads(status)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(format!("Failed to create index: {}", e)))?;

        sqlx::query("CREATE INDEX idx_downloads_priority ON downloads(priority DESC, created_at ASC)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(format!("Failed to create index: {}", e)))?;

        sqlx::query("CREATE INDEX idx_downloads_nzb_hash ON downloads(nzb_hash)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(format!("Failed to create index: {}", e)))?;

        sqlx::query("CREATE INDEX idx_downloads_job_name ON downloads(job_name)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(format!("Failed to create index: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to create download_articles table: {}", e)))?;

        // Indexes for download_articles
        sqlx::query("CREATE INDEX idx_articles_download ON download_articles(download_id)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(format!("Failed to create index: {}", e)))?;

        sqlx::query("CREATE INDEX idx_articles_status ON download_articles(download_id, status)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(format!("Failed to create index: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to create passwords table: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to create processed_nzbs table: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to create history table: {}", e)))?;

        // Index for history
        sqlx::query("CREATE INDEX idx_history_completed ON history(completed_at DESC)")
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(format!("Failed to create index: {}", e)))?;

        // Record migration
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (1, ?)")
            .bind(now)
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::Database(format!("Failed to record migration: {}", e)))?;

        tracing::info!("Database migration v1 complete");

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
        .map_err(|e| Error::Database(format!("Failed to insert download: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to get download: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to list downloads: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to list downloads by status: {}", e)))?;

        Ok(rows)
    }

    /// Update download status
    pub async fn update_status(&self, id: DownloadId, status: i32) -> Result<()> {
        sqlx::query("UPDATE downloads SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(format!("Failed to update status: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to update progress: {}", e)))?;

        Ok(())
    }

    /// Update download priority
    pub async fn update_priority(&self, id: DownloadId, priority: i32) -> Result<()> {
        sqlx::query("UPDATE downloads SET priority = ? WHERE id = ?")
            .bind(priority)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(format!("Failed to update priority: {}", e)))?;

        Ok(())
    }

    /// Set download error message
    pub async fn set_error(&self, id: DownloadId, error: &str) -> Result<()> {
        sqlx::query("UPDATE downloads SET error_message = ? WHERE id = ?")
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(format!("Failed to set error: {}", e)))?;

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
            .map_err(|e| Error::Database(format!("Failed to set started timestamp: {}", e)))?;

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
            .map_err(|e| Error::Database(format!("Failed to set completed timestamp: {}", e)))?;

        Ok(())
    }

    /// Delete a download
    pub async fn delete_download(&self, id: DownloadId) -> Result<()> {
        sqlx::query("DELETE FROM downloads WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(format!("Failed to delete download: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to get incomplete downloads: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to insert article: {}", e)))?;

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
            .map_err(|e| Error::Database(format!("Failed to insert articles batch: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to update article status: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to update article status: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to get articles: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to get pending articles: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to get article: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to count articles: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to count articles: {}", e)))?;

        Ok(count)
    }

    /// Delete all articles for a download (automatic via CASCADE, but explicit method for clarity)
    pub async fn delete_articles(&self, download_id: DownloadId) -> Result<()> {
        sqlx::query("DELETE FROM download_articles WHERE download_id = ?")
            .bind(download_id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database(format!("Failed to delete articles: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to set correct password: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to get cached password: {}", e)))?;

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
            .map_err(|e| Error::Database(format!("Failed to delete cached password: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to find download by nzb_hash: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to find download by name: {}", e)))?;

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
        .map_err(|e| Error::Database(format!("Failed to find download by job_name: {}", e)))?;

        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_database_creation() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();

        let db = Database::new(db_path).await.unwrap();

        // Verify tables exist
        let mut conn = db.pool.acquire().await.unwrap();

        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
        )
        .fetch_all(&mut *conn)
        .await
        .unwrap();

        assert!(tables.contains(&"downloads".to_string()));
        assert!(tables.contains(&"download_articles".to_string()));
        assert!(tables.contains(&"passwords".to_string()));
        assert!(tables.contains(&"processed_nzbs".to_string()));
        assert!(tables.contains(&"history".to_string()));
        assert!(tables.contains(&"schema_version".to_string()));

        db.close().await;
    }

    #[tokio::test]
    async fn test_migration_idempotency() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();

        // Create database twice
        let db1 = Database::new(db_path).await.unwrap();
        db1.close().await;

        let db2 = Database::new(db_path).await.unwrap();

        // Verify schema version is 1
        let mut conn = db2.pool.acquire().await.unwrap();
        let version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM schema_version")
            .fetch_one(&mut *conn)
            .await
            .unwrap();

        assert_eq!(version, 1);

        db2.close().await;
    }

    #[tokio::test]
    async fn test_insert_and_get_download() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Insert a download
        let new_download = NewDownload {
            name: "Test Download".to_string(),
            nzb_path: "/path/to/test.nzb".to_string(),
            nzb_meta_name: Some("Test Meta Name".to_string()),
            nzb_hash: Some("abc123".to_string()),
            job_name: Some("test_job".to_string()),
            category: Some("movies".to_string()),
            destination: "/downloads/movies".to_string(),
            post_process: 4, // UnpackAndCleanup
            priority: 0, // Normal
            status: 0, // Queued
            size_bytes: 1024 * 1024 * 100, // 100 MB
        };

        let id = db.insert_download(&new_download).await.unwrap();
        assert!(id > 0);

        // Get the download
        let download = db.get_download(id).await.unwrap();
        assert!(download.is_some());

        let download = download.unwrap();
        assert_eq!(download.name, "Test Download");
        assert_eq!(download.nzb_path, "/path/to/test.nzb");
        assert_eq!(download.category, Some("movies".to_string()));
        assert_eq!(download.status, 0);
        assert_eq!(download.progress, 0.0);
        assert_eq!(download.size_bytes, 1024 * 1024 * 100);

        db.close().await;
    }

    #[tokio::test]
    async fn test_list_downloads() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Insert multiple downloads
        for i in 0..3 {
            let new_download = NewDownload {
                name: format!("Download {}", i),
                nzb_path: format!("/path/to/test{}.nzb", i),
                nzb_meta_name: None,
                nzb_hash: None,
                job_name: None,
                category: None,
                destination: "/downloads".to_string(),
                post_process: 4,
                priority: i as i32,
                status: 0,
                size_bytes: 1024,
            };
            db.insert_download(&new_download).await.unwrap();
        }

        // List all downloads
        let downloads = db.list_downloads().await.unwrap();
        assert_eq!(downloads.len(), 3);

        // Should be ordered by priority DESC
        assert_eq!(downloads[0].name, "Download 2");
        assert_eq!(downloads[1].name, "Download 1");
        assert_eq!(downloads[2].name, "Download 0");

        db.close().await;
    }

    #[tokio::test]
    async fn test_update_status() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0, // Queued
            size_bytes: 1024,
        };

        let id = db.insert_download(&new_download).await.unwrap();

        // Update status to Downloading (1)
        db.update_status(id, 1).await.unwrap();

        let download = db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, 1);

        db.close().await;
    }

    #[tokio::test]
    async fn test_update_progress() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 1, // Downloading
            size_bytes: 1024 * 1024,
        };

        let id = db.insert_download(&new_download).await.unwrap();

        // Update progress
        db.update_progress(id, 45.5, 1024 * 1024, 500 * 1024).await.unwrap();

        let download = db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.progress, 45.5);
        assert_eq!(download.speed_bps, 1024 * 1024);
        assert_eq!(download.downloaded_bytes, 500 * 1024);

        db.close().await;
    }

    #[tokio::test]
    async fn test_delete_download() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };

        let id = db.insert_download(&new_download).await.unwrap();

        // Delete the download
        db.delete_download(id).await.unwrap();

        // Should not exist anymore
        let download = db.get_download(id).await.unwrap();
        assert!(download.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn test_get_incomplete_downloads() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Insert downloads with different statuses
        for (i, status) in [0, 1, 2, 3, 4, 5].iter().enumerate() {
            let new_download = NewDownload {
                name: format!("Download {}", i),
                nzb_path: format!("/test{}.nzb", i),
                nzb_meta_name: None,
                nzb_hash: None,
                job_name: None,
                category: None,
                destination: "/downloads".to_string(),
                post_process: 4,
                priority: 0,
                status: *status,
                size_bytes: 1024,
            };
            db.insert_download(&new_download).await.unwrap();
        }

        // Get incomplete (statuses: 0=Queued, 1=Downloading, 3=Processing)
        let incomplete = db.get_incomplete_downloads().await.unwrap();

        // Should only have 3 downloads (status 0, 1, 3)
        assert_eq!(incomplete.len(), 3);

        db.close().await;
    }

    // Article-level tracking tests

    #[tokio::test]
    async fn test_insert_and_get_article() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Create a download first
        let new_download = NewDownload {
            name: "Test Download".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024 * 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Insert an article
        let new_article = NewArticle {
            download_id,
            message_id: "<test@example.com>".to_string(),
            segment_number: 1,
            size_bytes: 512 * 1024,
        };
        let article_id = db.insert_article(&new_article).await.unwrap();
        assert!(article_id > 0);

        // Get the article
        let article = db.get_article_by_message_id(download_id, "<test@example.com>")
            .await.unwrap();
        assert!(article.is_some());

        let article = article.unwrap();
        assert_eq!(article.download_id, download_id);
        assert_eq!(article.message_id, "<test@example.com>");
        assert_eq!(article.segment_number, 1);
        assert_eq!(article.size_bytes, 512 * 1024);
        assert_eq!(article.status, super::article_status::PENDING);
        assert!(article.downloaded_at.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn test_insert_articles_batch() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Create a download
        let new_download = NewDownload {
            name: "Test Download".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024 * 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Insert multiple articles in a batch
        let articles: Vec<NewArticle> = (0..100).map(|i| NewArticle {
            download_id,
            message_id: format!("<article{}@example.com>", i),
            segment_number: i,
            size_bytes: 10240,
        }).collect();

        db.insert_articles_batch(&articles).await.unwrap();

        // Verify all articles were inserted
        let count = db.count_articles(download_id).await.unwrap();
        assert_eq!(count, 100);

        // Verify they're all pending
        let pending_count = db.count_articles_by_status(
            download_id,
            super::article_status::PENDING
        ).await.unwrap();
        assert_eq!(pending_count, 100);

        db.close().await;
    }

    #[tokio::test]
    async fn test_update_article_status() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Create a download and article
        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 1, // Downloading
            size_bytes: 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        let new_article = NewArticle {
            download_id,
            message_id: "<test@example.com>".to_string(),
            segment_number: 1,
            size_bytes: 1024,
        };
        let article_id = db.insert_article(&new_article).await.unwrap();

        // Update status to DOWNLOADED
        db.update_article_status(article_id, super::article_status::DOWNLOADED)
            .await.unwrap();

        // Verify status was updated
        let article = db.get_article_by_message_id(download_id, "<test@example.com>")
            .await.unwrap().unwrap();
        assert_eq!(article.status, super::article_status::DOWNLOADED);
        assert!(article.downloaded_at.is_some());

        db.close().await;
    }

    #[tokio::test]
    async fn test_get_pending_articles() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Create a download
        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 1,
            size_bytes: 10240,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Insert 10 articles
        let articles: Vec<NewArticle> = (0..10).map(|i| NewArticle {
            download_id,
            message_id: format!("<article{}@example.com>", i),
            segment_number: i,
            size_bytes: 1024,
        }).collect();
        db.insert_articles_batch(&articles).await.unwrap();

        // Mark some as downloaded
        for i in 0..5 {
            db.update_article_status_by_message_id(
                download_id,
                &format!("<article{}@example.com>", i),
                super::article_status::DOWNLOADED,
            ).await.unwrap();
        }

        // Mark one as failed
        db.update_article_status_by_message_id(
            download_id,
            "<article5@example.com>",
            super::article_status::FAILED,
        ).await.unwrap();

        // Get pending articles (should be 4 remaining: 6, 7, 8, 9)
        let pending = db.get_pending_articles(download_id).await.unwrap();
        assert_eq!(pending.len(), 4);
        assert_eq!(pending[0].segment_number, 6);
        assert_eq!(pending[1].segment_number, 7);
        assert_eq!(pending[2].segment_number, 8);
        assert_eq!(pending[3].segment_number, 9);

        // Verify counts
        let downloaded_count = db.count_articles_by_status(
            download_id,
            super::article_status::DOWNLOADED
        ).await.unwrap();
        assert_eq!(downloaded_count, 5);

        let failed_count = db.count_articles_by_status(
            download_id,
            super::article_status::FAILED
        ).await.unwrap();
        assert_eq!(failed_count, 1);

        db.close().await;
    }

    #[tokio::test]
    async fn test_delete_articles_cascade() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Create a download
        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Insert articles
        let articles: Vec<NewArticle> = (0..5).map(|i| NewArticle {
            download_id,
            message_id: format!("<article{}@example.com>", i),
            segment_number: i,
            size_bytes: 1024,
        }).collect();
        db.insert_articles_batch(&articles).await.unwrap();

        // Verify articles exist
        let count = db.count_articles(download_id).await.unwrap();
        assert_eq!(count, 5);

        // Delete the download (should cascade delete articles)
        db.delete_download(download_id).await.unwrap();

        // Verify articles were deleted via cascade
        let count = db.count_articles(download_id).await.unwrap();
        assert_eq!(count, 0);

        db.close().await;
    }

    // Password cache tests

    #[tokio::test]
    async fn test_set_and_get_cached_password() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Create a download
        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Initially should have no cached password
        let password = db.get_cached_password(download_id).await.unwrap();
        assert!(password.is_none());

        // Set a password
        db.set_correct_password(download_id, "secret123").await.unwrap();

        // Get the password
        let password = db.get_cached_password(download_id).await.unwrap();
        assert_eq!(password, Some("secret123".to_string()));

        db.close().await;
    }

    #[tokio::test]
    async fn test_update_cached_password() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Create a download
        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Set initial password
        db.set_correct_password(download_id, "password1").await.unwrap();

        // Update to new password (should use ON CONFLICT to update)
        db.set_correct_password(download_id, "password2").await.unwrap();

        // Should have the new password
        let password = db.get_cached_password(download_id).await.unwrap();
        assert_eq!(password, Some("password2".to_string()));

        db.close().await;
    }

    #[tokio::test]
    async fn test_delete_cached_password() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Create a download
        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Set a password
        db.set_correct_password(download_id, "secret").await.unwrap();

        // Verify it exists
        let password = db.get_cached_password(download_id).await.unwrap();
        assert!(password.is_some());

        // Delete the password
        db.delete_cached_password(download_id).await.unwrap();

        // Should be gone
        let password = db.get_cached_password(download_id).await.unwrap();
        assert!(password.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn test_password_cascade_delete() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Create a download
        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Set a password
        db.set_correct_password(download_id, "password123").await.unwrap();

        // Verify password exists
        let password = db.get_cached_password(download_id).await.unwrap();
        assert_eq!(password, Some("password123".to_string()));

        // Delete the download (should cascade delete password)
        db.delete_download(download_id).await.unwrap();

        // Password should be automatically deleted via CASCADE
        let password = db.get_cached_password(download_id).await.unwrap();
        assert!(password.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn test_empty_password() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Create a download
        let new_download = NewDownload {
            name: "Test".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Set an empty password (valid use case for password-less archives)
        db.set_correct_password(download_id, "").await.unwrap();

        // Should be able to retrieve empty password
        let password = db.get_cached_password(download_id).await.unwrap();
        assert_eq!(password, Some("".to_string()));

        db.close().await;
    }

    // Duplicate detection tests

    #[tokio::test]
    async fn test_find_by_nzb_hash() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Insert a download with a specific NZB hash
        let new_download = NewDownload {
            name: "Test Download".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: Some("abc123def456".to_string()),
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Find by NZB hash (should find it)
        let found = db.find_by_nzb_hash("abc123def456").await.unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, download_id);
        assert_eq!(found.name, "Test Download");
        assert_eq!(found.nzb_hash, Some("abc123def456".to_string()));

        // Try to find with non-existent hash (should return None)
        let not_found = db.find_by_nzb_hash("nonexistent").await.unwrap();
        assert!(not_found.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn test_find_by_nzb_hash_multiple() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Insert multiple downloads with different hashes
        let downloads = vec![
            ("Download 1", "hash1"),
            ("Download 2", "hash2"),
            ("Download 3", "hash3"),
        ];

        for (name, hash) in &downloads {
            let new_download = NewDownload {
                name: name.to_string(),
                nzb_path: format!("/{}.nzb", name),
                nzb_meta_name: None,
                nzb_hash: Some(hash.to_string()),
                job_name: None,
                category: None,
                destination: "/downloads".to_string(),
                post_process: 4,
                priority: 0,
                status: 0,
                size_bytes: 1024,
            };
            db.insert_download(&new_download).await.unwrap();
        }

        // Find each by hash
        for (name, hash) in &downloads {
            let found = db.find_by_nzb_hash(hash).await.unwrap();
            assert!(found.is_some());
            assert_eq!(found.unwrap().name, *name);
        }

        db.close().await;
    }

    #[tokio::test]
    async fn test_find_by_name() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Insert a download with a specific name
        let new_download = NewDownload {
            name: "Unique Download Name".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Find by exact name (should find it)
        let found = db.find_by_name("Unique Download Name").await.unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, download_id);
        assert_eq!(found.name, "Unique Download Name");

        // Case-sensitive: different case should not match
        let not_found = db.find_by_name("unique download name").await.unwrap();
        assert!(not_found.is_none());

        // Different name should not match
        let not_found = db.find_by_name("Different Name").await.unwrap();
        assert!(not_found.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn test_find_by_name_returns_first_match() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Insert two downloads with the same name (shouldn't happen in practice but test LIMIT 1)
        let new_download1 = NewDownload {
            name: "Same Name".to_string(),
            nzb_path: "/test1.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        let id1 = db.insert_download(&new_download1).await.unwrap();

        let new_download2 = NewDownload {
            name: "Same Name".to_string(),
            nzb_path: "/test2.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 2048,
        };
        db.insert_download(&new_download2).await.unwrap();

        // Should return the first one (LIMIT 1)
        let found = db.find_by_name("Same Name").await.unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, id1);
        assert_eq!(found.nzb_path, "/test1.nzb");

        db.close().await;
    }

    #[tokio::test]
    async fn test_find_by_job_name() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Insert a download with a deobfuscated job name
        let new_download = NewDownload {
            name: "a3f8d9e1b2c4.nzb".to_string(), // Obfuscated name
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: Some("My Movie 2024".to_string()), // Deobfuscated job name
            category: Some("movies".to_string()),
            destination: "/downloads/movies".to_string(),
            post_process: 4,
            priority: 0,
            status: 4, // Complete
            size_bytes: 1024 * 1024 * 1024,
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Find by job name (should find it)
        let found = db.find_by_job_name("My Movie 2024").await.unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, download_id);
        assert_eq!(found.job_name, Some("My Movie 2024".to_string()));
        assert_eq!(found.name, "a3f8d9e1b2c4.nzb");

        // Try to find with non-existent job name (should return None)
        let not_found = db.find_by_job_name("Different Movie").await.unwrap();
        assert!(not_found.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn test_find_by_job_name_null_handling() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Insert downloads with and without job names
        let download_with_job = NewDownload {
            name: "With Job Name".to_string(),
            nzb_path: "/test1.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: Some("actual_job_name".to_string()),
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        db.insert_download(&download_with_job).await.unwrap();

        let download_without_job = NewDownload {
            name: "Without Job Name".to_string(),
            nzb_path: "/test2.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None, // No job name
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        db.insert_download(&download_without_job).await.unwrap();

        // Find by existing job name
        let found = db.find_by_job_name("actual_job_name").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "With Job Name");

        // Try to find with a job name that doesn't exist (should not match NULL)
        let not_found = db.find_by_job_name("nonexistent").await.unwrap();
        assert!(not_found.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn test_duplicate_detection_priority() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path();
        let db = Database::new(db_path).await.unwrap();

        // Insert a download with all three duplicate detection fields
        let new_download = NewDownload {
            name: "Test.Movie.2024.nzb".to_string(),
            nzb_path: "/test.nzb".to_string(),
            nzb_meta_name: Some("Test Movie 2024".to_string()),
            nzb_hash: Some("hash123abc".to_string()),
            job_name: Some("Test.Movie.2024".to_string()),
            category: Some("movies".to_string()),
            destination: "/downloads/movies".to_string(),
            post_process: 4,
            priority: 0,
            status: 4, // Complete
            size_bytes: 5 * 1024 * 1024 * 1024, // 5 GB
        };
        let download_id = db.insert_download(&new_download).await.unwrap();

        // Test all three detection methods find the same download
        let by_hash = db.find_by_nzb_hash("hash123abc").await.unwrap();
        assert!(by_hash.is_some());
        assert_eq!(by_hash.as_ref().unwrap().id, download_id);

        let by_name = db.find_by_name("Test.Movie.2024.nzb").await.unwrap();
        assert!(by_name.is_some());
        assert_eq!(by_name.as_ref().unwrap().id, download_id);

        let by_job = db.find_by_job_name("Test.Movie.2024").await.unwrap();
        assert!(by_job.is_some());
        assert_eq!(by_job.as_ref().unwrap().id, download_id);

        // All three methods should return the same complete download info
        assert_eq!(by_hash.as_ref().unwrap().name, by_name.as_ref().unwrap().name);
        assert_eq!(by_hash.as_ref().unwrap().category, by_job.as_ref().unwrap().category);
        assert_eq!(by_hash.as_ref().unwrap().status, 4);

        db.close().await;
    }
}
