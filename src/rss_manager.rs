use crate::config::RssFeedConfig;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::UsenetDownloader;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Represents an item from an RSS or Atom feed
#[derive(Clone, Debug)]
pub struct RssItem {
    /// Item title
    pub title: String,

    /// Item link/URL
    pub link: Option<String>,

    /// Unique identifier (GUID for RSS, id for Atom)
    pub guid: String,

    /// Publication date
    pub pub_date: Option<DateTime<Utc>>,

    /// Item description
    pub description: Option<String>,

    /// Size in bytes (from enclosure or custom tags)
    pub size: Option<u64>,

    /// NZB download URL (from enclosure or link)
    pub nzb_url: Option<String>,
}

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

    /// Check a single RSS/Atom feed for new items
    ///
    /// This method:
    /// 1. Fetches the feed content via HTTP
    /// 2. Attempts to parse as RSS, falls back to Atom if that fails
    /// 3. Extracts items with their metadata (title, link, guid, size, etc.)
    /// 4. Returns a list of parsed items
    ///
    /// # Arguments
    /// * `feed_config` - The feed configuration containing the URL and other settings
    ///
    /// # Returns
    /// A vector of `RssItem` representing the items in the feed
    ///
    /// # Errors
    /// Returns error if:
    /// - HTTP request fails
    /// - Feed cannot be parsed as either RSS or Atom
    /// - Network timeout occurs
    pub async fn check_feed(&self, feed_config: &RssFeedConfig) -> Result<Vec<RssItem>> {
        debug!("Checking RSS feed: {}", feed_config.url);

        // Fetch feed content
        let response = self.http_client
            .get(&feed_config.url)
            .send()
            .await
            .map_err(|e| Error::Other(format!("Failed to fetch RSS feed: {}", e)))?;

        let content = response
            .text()
            .await
            .map_err(|e| Error::Other(format!("Failed to read RSS feed content: {}", e)))?;

        // Try parsing as RSS first, then Atom
        match self.parse_as_rss(&content) {
            Ok(items) => {
                debug!("Successfully parsed as RSS, found {} items", items.len());
                Ok(items)
            }
            Err(rss_err) => {
                debug!("Failed to parse as RSS: {}, trying Atom", rss_err);
                match self.parse_as_atom(&content) {
                    Ok(items) => {
                        debug!("Successfully parsed as Atom, found {} items", items.len());
                        Ok(items)
                    }
                    Err(atom_err) => {
                        Err(Error::Other(format!(
                            "Failed to parse feed as RSS or Atom. RSS error: {}. Atom error: {}",
                            rss_err, atom_err
                        )))
                    }
                }
            }
        }
    }

    /// Parse feed content as RSS
    fn parse_as_rss(&self, content: &str) -> Result<Vec<RssItem>> {
        let channel = content
            .parse::<rss::Channel>()
            .map_err(|e| Error::Other(format!("RSS parse error: {}", e)))?;

        let items = channel
            .items()
            .iter()
            .map(|item| {
                // Extract GUID (prefer guid, fallback to link, then title)
                let guid = item
                    .guid()
                    .map(|g| g.value().to_string())
                    .or_else(|| item.link().map(|l| l.to_string()))
                    .unwrap_or_else(|| item.title().unwrap_or("").to_string());

                // Parse publication date
                let pub_date = item
                    .pub_date()
                    .and_then(|date_str| {
                        chrono::DateTime::parse_from_rfc2822(date_str)
                            .ok()
                            .map(|dt| dt.with_timezone(&Utc))
                    });

                // Extract NZB URL (from enclosure or link)
                let nzb_url = item
                    .enclosure()
                    .map(|enc| enc.url().to_string())
                    .or_else(|| {
                        item.link()
                            .filter(|link| link.ends_with(".nzb"))
                            .map(|l| l.to_string())
                    });

                // Extract size from enclosure
                let size = item.enclosure().and_then(|enc| {
                    enc.length()
                        .parse::<u64>()
                        .ok()
                });

                RssItem {
                    title: item.title().unwrap_or("").to_string(),
                    link: item.link().map(|l| l.to_string()),
                    guid,
                    pub_date,
                    description: item.description().map(|d| d.to_string()),
                    size,
                    nzb_url,
                }
            })
            .collect();

        Ok(items)
    }

    /// Parse feed content as Atom
    fn parse_as_atom(&self, content: &str) -> Result<Vec<RssItem>> {
        let feed = atom_syndication::Feed::read_from(content.as_bytes())
            .map_err(|e| Error::Other(format!("Atom parse error: {}", e)))?;

        let items = feed
            .entries()
            .iter()
            .map(|entry| {
                // GUID is the entry ID
                let guid = entry.id().to_string();

                // Publication date (prefer published, fallback to updated)
                let pub_date = entry
                    .published()
                    .or_else(|| Some(entry.updated()))
                    .and_then(|dt| {
                        chrono::DateTime::parse_from_rfc3339(&dt.to_rfc3339())
                            .ok()
                            .map(|dt| dt.with_timezone(&Utc))
                    });

                // Extract NZB URL from links
                let nzb_url = entry
                    .links()
                    .iter()
                    .find(|link| {
                        link.href().ends_with(".nzb") ||
                        link.mime_type() == Some("application/x-nzb")
                    })
                    .map(|link| link.href().to_string());

                // Try to get the primary link
                let link = entry
                    .links()
                    .first()
                    .map(|link| link.href().to_string());

                // Extract size from enclosure-type links
                let size = entry
                    .links()
                    .iter()
                    .find(|link| link.rel() == "enclosure")
                    .and_then(|link| link.length().and_then(|l| l.parse::<u64>().ok()));

                // Description from summary or content
                let description = entry
                    .summary()
                    .map(|s| s.as_str().to_string())
                    .or_else(|| {
                        entry.content().and_then(|c| {
                            c.value().map(|v| v.to_string())
                        })
                    });

                RssItem {
                    title: entry.title().as_str().to_string(),
                    link,
                    guid,
                    pub_date,
                    description,
                    size,
                    nzb_url,
                }
            })
            .collect();

        Ok(items)
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

    #[tokio::test]
    async fn test_parse_rss_feed() {
        let (db, downloader) = create_test_setup().await;
        let manager = RssManager::new(db, downloader, vec![]).unwrap();

        let rss_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
    <channel>
        <title>Test Feed</title>
        <link>https://example.com</link>
        <description>Test RSS Feed</description>
        <item>
            <title>Test Movie 1080p</title>
            <link>https://example.com/nzb/1</link>
            <guid>https://example.com/nzb/1</guid>
            <pubDate>Mon, 01 Jan 2024 12:00:00 +0000</pubDate>
            <description>A test movie</description>
            <enclosure url="https://example.com/download/1.nzb" length="1073741824" type="application/x-nzb"/>
        </item>
        <item>
            <title>Another Movie 720p</title>
            <link>https://example.com/nzb/2.nzb</link>
            <guid>guid-2</guid>
            <pubDate>Tue, 02 Jan 2024 14:30:00 +0000</pubDate>
        </item>
    </channel>
</rss>"#;

        let items = manager.parse_as_rss(rss_content).expect("Failed to parse RSS");

        assert_eq!(items.len(), 2, "Should parse 2 items");

        // First item
        assert_eq!(items[0].title, "Test Movie 1080p");
        assert_eq!(items[0].link, Some("https://example.com/nzb/1".to_string()));
        assert_eq!(items[0].guid, "https://example.com/nzb/1");
        assert!(items[0].pub_date.is_some());
        assert_eq!(items[0].description, Some("A test movie".to_string()));
        assert_eq!(items[0].size, Some(1073741824));
        assert_eq!(items[0].nzb_url, Some("https://example.com/download/1.nzb".to_string()));

        // Second item (NZB URL from link ending in .nzb)
        assert_eq!(items[1].title, "Another Movie 720p");
        assert_eq!(items[1].guid, "guid-2");
        assert_eq!(items[1].nzb_url, Some("https://example.com/nzb/2.nzb".to_string()));
    }

    #[tokio::test]
    async fn test_parse_atom_feed() {
        let (db, downloader) = create_test_setup().await;
        let manager = RssManager::new(db, downloader, vec![]).unwrap();

        let atom_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
    <title>Test Atom Feed</title>
    <id>https://example.com/atom</id>
    <updated>2024-01-01T12:00:00Z</updated>
    <entry>
        <title>Test Release 1080p</title>
        <id>entry-1</id>
        <updated>2024-01-01T12:00:00Z</updated>
        <published>2024-01-01T10:00:00Z</published>
        <summary>A test release</summary>
        <link href="https://example.com/download/1.nzb" rel="enclosure" type="application/x-nzb" length="2147483648"/>
    </entry>
    <entry>
        <title>Another Release 720p</title>
        <id>entry-2</id>
        <updated>2024-01-02T14:30:00Z</updated>
        <link href="https://example.com/details/2" rel="alternate"/>
        <link href="https://example.com/download/2.nzb" rel="enclosure"/>
    </entry>
</feed>"#;

        let items = manager.parse_as_atom(atom_content).expect("Failed to parse Atom");

        assert_eq!(items.len(), 2, "Should parse 2 items");

        // First item
        assert_eq!(items[0].title, "Test Release 1080p");
        assert_eq!(items[0].guid, "entry-1");
        assert!(items[0].pub_date.is_some());
        assert_eq!(items[0].description, Some("A test release".to_string()));
        assert_eq!(items[0].nzb_url, Some("https://example.com/download/1.nzb".to_string()));
        assert_eq!(items[0].size, Some(2147483648));

        // Second item
        assert_eq!(items[1].title, "Another Release 720p");
        assert_eq!(items[1].guid, "entry-2");
        assert_eq!(items[1].nzb_url, Some("https://example.com/download/2.nzb".to_string()));
    }

    #[tokio::test]
    async fn test_parse_invalid_feed() {
        let (db, downloader) = create_test_setup().await;
        let manager = RssManager::new(db, downloader, vec![]).unwrap();

        let invalid_content = "This is not XML at all!";

        // Should fail to parse as RSS
        let rss_result = manager.parse_as_rss(invalid_content);
        assert!(rss_result.is_err(), "Should fail to parse invalid content as RSS");

        // Should fail to parse as Atom
        let atom_result = manager.parse_as_atom(invalid_content);
        assert!(atom_result.is_err(), "Should fail to parse invalid content as Atom");
    }

    #[tokio::test]
    async fn test_rss_item_guid_fallback() {
        let (db, downloader) = create_test_setup().await;
        let manager = RssManager::new(db, downloader, vec![]).unwrap();

        // RSS item without GUID should use link
        let rss_no_guid = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
    <channel>
        <title>Test</title>
        <item>
            <title>Movie Without GUID</title>
            <link>https://example.com/movie</link>
        </item>
    </channel>
</rss>"#;

        let items = manager.parse_as_rss(rss_no_guid).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].guid, "https://example.com/movie", "Should use link as GUID");

        // RSS item without GUID or link should use title
        let rss_no_guid_no_link = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
    <channel>
        <title>Test</title>
        <item>
            <title>Movie Title Only</title>
        </item>
    </channel>
</rss>"#;

        let items = manager.parse_as_rss(rss_no_guid_no_link).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].guid, "Movie Title Only", "Should use title as GUID");
    }
}
