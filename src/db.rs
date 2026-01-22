//! Database layer for usenet-dl
//!
//! Handles SQLite persistence for downloads, articles, passwords, and history.

use crate::{Error, Result};
use sqlx::{sqlite::SqlitePool, SqliteConnection};
use std::path::Path;

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
}
