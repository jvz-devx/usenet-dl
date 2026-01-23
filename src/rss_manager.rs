use crate::config::RssFeedConfig;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::UsenetDownloader;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Manages RSS feed monitoring and auto-downloading
///
/// The RssManager is responsible for:
/// - Periodically checking RSS/Atom feeds for new items
/// - Filtering items based on configured rules
/// - Tracking seen items to prevent duplicates
/// - Automatically downloading matching NZB files
pub struct RssManager {
    /// HTTP client for fetching RSS feeds
    http_client: reqwest::Client,

    /// Database reference for persistence
    db: Arc<Database>,

    /// Reference to the downloader for adding NZBs
    downloader: Arc<UsenetDownloader>,

    /// Configured RSS feeds
    feeds: Vec<RssFeedConfig>,
}

impl RssManager {
    /// Create a new RSS manager
    ///
    /// # Arguments
    /// * `db` - Database instance for persistence
    /// * `downloader` - Reference to the UsenetDownloader instance
    /// * `feeds` - List of RSS feed configurations to monitor
    ///
    /// # Errors
    /// Returns error if the HTTP client cannot be created
    pub fn new(
        db: Arc<Database>,
        downloader: Arc<UsenetDownloader>,
        feeds: Vec<RssFeedConfig>,
    ) -> Result<Self> {
        // Create HTTP client with reasonable timeout (30 seconds)
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("usenet-dl RSS Reader")
            .build()
            .map_err(|e| Error::Other(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            http_client,
            db,
            downloader,
            feeds,
        })
    }

    /// Start the RSS manager
    ///
    /// This method initializes the RSS manager and prepares it for checking feeds.
    /// Currently a no-op, but could be used for initialization in the future.
    pub fn start(&self) -> Result<()> {
        info!("RSS manager initialized with {} feeds", self.feeds.len());
        Ok(())
    }

    /// Stop the RSS manager
    ///
    /// This method stops all feed checking tasks.
    /// Currently a no-op, but will be used when scheduled checking is implemented.
    pub async fn stop(&self) {
        info!("RSS manager stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, RssFeedConfig, RssFilter};
    use crate::types::Priority;
    use std::time::Duration;
    use tempfile::tempdir;

    async fn create_test_setup() -> (Arc<Database>, Arc<UsenetDownloader>) {
        // Create temporary database
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test.db");

        let db = Database::new(&db_path)
            .await
            .expect("Failed to create database");
        let db = Arc::new(db);

        // Create downloader with test config
        let mut config = Config::default();
        config.database_path = db_path;
        let downloader = UsenetDownloader::new(config)
            .await
            .expect("Failed to create downloader");
        let downloader = Arc::new(downloader);

        (db, downloader)
    }

    #[tokio::test]
    async fn test_rss_manager_new() {
        let (db, downloader) = create_test_setup().await;

        let feeds = vec![
            RssFeedConfig {
                url: "https://example.com/rss".to_string(),
                check_interval: Duration::from_secs(900),
                category: Some("movies".to_string()),
                filters: vec![],
                auto_download: true,
                priority: Priority::Normal,
                enabled: true,
            }
        ];

        let manager = RssManager::new(
            db,
            downloader,
            feeds,
        );

        assert!(manager.is_ok(), "RssManager creation should succeed");
        let manager = manager.unwrap();
        assert_eq!(manager.feeds.len(), 1, "Should have 1 feed configured");
    }

    #[tokio::test]
    async fn test_rss_manager_start_stop() {
        let (db, downloader) = create_test_setup().await;

        let manager = RssManager::new(
            db,
            downloader,
            vec![],
        ).expect("Failed to create manager");

        assert!(manager.start().is_ok(), "Start should succeed");
        manager.stop().await;
    }

    #[tokio::test]
    async fn test_rss_manager_with_filters() {
        let (db, downloader) = create_test_setup().await;

        let feeds = vec![
            RssFeedConfig {
                url: "https://example.com/rss".to_string(),
                check_interval: Duration::from_secs(900),
                category: Some("movies".to_string()),
                filters: vec![
                    RssFilter {
                        name: "HD Movies".to_string(),
                        include: vec!["1080p".to_string(), "720p".to_string()],
                        exclude: vec!["cam".to_string(), "ts".to_string()],
                        min_size: Some(1024 * 1024 * 1024), // 1 GB
                        max_size: Some(10 * 1024 * 1024 * 1024), // 10 GB
                        max_age: Some(Duration::from_secs(86400 * 7)), // 7 days
                    }
                ],
                auto_download: true,
                priority: Priority::High,
                enabled: true,
            }
        ];

        let manager = RssManager::new(
            db,
            downloader,
            feeds,
        ).expect("Failed to create manager");

        assert_eq!(manager.feeds.len(), 1, "Should have 1 feed");
        assert_eq!(manager.feeds[0].filters.len(), 1, "Should have 1 filter");
        assert_eq!(manager.feeds[0].filters[0].include.len(), 2, "Should have 2 include patterns");
    }
}
