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
}
