# RSS Feed Manual Testing Guide

This guide demonstrates how to test the RSS feed functionality with a real indexer feed.

## Prerequisites

1. Access to a Usenet indexer with RSS feed support (e.g., NZBGeek, NZBFinder, etc.)
2. RSS feed URL from your indexer (usually found in account settings)
3. Optional: API key if your indexer requires authentication

## Testing Scenarios

### Scenario 1: Basic Feed Fetching

Test that the RSS manager can fetch and parse a real indexer feed.

```rust
use usenet_dl::{Config, UsenetDownloader};
use usenet_dl::config::{RssFeedConfig, RssFilter};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure your indexer's RSS feed
    let feed_url = "https://your-indexer.com/rss?t=5000&apikey=YOUR_API_KEY";

    let mut config = Config::default();
    config.rss_feeds = vec![
        RssFeedConfig {
            url: feed_url.to_string(),
            check_interval: Duration::from_secs(300), // 5 minutes
            category: Some("movies".to_string()),
            filters: vec![],  // No filters - accept everything
            auto_download: false,  // Just monitor, don't download
            priority: usenet_dl::types::Priority::Normal,
            enabled: true,
        }
    ];

    let downloader = UsenetDownloader::new(config).await?;

    // Start RSS scheduler
    let rss_task = downloader.start_rss_scheduler();

    // Let it run for 30 seconds to check the feed once
    tokio::time::sleep(Duration::from_secs(30)).await;

    // Check logs for feed fetch results
    println!("Check logs for RSS feed activity");

    Ok(())
}
```

**Expected Output:**
- Log messages showing feed fetch attempts
- Parse success/failure messages
- Item counts from the feed

### Scenario 2: Feed with Filters

Test filtering functionality with real indexer data.

```rust
let feed_config = RssFeedConfig {
    url: feed_url.to_string(),
    check_interval: Duration::from_secs(300),
    category: Some("movies".to_string()),
    filters: vec![
        RssFilter {
            name: "1080p Movies".to_string(),
            include: vec!["(?i)1080p".to_string()],
            exclude: vec!["(?i)(cam|ts|screener)".to_string()],
            min_size: Some(4_000_000_000),  // 4 GB minimum
            max_size: Some(20_000_000_000), // 20 GB maximum
            max_age: Some(Duration::from_secs(86400 * 7)), // 7 days
        },
    ],
    auto_download: false,
    priority: usenet_dl::types::Priority::High,
    enabled: true,
};
```

**Expected Behavior:**
- Only items matching the filter criteria are marked as seen
- Items not matching filters are ignored
- Exclude patterns override include patterns

### Scenario 3: Auto-Download with Real Indexer

Test automatic NZB downloading from matching feed items.

⚠️ **Warning:** This will attempt to download actual NZBs from your indexer!

```rust
let feed_config = RssFeedConfig {
    url: feed_url.to_string(),
    check_interval: Duration::from_secs(300),
    category: Some("movies".to_string()),
    filters: vec![
        RssFilter {
            name: "Specific Movie".to_string(),
            include: vec!["(?i)ubuntu.*22\\.04".to_string()],  // Safe test case
            exclude: vec![],
            min_size: None,
            max_size: None,
            max_age: None,
        },
    ],
    auto_download: true,  // Enable auto-download
    priority: usenet_dl::types::Priority::High,
    enabled: true,
};
```

**Expected Behavior:**
- Matching items are automatically added to the download queue
- Download progress events are emitted
- Items are marked as seen to prevent re-downloading

### Scenario 4: Multiple Feeds with Different Intervals

Test multiple RSS feeds being monitored simultaneously.

```rust
config.rss_feeds = vec![
    RssFeedConfig {
        url: "https://indexer1.com/rss?t=5000&apikey=KEY1".to_string(),
        check_interval: Duration::from_secs(300),  // 5 minutes
        category: Some("movies".to_string()),
        filters: vec![/* movie filters */],
        auto_download: true,
        priority: usenet_dl::types::Priority::High,
        enabled: true,
    },
    RssFeedConfig {
        url: "https://indexer2.com/rss?t=5030&apikey=KEY2".to_string(),
        check_interval: Duration::from_secs(900),  // 15 minutes
        category: Some("tv".to_string()),
        filters: vec![/* TV show filters */],
        auto_download: true,
        priority: usenet_dl::types::Priority::Normal,
        enabled: true,
    },
];
```

**Expected Behavior:**
- Each feed is checked according to its own interval
- Feeds operate independently
- Different categories are assigned based on feed config

## Verification Steps

### 1. Check Database for Seen Items

```bash
sqlite3 usenet-dl.db "SELECT * FROM rss_seen LIMIT 10;"
```

Should show GUIDs of items that have been processed.

### 2. Check RSS Feeds Table

```bash
sqlite3 usenet-dl.db "SELECT * FROM rss_feeds;"
```

Should show configured RSS feeds with last check timestamps.

### 3. Monitor Logs

Enable debug logging to see detailed RSS activity:

```rust
tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .init();
```

