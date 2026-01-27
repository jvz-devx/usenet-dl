//! Database lifecycle and schema migrations.

use crate::error::DatabaseError;
use crate::{Error, Result};
use sqlx::sqlite::SqlitePool;
use sqlx::SqliteConnection;
use std::path::Path;

use super::Database;

impl Database {
    /// Create a new database connection
    ///
    /// Creates the database file if it doesn't exist and runs migrations.
    pub async fn new(path: &Path) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                Error::Database(DatabaseError::ConnectionFailed(format!(
                    "Failed to create database directory: {}",
                    e
                )))
            })?;
        }

        // Connect to database
        let connection_string = format!("sqlite:{}?mode=rwc", path.display());
        let pool = SqlitePool::connect(&connection_string).await.map_err(|e| {
            Error::Database(DatabaseError::ConnectionFailed(format!(
                "Failed to connect to database: {}",
                e
            )))
        })?;

        let db = Self { pool };

        // Run migrations
        db.run_migrations().await?;

        Ok(db)
    }

    /// Run database migrations
    async fn run_migrations(&self) -> Result<()> {
        let mut conn = self.pool.acquire().await.map_err(|e| {
            Error::Database(DatabaseError::ConnectionFailed(format!(
                "Failed to acquire connection: {}",
                e
            )))
        })?;

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
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create schema_version table: {}",
                e
            )))
        })?;

        // Check current version
        let current_version: Option<i64> =
            sqlx::query_scalar("SELECT MAX(version) FROM schema_version")
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| {
                    Error::Database(DatabaseError::QueryFailed(format!(
                        "Failed to query schema version: {}",
                        e
                    )))
                })?;

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
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create downloads table: {}",
                e
            )))
        })?;

        // Indexes for downloads
        sqlx::query("CREATE INDEX idx_downloads_status ON downloads(status)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        sqlx::query(
            "CREATE INDEX idx_downloads_priority ON downloads(priority DESC, created_at ASC)",
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create index: {}",
                e
            )))
        })?;

        sqlx::query("CREATE INDEX idx_downloads_nzb_hash ON downloads(nzb_hash)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        sqlx::query("CREATE INDEX idx_downloads_job_name ON downloads(job_name)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

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
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create download_articles table: {}",
                e
            )))
        })?;

        // Indexes for download_articles
        sqlx::query("CREATE INDEX idx_articles_download ON download_articles(download_id)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        sqlx::query("CREATE INDEX idx_articles_status ON download_articles(download_id, status)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

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
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create passwords table: {}",
                e
            )))
        })?;

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
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create processed_nzbs table: {}",
                e
            )))
        })?;

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
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create history table: {}",
                e
            )))
        })?;

        // Index for history
        sqlx::query("CREATE INDEX idx_history_completed ON history(completed_at DESC)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        // Record migration
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (1, ?)")
            .bind(now)
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to record migration: {}",
                    e
                )))
            })?;

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
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create runtime_state table: {}",
                e
            )))
        })?;

        // Initialize shutdown state as unclean (will be set to clean on proper startup)
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            r#"
            INSERT INTO runtime_state (key, value, updated_at)
            VALUES ('clean_shutdown', 'false', ?)
            "#,
        )
        .bind(now)
        .execute(&mut *conn)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to initialize runtime_state: {}",
                e
            )))
        })?;

        // Record migration
        sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (2, ?)")
            .bind(now)
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to record migration: {}",
                    e
                )))
            })?;

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
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create rss_feeds table: {}",
                e
            )))
        })?;

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
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create rss_filters table: {}",
                e
            )))
        })?;

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
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create rss_seen table: {}",
                e
            )))
        })?;

        // Index for rss_seen
        sqlx::query("CREATE INDEX idx_rss_seen_feed ON rss_seen(feed_id)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        // Record migration
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (3, ?)")
            .bind(now)
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to record migration: {}",
                    e
                )))
            })?;

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
}
