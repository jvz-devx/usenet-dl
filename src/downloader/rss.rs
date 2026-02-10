//! RSS feed management — CRUD operations and feed checking.

use crate::error::{Error, Result};
use std::sync::Arc;

use super::UsenetDownloader;

impl UsenetDownloader {
    /// Get all RSS feeds
    pub async fn get_rss_feeds(&self) -> Result<Vec<crate::config::RssFeedConfig>> {
        use std::time::Duration;

        let feeds = self.db.get_all_rss_feeds().await?;
        let mut result = Vec::new();

        for feed in feeds {
            // Get filters for this feed
            let filter_rows = self.db.get_rss_filters(feed.id).await?;
            let filters = filter_rows
                .into_iter()
                .map(|row| crate::config::RssFilter {
                    name: row.name,
                    include: row
                        .include_patterns
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    exclude: row
                        .exclude_patterns
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    min_size: row.min_size.map(|s| s as u64),
                    max_size: row.max_size.map(|s| s as u64),
                    max_age: row.max_age_secs.map(|s| Duration::from_secs(s as u64)),
                })
                .collect();

            result.push(crate::config::RssFeedConfig {
                url: feed.url,
                check_interval: Duration::from_secs(feed.check_interval_secs as u64),
                category: feed.category,
                filters,
                auto_download: feed.auto_download != 0,
                priority: crate::types::Priority::from_i32(feed.priority),
                enabled: feed.enabled != 0,
            });
        }

        Ok(result)
    }

    /// Get RSS feed by ID with its configuration
    pub async fn get_rss_feed(
        &self,
        id: i64,
    ) -> Result<Option<(i64, String, crate::config::RssFeedConfig)>> {
        use std::time::Duration;

        let feed = match self.db.get_rss_feed(id).await? {
            Some(f) => f,
            None => return Ok(None),
        };

        // Get filters for this feed
        let filter_rows = self.db.get_rss_filters(feed.id).await?;
        let filters = filter_rows
            .into_iter()
            .map(|row| crate::config::RssFilter {
                name: row.name,
                include: row
                    .include_patterns
                    .map(|s| serde_json::from_str(&s).unwrap_or_default())
                    .unwrap_or_default(),
                exclude: row
                    .exclude_patterns
                    .map(|s| serde_json::from_str(&s).unwrap_or_default())
                    .unwrap_or_default(),
                min_size: row.min_size.map(|s| s as u64),
                max_size: row.max_size.map(|s| s as u64),
                max_age: row.max_age_secs.map(|s| Duration::from_secs(s as u64)),
            })
            .collect();

        let config = crate::config::RssFeedConfig {
            url: feed.url,
            check_interval: Duration::from_secs(feed.check_interval_secs as u64),
            category: feed.category,
            filters,
            auto_download: feed.auto_download != 0,
            priority: crate::types::Priority::from_i32(feed.priority),
            enabled: feed.enabled != 0,
        };

        Ok(Some((feed.id, feed.name, config)))
    }