Expected log messages:
- `RSS scheduler started`
- `Checking feed: {url}`
- `Fetched {count} items from feed`
- `Marked {count} items as seen`
- `Auto-downloaded {count} matching items`

### 4. Test API Endpoints

Use the REST API to monitor RSS activity:

```bash
# Get configured feeds
curl http://localhost:6789/api/v1/rss

# Force check a specific feed
curl -X POST http://localhost:6789/api/v1/rss/1/check

# Get download queue (shows auto-downloaded items)
curl http://localhost:6789/api/v1/downloads
```

## Common Issues and Solutions

### Issue: Feed Not Being Checked

**Symptoms:** No log messages about feed checks

**Solutions:**
1. Verify `enabled: true` in feed config
2. Check that RSS scheduler was started: `downloader.start_rss_scheduler()`
3. Verify feed URL is accessible: `curl -I "YOUR_FEED_URL"`

### Issue: All Items Being Skipped

**Symptoms:** "Marked 0 items as seen" in logs

**Solutions:**
1. Items may have already been seen - check `rss_seen` table
2. Filters may be too restrictive - test with no filters first
3. Feed may be returning old items - check `pub_date` of feed items

### Issue: Auto-Download Not Working

**Symptoms:** Items marked as seen but not appearing in download queue

**Solutions:**
1. Verify `auto_download: true` in feed config
2. Check that items have valid `nzb_url` in the feed
3. Verify Usenet servers are configured correctly
4. Check download queue for errors: `SELECT * FROM downloads WHERE status = 5;`

## Performance Considerations

### Recommended Check Intervals

- **Busy indexers:** 15 minutes (900 seconds)
- **Normal use:** 30 minutes (1800 seconds)
- **Low activity:** 60 minutes (3600 seconds)

**Note:** Too frequent checks may violate indexer rate limits!

### Resource Usage

Each active RSS feed:
- Memory: ~50KB per feed (minimal)
- Network: 1 HTTP request per check interval
- Database: ~100 bytes per seen item

## Example: Complete Integration Test

Here's a complete example that tests all RSS functionality:

```rust
use usenet_dl::{Config, UsenetDownloader};
use usenet_dl::config::{RssFeedConfig, RssFilter, ServerConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Enable debug logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Configure Usenet server
    let mut config = Config::default();
    config.servers = vec![
        ServerConfig {
            host: "news.example.com".to_string(),
            port: 563,
            tls: true,
            username: Some("your_username".to_string()),
            password: Some("your_password".to_string()),
            connections: 10,
            priority: 0,
        }
    ];

    // Configure RSS feed
    config.rss_feeds = vec![
        RssFeedConfig {
            url: "https://your-indexer.com/rss?t=5000&apikey=YOUR_KEY".to_string(),
            check_interval: Duration::from_secs(300),
            category: Some("test".to_string()),
            filters: vec![
                RssFilter {
                    name: "Safe Test Files".to_string(),
                    include: vec!["(?i)(ubuntu|debian).*iso".to_string()],
                    exclude: vec!["(?i)sample".to_string()],
                    min_size: Some(1_000_000_000), // 1 GB
                    max_size: None,
                    max_age: Some(Duration::from_secs(86400 * 30)), // 30 days
                },
            ],
            auto_download: true,
            priority: usenet_dl::types::Priority::Normal,
            enabled: true,
        }
    ];

    // Create downloader
    let downloader = UsenetDownloader::new(config).await?;

    // Subscribe to events
    let mut events = downloader.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            println!("Event: {:?}", event);
        }
    });

    // Start RSS scheduler
    let _rss_task = downloader.start_rss_scheduler();

    // Run for 10 minutes to observe behavior
    println!("RSS scheduler started. Monitoring for 10 minutes...");
    tokio::time::sleep(Duration::from_secs(600)).await;

    // Graceful shutdown
    downloader.shutdown().await?;

    Ok(())
}
```

## Testing Checklist

- [ ] Feed fetching works with real indexer URL
- [ ] RSS items are parsed correctly (title, guid, size, nzb_url)
- [ ] Include patterns filter items correctly
- [ ] Exclude patterns override includes
- [ ] Size constraints filter items correctly
- [ ] Age constraints filter items correctly
- [ ] Items are marked as seen in database
- [ ] Duplicate items are skipped (not re-downloaded)
- [ ] Auto-download adds matching items to queue
- [ ] Multiple feeds work independently
- [ ] Different check intervals are respected
- [ ] Disabled feeds are not checked
- [ ] Scheduler stops gracefully on shutdown

## Additional Resources

- [API Documentation](http://localhost:6789/swagger-ui/) - RSS endpoints
- [Database Schema](src/db.rs) - See `rss_feeds` and `rss_seen` tables
- [Architecture Overview](../../docs/architecture.md) - System design and RSS integration

## Support

For issues or questions about RSS functionality:
1. Check the logs for error messages
2. Verify your indexer feed URL is working: `curl -v "YOUR_FEED_URL"`
3. Test with a minimal configuration (no filters, auto_download=false)
4. Review the automated test in `src/rss_manager.rs::test_rss_end_to_end_with_mock_server`
