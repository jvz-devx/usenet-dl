//! RSS feed scheduling and periodic checking
//!
//! This module provides background scheduling for RSS feed checks. The scheduler
//! manages multiple feeds with independent check intervals, automatically fetching
//! and processing new items.
//!
//! # Features
//!
//! - Independent per-feed check intervals
//! - Respects feed enable/disable state
//! - Graceful shutdown handling
//! - Last-check time tracking
//!
//! # Example
//!
//! ```no_run
//! use usenet_dl::{UsenetDownloader, config::Config};
//! use usenet_dl::rss_scheduler::RssScheduler;
//! use usenet_dl::rss_manager::RssManager;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = Config::default();
//! let downloader = Arc::new(UsenetDownloader::new(config).await?);
//! let rss_manager = Arc::new(RssManager::new(
//!     downloader.db.clone(),
//!     downloader.clone(),
//!     downloader.config.rss_feeds.clone(),
//! )?);
//!
//! let scheduler = RssScheduler::new(downloader.clone(), rss_manager);
//!
//! // Run scheduler (blocks until shutdown)
//! tokio::spawn(async move {
//!     scheduler.run().await;
//! });
//! # Ok(())
//! # }
//! ```

use crate::{config::RssFeedConfig, rss_manager::RssManager, UsenetDownloader};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};

/// RSS feed scheduler that periodically checks configured feeds
///
/// The scheduler manages periodic checking of RSS feeds based on their
/// configured check intervals. Each feed is checked independently according
/// to its own interval setting.
pub struct RssScheduler {
    /// Reference to the RSS manager for fetching/processing feeds
    rss_manager: Arc<RssManager>,

    /// Reference to downloader for accessing config and shutdown status
    downloader: Arc<UsenetDownloader>,
}

impl RssScheduler {
    /// Creates a new RSS scheduler
    ///
    /// # Parameters
    /// - `downloader`: Reference to the UsenetDownloader for config access
    /// - `rss_manager`: Reference to the RSS manager for feed operations
    pub fn new(downloader: Arc<UsenetDownloader>, rss_manager: Arc<RssManager>) -> Self {
        Self {
            rss_manager,
            downloader,
        }
    }

    /// Starts the RSS feed checking scheduler
    ///
    /// This runs in a loop checking each feed according to its check_interval.
    /// The scheduler will:
    /// 1. Check if shutdown was requested (via downloader.accepting_new flag)
    /// 2. For each enabled feed:
    ///    - Fetch and parse the feed
    ///    - Process new items (filter, mark as seen, auto-download)
    ///    - Log results
    /// 3. Sleep for a brief interval (30 seconds) before next check
    ///
    /// Each feed tracks its last check time independently. Feeds are checked
    /// when current_time - last_check >= check_interval.
    pub async fn run(self) {
        info!("RSS scheduler started");

        // Track last check time for each feed (indexed by URL for simplicity)
        let mut last_check_times: std::collections::HashMap<String, SystemTime> =
            std::collections::HashMap::new();

        loop {
            // Check for shutdown signal via downloader's accepting_new flag
            if !self.downloader.accepting_new.load(Ordering::SeqCst) {
                info!("RSS scheduler shutting down");
                break;
            }

            // Get current feeds from config
            let feeds: Vec<RssFeedConfig> = self.downloader.config.rss_feeds.clone();

            if feeds.is_empty() {
                debug!("No RSS feeds configured, scheduler idle");
                sleep(Duration::from_secs(30)).await;
                continue;
            }

            let now = SystemTime::now();

            // Check each feed
            for feed in &feeds {
                // Skip disabled feeds
                if !feed.enabled {
                    debug!(url = %feed.url, "RSS feed disabled, skipping");
                    continue;
                }

                // Check if it's time to check this feed
                let should_check = match last_check_times.get(&feed.url) {
                    Some(last_check) => {
                        match now.duration_since(*last_check) {
                            Ok(elapsed) => elapsed >= feed.check_interval,
                            Err(_) => {
                                // Clock went backwards, check anyway
                                warn!(url = %feed.url, "System time went backwards, checking feed");
                                true
                            }
                        }
                    }
                    None => {
                        // Never checked, check now
                        true
                    }
                };

                if !should_check {
                    continue;
                }

                // Check the feed
                debug!(
                    url = %feed.url,
                    interval = ?feed.check_interval,
                    "Checking RSS feed"
                );

                match self.rss_manager.check_feed(feed).await {
                    Ok(items) => {
                        info!(
                            url = %feed.url,
                            item_count = items.len(),
                            "Successfully fetched RSS feed"
                        );

                        // Process items (filter, mark as seen, auto-download)
                        match self.rss_manager.process_feed_items(0, feed, items).await {
                            Ok(downloaded_count) => {
                                if downloaded_count > 0 {
                                    info!(
                                        url = %feed.url,
                                        count = downloaded_count,
                                        "Auto-downloaded items from RSS feed"
                                    );
                                }
                            }
                            Err(e) => {
                                error!(
                                    url = %feed.url,
                                    error = %e,
                                    "Failed to process RSS feed items"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            url = %feed.url,
                            error = %e,
                            "Failed to fetch RSS feed"
                        );
                    }
                }

                // Update last check time
                last_check_times.insert(feed.url.clone(), now);
            }

            // Sleep before next check cycle (1 second)
            // This prevents tight loops while remaining responsive to shutdown
            sleep(Duration::from_secs(1)).await;
        }

        info!("RSS scheduler stopped");
    }
}