    /// Add a new RSS feed
    pub async fn add_rss_feed(
        &self,
        name: &str,
        config: crate::config::RssFeedConfig,
    ) -> Result<i64> {
        // Insert the feed
        let feed_id = self
            .db
            .insert_rss_feed(crate::db::InsertRssFeedParams {
                name,
                url: &config.url,
                check_interval_secs: config.check_interval.as_secs() as i64,
                category: config.category.as_deref(),
                auto_download: config.auto_download,
                priority: config.priority as i32,
                enabled: config.enabled,
            })
            .await?;

        // Insert filters
        for filter in &config.filters {
            let include_json = if filter.include.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.include)?)
            };

            let exclude_json = if filter.exclude.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.exclude)?)
            };

            self.db
                .insert_rss_filter(crate::db::InsertRssFilterParams {
                    feed_id,
                    name: &filter.name,
                    include_patterns: include_json.as_deref(),
                    exclude_patterns: exclude_json.as_deref(),
                    min_size: filter.min_size.map(|s| s as i64),
                    max_size: filter.max_size.map(|s| s as i64),
                    max_age_secs: filter.max_age.map(|d| d.as_secs() as i64),
                })
                .await?;
        }

        Ok(feed_id)
    }

    /// Update an existing RSS feed
    pub async fn update_rss_feed(
        &self,
        id: i64,
        name: &str,
        config: crate::config::RssFeedConfig,
    ) -> Result<bool> {
        // Update the feed
        let updated = self
            .db
            .update_rss_feed(crate::db::UpdateRssFeedParams {
                id,
                name,
                url: &config.url,
                check_interval_secs: config.check_interval.as_secs() as i64,
                category: config.category.as_deref(),
                auto_download: config.auto_download,
                priority: config.priority as i32,
                enabled: config.enabled,
            })
            .await?;

        if !updated {
            return Ok(false);
        }

        // Delete old filters and insert new ones
        self.db.delete_rss_filters(id).await?;

        for filter in &config.filters {
            let include_json = if filter.include.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.include)?)
            };

            let exclude_json = if filter.exclude.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.exclude)?)
            };

            self.db
                .insert_rss_filter(crate::db::InsertRssFilterParams {
                    feed_id: id,
                    name: &filter.name,
                    include_patterns: include_json.as_deref(),
                    exclude_patterns: exclude_json.as_deref(),
                    min_size: filter.min_size.map(|s| s as i64),
                    max_size: filter.max_size.map(|s| s as i64),
                    max_age_secs: filter.max_age.map(|d| d.as_secs() as i64),
                })
                .await?;
        }

        Ok(true)
    }

    /// Delete an RSS feed
    pub async fn delete_rss_feed(&self, id: i64) -> Result<bool> {
        self.db.delete_rss_feed(id).await
    }

    /// Force check an RSS feed now (for manual triggering via API)
    pub async fn check_rss_feed_now(&self, id: i64) -> Result<usize> {
        // Get the feed configuration
        let (feed_id, _name, config) = match self.get_rss_feed(id).await? {
            Some(f) => f,
            None => return Err(Error::NotFound(format!("RSS feed {} not found", id))),
        };

        // Create a temporary RssManager with just this feed
        let rss_manager = crate::rss_manager::RssManager::new(
            self.db.clone(),
            Arc::new(self.clone()),
            vec![config.clone()],
        )?;

        // Check the feed
        let items = rss_manager.check_feed(&config).await?;

        // Process items (auto-download if enabled)
        let queued = rss_manager
            .process_feed_items(feed_id, &config, items)
            .await?;

        // Update last check status
        self.db.update_rss_feed_check_status(id, None).await?;

        Ok(queued)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, RssFeedConfig, RssFilter};
    use crate::db::Database;
    use crate::types::Priority;
    use std::time::Duration;
    use tempfile::tempdir;

    /// Create a test UsenetDownloader with an ephemeral database.
    /// Returns the downloader and the tempdir (must be kept alive).
    async fn create_test_downloader() -> (UsenetDownloader, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let mut config = Config::default();
        config.persistence.database_path = db_path;
        config.servers = vec![];
        config.download.max_concurrent_downloads = 3;

        let db = Database::new(&config.persistence.database_path)
            .await
            .unwrap();

        let (event_tx, _rx) = tokio::sync::broadcast::channel(1000);
        let nntp_pools = Vec::new();
        let queue =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BinaryHeap::new()));
        let concurrent_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
            config.download.max_concurrent_downloads,
        ));
        let active_downloads =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let speed_limiter =
            crate::speed_limiter::SpeedLimiter::new(config.download.speed_limit_bps);
        let config_arc = std::sync::Arc::new(config.clone());
        let categories = std::sync::Arc::new(tokio::sync::RwLock::new(
            config.persistence.categories.clone(),
        ));
        let schedule_rules = std::sync::Arc::new(tokio::sync::RwLock::new(vec![]));
        let next_schedule_rule_id = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
        let parity_handler: std::sync::Arc<dyn crate::ParityHandler> =
            std::sync::Arc::new(crate::NoOpParityHandler);
        let db_arc = std::sync::Arc::new(db);
        let post_processor = std::sync::Arc::new(crate::post_processing::PostProcessor::new(
            event_tx.clone(),
            config_arc.clone(),
            parity_handler.clone(),
            db_arc.clone(),
        ));

        let queue_state = super::super::QueueState {
            queue,
            concurrent_limit,
            active_downloads,
            accepting_new: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        };
        let runtime_config = super::super::RuntimeConfig {
            categories,
            schedule_rules,
            next_schedule_rule_id,
        };
        let processing = super::super::ProcessingPipeline {
            post_processor,
            parity_handler,
        };

        let downloader = UsenetDownloader {
            db: db_arc,
            event_tx,
            config: config_arc,
            nntp_pools: std::sync::Arc::new(nntp_pools),
            speed_limiter,
            queue_state,
            runtime_config,
            processing,
        };

        (downloader, temp_dir)
    }

    /// Helper: build a feed config with filters for testing
    fn make_feed_config_with_filters() -> RssFeedConfig {
        RssFeedConfig {
            url: "https://example.com/rss.xml".to_string(),
            check_interval: Duration::from_secs(900),
            category: Some("movies".to_string()),
            filters: vec![
                RssFilter {
                    name: "HD Only".to_string(),
                    include: vec!["1080p".to_string(), "2160p".to_string()],
                    exclude: vec!["CAM".to_string()],
                    min_size: Some(500_000_000),
                    max_size: Some(50_000_000_000),
                    max_age: Some(Duration::from_secs(86400)),
                },
                RssFilter {
                    name: "No Spam".to_string(),
                    include: vec![],
                    exclude: vec!["SPAM".to_string(), "XXX".to_string()],
                    min_size: None,
                    max_size: None,
                    max_age: None,
                },
            ],
            auto_download: true,
            priority: Priority::High,
            enabled: true,
        }
    }

    // ---------------------------------------------------------------
    // add_rss_feed
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn add_feed_stores_feed_and_filters_in_db() {
        let (dl, _tmp) = create_test_downloader().await;

        let config = make_feed_config_with_filters();
        let id = dl.add_rss_feed("Test Feed", config).await.unwrap();

        assert!(id > 0, "feed ID should be positive");

        // Verify feed is in DB
        let db_feed = dl.db.get_rss_feed(id).await.unwrap().unwrap();
        assert_eq!(db_feed.name, "Test Feed");
        assert_eq!(db_feed.url, "https://example.com/rss.xml");
        assert_eq!(db_feed.check_interval_secs, 900);
        assert_eq!(db_feed.category.as_deref(), Some("movies"));
        assert_eq!(db_feed.auto_download, 1);
        assert_eq!(db_feed.priority, Priority::High as i32);
        assert_eq!(db_feed.enabled, 1);

        // Verify filters are in DB
        let filters = dl.db.get_rss_filters(id).await.unwrap();
        assert_eq!(filters.len(), 2, "both filters should be stored");
        assert_eq!(filters[0].name, "HD Only");
        assert_eq!(filters[1].name, "No Spam");

        // Verify include/exclude patterns stored as JSON
        let include_json: Vec<String> =
            serde_json::from_str(filters[0].include_patterns.as_ref().unwrap()).unwrap();
        assert_eq!(include_json, vec!["1080p", "2160p"]);

        let exclude_json: Vec<String> =
            serde_json::from_str(filters[0].exclude_patterns.as_ref().unwrap()).unwrap();
        assert_eq!(exclude_json, vec!["CAM"]);

        // Verify size and age stored correctly
        assert_eq!(filters[0].min_size, Some(500_000_000));
        assert_eq!(filters[0].max_size, Some(50_000_000_000));
        assert_eq!(filters[0].max_age_secs, Some(86400));
    }

    #[tokio::test]
    async fn add_feed_with_empty_include_exclude_stores_null_patterns() {
        let (dl, _tmp) = create_test_downloader().await;

        let config = RssFeedConfig {
            url: "https://example.com/feed.xml".to_string(),
            check_interval: Duration::from_secs(300),
            category: None,
            filters: vec![RssFilter {
                name: "Empty patterns".to_string(),
                include: vec![],
                exclude: vec![],
                min_size: None,
                max_size: None,
                max_age: None,
            }],
            auto_download: false,
            priority: Priority::Normal,
            enabled: false,
        };

        let id = dl.add_rss_feed("Empty Feed", config).await.unwrap();

        let filters = dl.db.get_rss_filters(id).await.unwrap();
        assert_eq!(filters.len(), 1);
        assert!(
            filters[0].include_patterns.is_none(),
            "empty include vec should be stored as NULL, not empty JSON array"
        );
        assert!(
            filters[0].exclude_patterns.is_none(),
            "empty exclude vec should be stored as NULL, not empty JSON array"
        );
    }

    // ---------------------------------------------------------------
    // get_rss_feeds (all)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn get_feeds_returns_all_with_reconstructed_configs() {
        let (dl, _tmp) = create_test_downloader().await;

        dl.add_rss_feed(
            "Feed One",
            RssFeedConfig {
                url: "https://one.com/rss".to_string(),
                check_interval: Duration::from_secs(600),
                category: Some("tv".to_string()),
                filters: vec![],
                auto_download: true,
                priority: Priority::Normal,
                enabled: true,
            },
        )
        .await
        .unwrap();

        dl.add_rss_feed(
            "Feed Two",
            RssFeedConfig {
                url: "https://two.com/rss".to_string(),
                check_interval: Duration::from_secs(1800),
                category: None,
                filters: vec![RssFilter {
                    name: "Size Filter".to_string(),
                    include: vec!["x265".to_string()],
                    exclude: vec![],
                    min_size: Some(1_000_000),
                    max_size: None,
                    max_age: Some(Duration::from_secs(7200)),
                }],
                auto_download: false,
                priority: Priority::Low,
                enabled: false,
            },
        )
        .await
        .unwrap();

        let feeds = dl.get_rss_feeds().await.unwrap();

        assert_eq!(feeds.len(), 2, "should return both feeds");

        // Feed One
        assert_eq!(feeds[0].url, "https://one.com/rss");
        assert_eq!(feeds[0].check_interval, Duration::from_secs(600));
        assert_eq!(feeds[0].category.as_deref(), Some("tv"));
        assert!(feeds[0].filters.is_empty());
        assert!(feeds[0].auto_download);
        assert_eq!(feeds[0].priority, Priority::Normal);
        assert!(feeds[0].enabled);

        // Feed Two -- verify filter reconstruction from DB JSON
        assert_eq!(feeds[1].url, "https://two.com/rss");
        assert_eq!(feeds[1].check_interval, Duration::from_secs(1800));
        assert!(feeds[1].category.is_none());
        assert!(!feeds[1].auto_download);
        assert_eq!(feeds[1].priority, Priority::Low);
        assert!(!feeds[1].enabled);
        assert_eq!(feeds[1].filters.len(), 1);

        let f = &feeds[1].filters[0];
        assert_eq!(f.name, "Size Filter");
        assert_eq!(f.include, vec!["x265"]);
        assert!(f.exclude.is_empty());
        assert_eq!(f.min_size, Some(1_000_000));
        assert!(f.max_size.is_none());
        assert_eq!(f.max_age, Some(Duration::from_secs(7200)));
    }

    #[tokio::test]
    async fn get_feeds_returns_empty_when_no_feeds_added() {
        let (dl, _tmp) = create_test_downloader().await;

        let feeds = dl.get_rss_feeds().await.unwrap();
        assert!(feeds.is_empty());
    }

    // ---------------------------------------------------------------
    // get_rss_feed (single)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn get_single_feed_returns_id_name_and_config() {
        let (dl, _tmp) = create_test_downloader().await;

        let config = make_feed_config_with_filters();
        let id = dl.add_rss_feed("My Feed", config.clone()).await.unwrap();

        let result = dl.get_rss_feed(id).await.unwrap();
        assert!(result.is_some(), "feed should exist");

        let (ret_id, ret_name, ret_config) = result.unwrap();
        assert_eq!(ret_id, id);
        assert_eq!(ret_name, "My Feed");
        assert_eq!(ret_config.url, "https://example.com/rss.xml");
        assert_eq!(ret_config.check_interval, Duration::from_secs(900));
        assert_eq!(ret_config.category.as_deref(), Some("movies"));
        assert!(ret_config.auto_download);
        assert_eq!(ret_config.priority, Priority::High);
        assert!(ret_config.enabled);

        // Verify filters round-tripped through DB JSON serialization
        assert_eq!(ret_config.filters.len(), 2);
        assert_eq!(ret_config.filters[0].name, "HD Only");
        assert_eq!(ret_config.filters[0].include, vec!["1080p", "2160p"]);
        assert_eq!(ret_config.filters[0].exclude, vec!["CAM"]);
        assert_eq!(ret_config.filters[0].min_size, Some(500_000_000));
        assert_eq!(ret_config.filters[0].max_size, Some(50_000_000_000));
        assert_eq!(
            ret_config.filters[0].max_age,
            Some(Duration::from_secs(86400))
        );
    }

    #[tokio::test]
    async fn get_nonexistent_feed_returns_none() {
        let (dl, _tmp) = create_test_downloader().await;

        let result = dl.get_rss_feed(99999).await.unwrap();
        assert!(result.is_none(), "non-existent feed should return None");
    }

    // ---------------------------------------------------------------
    // update_rss_feed
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn update_feed_replaces_config_and_filters() {
        let (dl, _tmp) = create_test_downloader().await;

        // Create initial feed with 2 filters
        let original = make_feed_config_with_filters();
        let id = dl.add_rss_feed("Original", original).await.unwrap();

        // Update with completely different config and 1 filter
        let updated_config = RssFeedConfig {
            url: "https://new-url.com/rss".to_string(),
            check_interval: Duration::from_secs(3600),
            category: Some("tv".to_string()),
            filters: vec![RssFilter {
                name: "New Filter".to_string(),
                include: vec!["HDTV".to_string()],
                exclude: vec![],
                min_size: None,
                max_size: None,
                max_age: None,
            }],
            auto_download: false,
            priority: Priority::Low,
            enabled: false,
        };

        let updated = dl
            .update_rss_feed(id, "Renamed", updated_config)
            .await
            .unwrap();
        assert!(updated, "update should succeed for existing feed");

        // Verify via get_rss_feed
        let (_, name, config) = dl.get_rss_feed(id).await.unwrap().unwrap();
        assert_eq!(name, "Renamed");
        assert_eq!(config.url, "https://new-url.com/rss");
        assert_eq!(config.check_interval, Duration::from_secs(3600));
        assert_eq!(config.category.as_deref(), Some("tv"));
        assert!(!config.auto_download);
        assert_eq!(config.priority, Priority::Low);
        assert!(!config.enabled);

        // Old 2 filters should be gone, replaced by 1 new filter
        assert_eq!(
            config.filters.len(),
            1,
            "old filters should be deleted, replaced by new"
        );
        assert_eq!(config.filters[0].name, "New Filter");
        assert_eq!(config.filters[0].include, vec!["HDTV"]);
    }

    #[tokio::test]
    async fn update_nonexistent_feed_returns_false_and_skips_filter_insertion() {
        let (dl, _tmp) = create_test_downloader().await;

        let config = RssFeedConfig {
            url: "https://ghost.com/rss".to_string(),
            check_interval: Duration::from_secs(300),
            category: None,
            filters: vec![RssFilter {
                name: "Should Not Be Inserted".to_string(),
                include: vec!["anything".to_string()],
                exclude: vec![],
                min_size: None,
                max_size: None,
                max_age: None,
            }],
            auto_download: false,
            priority: Priority::Normal,
            enabled: false,
        };

        let updated = dl.update_rss_feed(99999, "Ghost", config).await.unwrap();
        assert!(!updated, "updating non-existent feed should return false");

        // Verify no orphan filters were created
        let filters = dl.db.get_rss_filters(99999).await.unwrap();
        assert!(
            filters.is_empty(),
            "no filters should be inserted when feed update returns false"
        );
    }

    // ---------------------------------------------------------------
    // delete_rss_feed
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn delete_feed_removes_feed_and_cascades_to_filters() {
        let (dl, _tmp) = create_test_downloader().await;

        let config = make_feed_config_with_filters();
        let id = dl.add_rss_feed("Delete Me", config).await.unwrap();

        // Confirm feed and filters exist
        assert!(dl.get_rss_feed(id).await.unwrap().is_some());
        assert_eq!(dl.db.get_rss_filters(id).await.unwrap().len(), 2);

        // Delete
        let deleted = dl.delete_rss_feed(id).await.unwrap();
        assert!(deleted);

        // Feed gone
        assert!(dl.get_rss_feed(id).await.unwrap().is_none());

        // Filters also gone (cascade)
        let filters = dl.db.get_rss_filters(id).await.unwrap();
        assert!(
            filters.is_empty(),
            "cascade delete should remove all filters"
        );
    }

    #[tokio::test]
    async fn delete_nonexistent_feed_returns_false() {
        let (dl, _tmp) = create_test_downloader().await;

        let deleted = dl.delete_rss_feed(99999).await.unwrap();
        assert!(!deleted);
    }

    // ---------------------------------------------------------------
    // Round-trip: add -> get_all -> verify data integrity
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn add_then_get_all_preserves_filter_json_patterns() {
        let (dl, _tmp) = create_test_downloader().await;

        let config = RssFeedConfig {
            url: "https://pattern-test.com/rss".to_string(),
            check_interval: Duration::from_secs(600),
            category: None,
            filters: vec![RssFilter {
                name: "Regex Filter".to_string(),
                include: vec![r"S\d{2}E\d{2}".to_string(), "720p|1080p".to_string()],
                exclude: vec!["REPACK".to_string()],
                min_size: Some(100_000_000),
                max_size: Some(5_000_000_000),
                max_age: Some(Duration::from_secs(172800)),
            }],
            auto_download: true,
            priority: Priority::Force,
            enabled: true,
        };

        dl.add_rss_feed("Pattern Feed", config).await.unwrap();

        let feeds = dl.get_rss_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);

        let f = &feeds[0].filters[0];
        assert_eq!(
            f.include,
            vec![r"S\d{2}E\d{2}", "720p|1080p"],
            "regex include patterns must survive DB round-trip"
        );
        assert_eq!(f.exclude, vec!["REPACK"]);
        assert_eq!(f.min_size, Some(100_000_000));
        assert_eq!(f.max_size, Some(5_000_000_000));
        assert_eq!(f.max_age, Some(Duration::from_secs(172800)));
        assert_eq!(feeds[0].priority, Priority::Force);
    }
}
