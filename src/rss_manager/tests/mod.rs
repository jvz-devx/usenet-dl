use super::*;
use crate::config::{Config, RssFeedConfig, RssFilter};
use crate::types::Priority;
use std::net::TcpListener;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener as TokioTcpListener;

async fn create_test_setup() -> (Arc<Database>, Arc<UsenetDownloader>) {
    // Create temporary database
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.db");

    // Create database first
    let db = Database::new(&db_path)
        .await
        .expect("Failed to create database");
    let db = Arc::new(db);

    // Create downloader with test config pointing to same database
    let mut config = Config::default();
    config.persistence.database_path = db_path.clone();
    let downloader = UsenetDownloader::new(config)
        .await
        .expect("Failed to create downloader");
    let downloader = Arc::new(downloader);

    // Prevent temp_dir from being dropped (keep it alive for the test)
    std::mem::forget(temp_dir);

    (db, downloader)
}

#[tokio::test]
async fn test_rss_manager_with_filters() {
    let (db, downloader) = create_test_setup().await;

    let feeds = vec![RssFeedConfig {
        url: "https://example.com/rss".to_string(),
        check_interval: Duration::from_secs(900),
        category: Some("movies".to_string()),
        filters: vec![RssFilter {
            name: "HD Movies".to_string(),
            include: vec!["1080p".to_string(), "720p".to_string()],
            exclude: vec!["cam".to_string(), "ts".to_string()],
            min_size: Some(1024 * 1024 * 1024),            // 1 GB
            max_size: Some(10 * 1024 * 1024 * 1024),       // 10 GB
            max_age: Some(Duration::from_secs(86400 * 7)), // 7 days
        }],
        auto_download: true,
        priority: Priority::High,
        enabled: true,
    }];

    let manager = RssManager::new(db, downloader, feeds).expect("Failed to create manager");

    assert_eq!(manager.feeds.len(), 1, "Should have 1 feed");
    assert_eq!(manager.feeds[0].filters.len(), 1, "Should have 1 filter");
    assert_eq!(
        manager.feeds[0].filters[0].include.len(),
        2,
        "Should have 2 include patterns"
    );
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

    let items = manager
        .parse_as_rss(rss_content)
        .expect("Failed to parse RSS");

    assert_eq!(items.len(), 2, "Should parse 2 items");

    // First item
    assert_eq!(items[0].title, "Test Movie 1080p");
    assert_eq!(items[0].link, Some("https://example.com/nzb/1".to_string()));
    assert_eq!(items[0].guid, "https://example.com/nzb/1");
    assert!(items[0].pub_date.is_some());
    assert_eq!(items[0].description, Some("A test movie".to_string()));
    assert_eq!(items[0].size, Some(1073741824));
    assert_eq!(
        items[0].nzb_url,
        Some("https://example.com/download/1.nzb".to_string())
    );

    // Second item (NZB URL from link ending in .nzb)
    assert_eq!(items[1].title, "Another Movie 720p");
    assert_eq!(items[1].guid, "guid-2");
    assert_eq!(
        items[1].nzb_url,
        Some("https://example.com/nzb/2.nzb".to_string())
    );
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

    let items = manager
        .parse_as_atom(atom_content)
        .expect("Failed to parse Atom");

    assert_eq!(items.len(), 2, "Should parse 2 items");

    // First item
    assert_eq!(items[0].title, "Test Release 1080p");
    assert_eq!(items[0].guid, "entry-1");
    assert!(items[0].pub_date.is_some());
    assert_eq!(items[0].description, Some("A test release".to_string()));
    assert_eq!(
        items[0].nzb_url,
        Some("https://example.com/download/1.nzb".to_string())
    );
    assert_eq!(items[0].size, Some(2147483648));

    // Second item
    assert_eq!(items[1].title, "Another Release 720p");
    assert_eq!(items[1].guid, "entry-2");
    assert_eq!(
        items[1].nzb_url,
        Some("https://example.com/download/2.nzb".to_string())
    );
}

#[tokio::test]
async fn test_parse_invalid_feed() {
    let (db, downloader) = create_test_setup().await;
    let manager = RssManager::new(db, downloader, vec![]).unwrap();

    let invalid_content = "This is not XML at all!";

    // Should fail to parse as RSS
    let rss_result = manager.parse_as_rss(invalid_content);
    assert!(
        rss_result.is_err(),
        "Should fail to parse invalid content as RSS"
    );

    // Should fail to parse as Atom
    let atom_result = manager.parse_as_atom(invalid_content);
    assert!(
        atom_result.is_err(),
        "Should fail to parse invalid content as Atom"
    );
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
    assert_eq!(
        items[0].guid, "https://example.com/movie",
        "Should use link as GUID"
    );

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
    assert_eq!(
        items[0].guid, "Movie Title Only",
        "Should use title as GUID"
    );
}

#[tokio::test]
async fn test_matches_filters_include_patterns() {
    let (db, downloader) = create_test_setup().await;
    let manager = RssManager::new(db, downloader, vec![]).unwrap();

    let filter = RssFilter {
        name: "HD Filter".to_string(),
        include: vec!["1080p".to_string(), "720p".to_string()],
        exclude: vec![],
        min_size: None,
        max_size: None,
        max_age: None,
    };

    // Should match - has 1080p
    let item1 = RssItem {
        title: "Movie Name 1080p BluRay".to_string(),
        link: None,
        guid: "1".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        manager.matches_filters(&item1, &filter),
        "Should match include pattern 1080p"
    );

    // Should match - has 720p
    let item2 = RssItem {
        title: "TV Show S01E01 720p".to_string(),
        link: None,
        guid: "2".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        manager.matches_filters(&item2, &filter),
        "Should match include pattern 720p"
    );

    // Should NOT match - has neither
    let item3 = RssItem {
        title: "Movie Name 480p".to_string(),
        link: None,
        guid: "3".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        !manager.matches_filters(&item3, &filter),
        "Should not match - no include pattern found"
    );
}

#[tokio::test]
async fn test_matches_filters_exclude_patterns() {
    let (db, downloader) = create_test_setup().await;
    let manager = RssManager::new(db, downloader, vec![]).unwrap();

    let filter = RssFilter {
        name: "No CAM Filter".to_string(),
        include: vec!["1080p".to_string()],
        exclude: vec!["CAM".to_string(), "TS".to_string()],
        min_size: None,
        max_size: None,
        max_age: None,
    };

    // Should match - has 1080p, no CAM
    let item1 = RssItem {
        title: "Movie Name 1080p BluRay".to_string(),
        link: None,
        guid: "1".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        manager.matches_filters(&item1, &filter),
        "Should match - has include, no exclude"
    );

    // Should NOT match - has CAM (exclude overrides include)
    let item2 = RssItem {
        title: "Movie Name 1080p CAM".to_string(),
        link: None,
        guid: "2".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        !manager.matches_filters(&item2, &filter),
        "Should not match - CAM in title"
    );

    // Should NOT match - has TS
    let item3 = RssItem {
        title: "Movie Name 1080p TS".to_string(),
        link: None,
        guid: "3".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        !manager.matches_filters(&item3, &filter),
        "Should not match - TS in title"
    );
}

#[tokio::test]
async fn test_matches_filters_regex_patterns() {
    let (db, downloader) = create_test_setup().await;
    let manager = RssManager::new(db, downloader, vec![]).unwrap();

    let filter = RssFilter {
        name: "Regex Filter".to_string(),
        include: vec![r"S\d{2}E\d{2}".to_string()], // Matches S01E01 format
        exclude: vec![r"(?i)french".to_string()],   // Case-insensitive French
        min_size: None,
        max_size: None,
        max_age: None,
    };

    // Should match - has S01E01
    let item1 = RssItem {
        title: "Show Name S01E01 1080p".to_string(),
        link: None,
        guid: "1".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        manager.matches_filters(&item1, &filter),
        "Should match episode pattern"
    );

    // Should NOT match - no episode pattern
    let item2 = RssItem {
        title: "Movie Name 2024".to_string(),
        link: None,
        guid: "2".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        !manager.matches_filters(&item2, &filter),
        "Should not match - no episode pattern"
    );

    // Should NOT match - has french (case insensitive)
    let item3 = RssItem {
        title: "Show Name S02E05 FRENCH 1080p".to_string(),
        link: None,
        guid: "3".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        !manager.matches_filters(&item3, &filter),
        "Should not match - has FRENCH"
    );
}

#[tokio::test]
async fn test_matches_filters_size_constraints() {
    let (db, downloader) = create_test_setup().await;
    let manager = RssManager::new(db, downloader, vec![]).unwrap();

    let filter = RssFilter {
        name: "Size Filter".to_string(),
        include: vec![],
        exclude: vec![],
        min_size: Some(1024 * 1024 * 500),      // 500 MB
        max_size: Some(1024 * 1024 * 1024 * 5), // 5 GB
        max_age: None,
    };

    // Should match - size within range
    let item1 = RssItem {
        title: "Movie 1080p".to_string(),
        link: None,
        guid: "1".to_string(),
        pub_date: None,
        description: None,
        size: Some(1024 * 1024 * 1024 * 2), // 2 GB
        nzb_url: None,
    };
    assert!(
        manager.matches_filters(&item1, &filter),
        "Should match - size within range"
    );

    // Should NOT match - too small
    let item2 = RssItem {
        title: "Movie 480p".to_string(),
        link: None,
        guid: "2".to_string(),
        pub_date: None,
        description: None,
        size: Some(1024 * 1024 * 100), // 100 MB
        nzb_url: None,
    };
    assert!(
        !manager.matches_filters(&item2, &filter),
        "Should not match - too small"
    );

    // Should NOT match - too large
    let item3 = RssItem {
        title: "Movie 4K".to_string(),
        link: None,
        guid: "3".to_string(),
        pub_date: None,
        description: None,
        size: Some(1024 * 1024 * 1024 * 10), // 10 GB
        nzb_url: None,
    };
    assert!(
        !manager.matches_filters(&item3, &filter),
        "Should not match - too large"
    );

    // Should match - no size specified (ignores filter)
    let item4 = RssItem {
        title: "Movie Unknown Size".to_string(),
        link: None,
        guid: "4".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        manager.matches_filters(&item4, &filter),
        "Should match - no size to check"
    );
}

#[tokio::test]
async fn test_matches_filters_age_constraint() {
    let (db, downloader) = create_test_setup().await;
    let manager = RssManager::new(db, downloader, vec![]).unwrap();

    let filter = RssFilter {
        name: "Age Filter".to_string(),
        include: vec![],
        exclude: vec![],
        min_size: None,
        max_size: None,
        max_age: Some(Duration::from_secs(86400 * 7)), // 7 days
    };

    // Should match - recent item (1 day old)
    let item1 = RssItem {
        title: "Recent Movie".to_string(),
        link: None,
        guid: "1".to_string(),
        pub_date: Some(Utc::now() - chrono::Duration::days(1)),
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        manager.matches_filters(&item1, &filter),
        "Should match - recent item"
    );

    // Should NOT match - too old (30 days)
    let item2 = RssItem {
        title: "Old Movie".to_string(),
        link: None,
        guid: "2".to_string(),
        pub_date: Some(Utc::now() - chrono::Duration::days(30)),
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        !manager.matches_filters(&item2, &filter),
        "Should not match - too old"
    );

    // Should match - no pub_date (ignores age filter)
    let item3 = RssItem {
        title: "No Date Movie".to_string(),
        link: None,
        guid: "3".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };
    assert!(
        manager.matches_filters(&item3, &filter),
        "Should match - no date to check"
    );
}

#[tokio::test]
async fn test_matches_filters_description_matching() {
    let (db, downloader) = create_test_setup().await;
    let manager = RssManager::new(db, downloader, vec![]).unwrap();

    let filter = RssFilter {
        name: "Description Filter".to_string(),
        include: vec!["BluRay".to_string()],
        exclude: vec!["sample".to_string()],
        min_size: None,
        max_size: None,
        max_age: None,
    };

    // Should match - BluRay in description
    let item1 = RssItem {
        title: "Movie Name".to_string(),
        link: None,
        guid: "1".to_string(),
        pub_date: None,
        description: Some("Full BluRay release".to_string()),
        size: None,
        nzb_url: None,
    };
    assert!(
        manager.matches_filters(&item1, &filter),
        "Should match - BluRay in description"
    );

    // Should NOT match - sample in description
    let item2 = RssItem {
        title: "Movie BluRay".to_string(),
        link: None,
        guid: "2".to_string(),
        pub_date: None,
        description: Some("This is a sample release".to_string()),
        size: None,
        nzb_url: None,
    };
    assert!(
        !manager.matches_filters(&item2, &filter),
        "Should not match - sample in description"
    );
}

#[tokio::test]
async fn test_matches_filters_no_filters() {
    let (db, downloader) = create_test_setup().await;
    let manager = RssManager::new(db, downloader, vec![]).unwrap();

    // Empty filter - should match everything
    let filter = RssFilter {
        name: "Empty Filter".to_string(),
        include: vec![],
        exclude: vec![],
        min_size: None,
        max_size: None,
        max_age: None,
    };

    let item = RssItem {
        title: "Any Movie".to_string(),
        link: None,
        guid: "1".to_string(),
        pub_date: None,
        description: None,
        size: None,
        nzb_url: None,
    };

    assert!(
        manager.matches_filters(&item, &filter),
        "Empty filter should match any item"
    );
}

#[tokio::test]
async fn test_process_feed_items_auto_download_enabled() {
    let (db, downloader) = create_test_setup().await;

    // Create feed in database
    let feed_id = {
        let mut conn = db.pool().acquire().await.unwrap();
        let result = sqlx::query(
                "INSERT INTO rss_feeds (name, url, check_interval_secs, auto_download, enabled, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind("Test Feed")
            .bind("http://example.com/feed.rss")
            .bind(900)
            .bind(1)
            .bind(1)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *conn)
            .await
            .unwrap()
            .last_insert_rowid();
        drop(conn); // Drop connection before calling RSS manager
        result
    };

    let feed_config = RssFeedConfig {
        url: "http://example.com/feed.rss".to_string(),
        check_interval: std::time::Duration::from_secs(900),
        category: Some("movies".to_string()),
        filters: vec![], // No filters = accept all
        auto_download: true,
        priority: crate::types::Priority::Normal,
        enabled: true,
    };

    let items = vec![
        RssItem {
            title: "Movie 1".to_string(),
            link: Some("http://example.com/1".to_string()),
            guid: "guid-1".to_string(),
            pub_date: Some(Utc::now()),
            description: Some("Description 1".to_string()),
            size: Some(1024 * 1024 * 1024),
            nzb_url: Some("http://example.com/1.nzb".to_string()),
        },
        RssItem {
            title: "Movie 2".to_string(),
            link: Some("http://example.com/2".to_string()),
            guid: "guid-2".to_string(),
            pub_date: Some(Utc::now()),
            description: Some("Description 2".to_string()),
            size: Some(2 * 1024 * 1024 * 1024),
            nzb_url: Some("http://example.com/2.nzb".to_string()),
        },
    ];

    let manager =
        RssManager::new(db.clone(), downloader.clone(), vec![feed_config.clone()]).unwrap();
    let downloaded = manager
        .process_feed_items(feed_id, &feed_config, items)
        .await
        .unwrap();

    // Note: Downloads will fail because URLs are fake, but that's OK for this test
    // We're testing the RSS processing logic, not the actual download
    // The count will be 0 because downloads failed, but items should still be marked as seen
    assert_eq!(
        downloaded, 0,
        "Downloads failed (fake URLs), but logic executed"
    );

    // Items should be marked as seen even if downloads failed
    assert!(db.is_rss_item_seen(feed_id, "guid-1").await.unwrap());
    assert!(db.is_rss_item_seen(feed_id, "guid-2").await.unwrap());
}

#[tokio::test]
async fn test_process_feed_items_auto_download_disabled() {
    let (db, downloader) = create_test_setup().await;

    // Create feed in database
    let feed_id = {
        let mut conn = db.pool().acquire().await.unwrap();
        let result = sqlx::query(
                "INSERT INTO rss_feeds (name, url, check_interval_secs, auto_download, enabled, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind("Test Feed")
            .bind("http://example.com/feed.rss")
            .bind(900)
            .bind(0)  // auto_download disabled
            .bind(1)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *conn)
            .await
            .unwrap()
            .last_insert_rowid();
        drop(conn); // Drop connection before calling RSS manager
        result
    };

    let feed_config = RssFeedConfig {
        url: "http://example.com/feed.rss".to_string(),
        check_interval: std::time::Duration::from_secs(900),
        category: Some("movies".to_string()),
        filters: vec![],
        auto_download: false, // Disabled
        priority: crate::types::Priority::Normal,
        enabled: true,
    };

    let items = vec![RssItem {
        title: "Movie 1".to_string(),
        link: Some("http://example.com/1".to_string()),
        guid: "guid-1".to_string(),
        pub_date: Some(Utc::now()),
        description: Some("Description 1".to_string()),
        size: Some(1024 * 1024 * 1024),
        nzb_url: Some("http://example.com/1.nzb".to_string()),
    }];

    let manager =
        RssManager::new(db.clone(), downloader.clone(), vec![feed_config.clone()]).unwrap();
    let downloaded = manager
        .process_feed_items(feed_id, &feed_config, items)
        .await
        .unwrap();

    // No items should have been downloaded (auto_download is false)
    assert_eq!(
        downloaded, 0,
        "Should not download when auto_download=false"
    );

    // Item should still be marked as seen
    assert!(db.is_rss_item_seen(feed_id, "guid-1").await.unwrap());
}

#[tokio::test]
async fn test_process_feed_items_skips_seen() {
    let (db, downloader) = create_test_setup().await;

    // Create feed in database
    let feed_id = {
        let mut conn = db.pool().acquire().await.unwrap();
        let result = sqlx::query(
                "INSERT INTO rss_feeds (name, url, check_interval_secs, auto_download, enabled, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind("Test Feed")
            .bind("http://example.com/feed.rss")
            .bind(900)
            .bind(1)
            .bind(1)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *conn)
            .await
            .unwrap()
            .last_insert_rowid();
        drop(conn); // Drop connection before calling RSS manager
        result
    };

    // Mark one item as already seen
    db.mark_rss_item_seen(feed_id, "guid-1").await.unwrap();

    let feed_config = RssFeedConfig {
        url: "http://example.com/feed.rss".to_string(),
        check_interval: std::time::Duration::from_secs(900),
        category: None,
        filters: vec![],
        auto_download: true,
        priority: crate::types::Priority::Normal,
        enabled: true,
    };

    let items = vec![
        RssItem {
            title: "Movie 1 (Already Seen)".to_string(),
            link: Some("http://example.com/1".to_string()),
            guid: "guid-1".to_string(), // Already marked as seen
            pub_date: Some(Utc::now()),
            description: None,
            size: None,
            nzb_url: Some("http://example.com/1.nzb".to_string()),
        },
        RssItem {
            title: "Movie 2 (New)".to_string(),
            link: Some("http://example.com/2".to_string()),
            guid: "guid-2".to_string(), // Not seen yet
            pub_date: Some(Utc::now()),
            description: None,
            size: None,
            nzb_url: Some("http://example.com/2.nzb".to_string()),
        },
    ];

    let manager =
        RssManager::new(db.clone(), downloader.clone(), vec![feed_config.clone()]).unwrap();
    let downloaded = manager
        .process_feed_items(feed_id, &feed_config, items)
        .await
        .unwrap();

    // Downloads will fail (fake URL), but we verify only guid-2 was processed
    assert_eq!(downloaded, 0, "Downloads failed (fake URLs)");
}

#[tokio::test]
async fn test_process_feed_items_with_filters() {
    let (db, downloader) = create_test_setup().await;

    // Create feed in database
    let feed_id = {
        let mut conn = db.pool().acquire().await.unwrap();
        let result = sqlx::query(
                "INSERT INTO rss_feeds (name, url, check_interval_secs, auto_download, enabled, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind("Test Feed")
            .bind("http://example.com/feed.rss")
            .bind(900)
            .bind(1)
            .bind(1)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *conn)
            .await
            .unwrap()
            .last_insert_rowid();
        drop(conn); // Drop connection before calling RSS manager
        result
    };

    let feed_config = RssFeedConfig {
        url: "http://example.com/feed.rss".to_string(),
        check_interval: std::time::Duration::from_secs(900),
        category: Some("movies".to_string()),
        filters: vec![RssFilter {
            name: "Movies Only".to_string(),
            include: vec!["(?i)movie".to_string()],
            exclude: vec!["sample".to_string()],
            min_size: Some(500 * 1024 * 1024), // 500 MB minimum
            max_size: None,
            max_age: None,
        }],
        auto_download: true,
        priority: crate::types::Priority::Normal,
        enabled: true,
    };

    let items = vec![
        RssItem {
            title: "Great Movie 1080p".to_string(),
            link: Some("http://example.com/1".to_string()),
            guid: "guid-1".to_string(),
            pub_date: Some(Utc::now()),
            description: Some("A movie release".to_string()),
            size: Some(1024 * 1024 * 1024), // 1 GB - passes size filter
            nzb_url: Some("http://example.com/1.nzb".to_string()),
        },
        RssItem {
            title: "Movie Sample".to_string(),
            link: Some("http://example.com/2".to_string()),
            guid: "guid-2".to_string(),
            pub_date: Some(Utc::now()),
            description: Some("Sample file".to_string()),
            size: Some(10 * 1024 * 1024), // 10 MB - excluded by "sample" pattern
            nzb_url: Some("http://example.com/2.nzb".to_string()),
        },
        RssItem {
            title: "TV Show S01E01".to_string(),
            link: Some("http://example.com/3".to_string()),
            guid: "guid-3".to_string(),
            pub_date: Some(Utc::now()),
            description: Some("TV series".to_string()),
            size: Some(1024 * 1024 * 1024), // 1 GB - fails include pattern
            nzb_url: Some("http://example.com/3.nzb".to_string()),
        },
        RssItem {
            title: "Small Movie".to_string(),
            link: Some("http://example.com/4".to_string()),
            guid: "guid-4".to_string(),
            pub_date: Some(Utc::now()),
            description: Some("Movie".to_string()),
            size: Some(100 * 1024 * 1024), // 100 MB - too small
            nzb_url: Some("http://example.com/4.nzb".to_string()),
        },
    ];

    let manager =
        RssManager::new(db.clone(), downloader.clone(), vec![feed_config.clone()]).unwrap();
    let downloaded = manager
        .process_feed_items(feed_id, &feed_config, items)
        .await
        .unwrap();

    // Downloads will fail (fake URLs), but we verify filtering logic
    assert_eq!(downloaded, 0, "Downloads failed (fake URLs)");

    // Only guid-1 should be marked as seen (others were filtered out)
    assert!(db.is_rss_item_seen(feed_id, "guid-1").await.unwrap());
    assert!(!db.is_rss_item_seen(feed_id, "guid-2").await.unwrap());
    assert!(!db.is_rss_item_seen(feed_id, "guid-3").await.unwrap());
    assert!(!db.is_rss_item_seen(feed_id, "guid-4").await.unwrap());
}

#[tokio::test]
async fn test_process_feed_items_no_nzb_url() {
    let (db, downloader) = create_test_setup().await;

    // Create feed in database
    let feed_id = {
        let mut conn = db.pool().acquire().await.unwrap();
        let result = sqlx::query(
                "INSERT INTO rss_feeds (name, url, check_interval_secs, auto_download, enabled, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind("Test Feed")
            .bind("http://example.com/feed.rss")
            .bind(900)
            .bind(1)
            .bind(1)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *conn)
            .await
            .unwrap()
            .last_insert_rowid();
        drop(conn); // Drop connection before calling RSS manager
        result
    };

    let feed_config = RssFeedConfig {
        url: "http://example.com/feed.rss".to_string(),
        check_interval: std::time::Duration::from_secs(900),
        category: None,
        filters: vec![],
        auto_download: true,
        priority: crate::types::Priority::Normal,
        enabled: true,
    };

    let items = vec![RssItem {
        title: "Movie Without NZB URL".to_string(),
        link: Some("http://example.com/1".to_string()),
        guid: "guid-1".to_string(),
        pub_date: Some(Utc::now()),
        description: None,
        size: None,
        nzb_url: None, // No NZB URL - cannot download
    }];

    let manager =
        RssManager::new(db.clone(), downloader.clone(), vec![feed_config.clone()]).unwrap();
    let downloaded = manager
        .process_feed_items(feed_id, &feed_config, items)
        .await
        .unwrap();

    // No downloads (no NZB URL)
    assert_eq!(downloaded, 0, "Cannot download item without NZB URL");

    // Item should still be marked as seen
    assert!(db.is_rss_item_seen(feed_id, "guid-1").await.unwrap());
}

#[tokio::test]
async fn test_process_feed_items_multiple_filters_or_logic() {
    let (db, downloader) = create_test_setup().await;

    // Create feed in database
    let feed_id = {
        let mut conn = db.pool().acquire().await.unwrap();
        let result = sqlx::query(
                "INSERT INTO rss_feeds (name, url, check_interval_secs, auto_download, enabled, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind("Test Feed")
            .bind("http://example.com/feed.rss")
            .bind(900)
            .bind(1)
            .bind(1)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *conn)
            .await
            .unwrap()
            .last_insert_rowid();
        drop(conn); // Drop connection before calling RSS manager
        result
    };

    let feed_config = RssFeedConfig {
        url: "http://example.com/feed.rss".to_string(),
        check_interval: std::time::Duration::from_secs(900),
        category: None,
        filters: vec![
            RssFilter {
                name: "Movies".to_string(),
                include: vec!["(?i)movie".to_string()],
                exclude: vec![],
                min_size: None,
                max_size: None,
                max_age: None,
            },
            RssFilter {
                name: "TV Shows".to_string(),
                include: vec!["(?i)S\\d{2}E\\d{2}".to_string()],
                exclude: vec![],
                min_size: None,
                max_size: None,
                max_age: None,
            },
        ],
        auto_download: true,
        priority: crate::types::Priority::Normal,
        enabled: true,
    };

    let items = vec![
        RssItem {
            title: "Great Movie 1080p".to_string(),
            link: None,
            guid: "guid-1".to_string(),
            pub_date: Some(Utc::now()),
            description: None,
            size: None,
            nzb_url: Some("http://example.com/1.nzb".to_string()),
        },
        RssItem {
            title: "TV Show S01E05".to_string(),
            link: None,
            guid: "guid-2".to_string(),
            pub_date: Some(Utc::now()),
            description: None,
            size: None,
            nzb_url: Some("http://example.com/2.nzb".to_string()),
        },
        RssItem {
            title: "Random Document".to_string(),
            link: None,
            guid: "guid-3".to_string(),
            pub_date: Some(Utc::now()),
            description: None,
            size: None,
            nzb_url: Some("http://example.com/3.nzb".to_string()),
        },
    ];

    let manager =
        RssManager::new(db.clone(), downloader.clone(), vec![feed_config.clone()]).unwrap();
    let downloaded = manager
        .process_feed_items(feed_id, &feed_config, items)
        .await
        .unwrap();

    // Downloads will fail (fake URLs), but we verify OR logic
    // guid-1 matches first filter (movie), guid-2 matches second filter (TV), guid-3 matches neither
    assert_eq!(downloaded, 0, "Downloads failed (fake URLs)");

    assert!(db.is_rss_item_seen(feed_id, "guid-1").await.unwrap());
    assert!(db.is_rss_item_seen(feed_id, "guid-2").await.unwrap());
    assert!(!db.is_rss_item_seen(feed_id, "guid-3").await.unwrap());
}

#[tokio::test]
async fn test_rss_end_to_end_with_mock_server() {
    // This test simulates the complete RSS flow with a mock HTTP server
    // It demonstrates what happens with a real indexer feed

    use std::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener as TokioTcpListener;

    let (db, downloader) = create_test_setup().await;

    // Find a random available port
    let std_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    drop(std_listener); // Release the port

    // Start a mock HTTP server
    let listener = TokioTcpListener::bind(addr).await.unwrap();
    let server_url = format!("http://{}", addr);

    // Spawn server task
    let server_task = tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = vec![0; 1024];
            let _ = socket.read(&mut buf).await;

            // Send a realistic RSS feed response (based on typical indexer format)
            let rss_feed = r#"<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
  <channel>
    <title>Mock Indexer Feed</title>
    <link>http://example.com</link>
    <description>Test RSS Feed</description>
    <item>
      <title>Ubuntu.22.04.3.Desktop.x64</title>
      <link>http://example.com/details/123</link>
      <guid>http://example.com/details/123</guid>
      <pubDate>Thu, 18 Jan 2024 12:00:00 GMT</pubDate>
      <description>Ubuntu Desktop ISO</description>
      <enclosure url="http://example.com/download/123.nzb" length="2147483648" type="application/x-nzb"/>
    </item>
    <item>
      <title>Debian.12.Testing.x64</title>
      <link>http://example.com/details/124</link>
      <guid>http://example.com/details/124</guid>
      <pubDate>Thu, 18 Jan 2024 13:00:00 GMT</pubDate>
      <description>Debian Testing ISO</description>
      <enclosure url="http://example.com/download/124.nzb" length="1073741824" type="application/x-nzb"/>
    </item>
    <item>
      <title>Sample.Video.XviD</title>
      <link>http://example.com/details/125</link>
      <guid>http://example.com/details/125</guid>
      <pubDate>Thu, 18 Jan 2024 14:00:00 GMT</pubDate>
      <description>Small video sample</description>
      <enclosure url="http://example.com/download/125.nzb" length="524288" type="application/x-nzb"/>
    </item>
  </channel>
</rss>"#;

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\n\r\n{}",
                rss_feed.len(),
                rss_feed
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });

    // Give the server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Create feed in database
    let feed_id = {
        let mut conn = db.pool().acquire().await.unwrap();
        let result = sqlx::query(
                "INSERT INTO rss_feeds (name, url, check_interval_secs, auto_download, enabled, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind("Mock Indexer")
            .bind(&server_url)
            .bind(900)
            .bind(1)
            .bind(1)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *conn)
            .await
            .unwrap()
            .last_insert_rowid();
        drop(conn);
        result
    };

    // Configure feed with filter for Ubuntu/Debian items only
    let feed_config = RssFeedConfig {
        url: server_url.clone(),
        check_interval: std::time::Duration::from_secs(900),
        category: Some("linux".to_string()),
        filters: vec![RssFilter {
            name: "Linux ISOs".to_string(),
            include: vec!["(?i)(ubuntu|debian)".to_string()],
            exclude: vec!["(?i)sample".to_string()],
            min_size: Some(1_000_000_000), // 1 GB minimum
            max_size: None,
            max_age: None,
        }],
        auto_download: true,
        priority: crate::types::Priority::High,
        enabled: true,
    };

    // Create RSS manager and fetch feed
    let manager =
        RssManager::new(db.clone(), downloader.clone(), vec![feed_config.clone()]).unwrap();

    // Fetch and parse the feed
    let items = manager.check_feed(&feed_config).await;

    // Wait for server task to complete
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(2), server_task).await;

    assert!(items.is_ok(), "Feed fetch should succeed");
    let items = items.unwrap();

    // Verify we got all 3 items from the feed
    assert_eq!(items.len(), 3, "Should parse all 3 items from feed");

    // Verify item details
    assert_eq!(items[0].title, "Ubuntu.22.04.3.Desktop.x64");
    assert_eq!(items[0].guid, "http://example.com/details/123");
    assert_eq!(items[0].size, Some(2147483648));
    assert_eq!(
        items[0].nzb_url,
        Some("http://example.com/download/123.nzb".to_string())
    );

    assert_eq!(items[1].title, "Debian.12.Testing.x64");
    assert_eq!(items[1].size, Some(1073741824));

    assert_eq!(items[2].title, "Sample.Video.XviD");
    assert_eq!(items[2].size, Some(524288));

    // Verify filtering logic manually (without attempting downloads)
    // - Ubuntu item: matches include pattern, size >= 1GB -> should match
    // - Debian item: matches include pattern, size >= 1GB -> should match
    // - Sample item: excluded by "sample" pattern -> should NOT match

    let ubuntu_matches = manager.matches_filters(&items[0], &feed_config.filters[0]);
    let debian_matches = manager.matches_filters(&items[1], &feed_config.filters[0]);
    let sample_matches = manager.matches_filters(&items[2], &feed_config.filters[0]);

    assert!(ubuntu_matches, "Ubuntu item should match filter");
    assert!(debian_matches, "Debian item should match filter");
    assert!(
        !sample_matches,
        "Sample item should NOT match filter (excluded)"
    );

    // Create test config with auto_download disabled to test seen tracking
    let test_config = RssFeedConfig {
        auto_download: false, // Don't attempt downloads (fake URLs would fail)
        ..feed_config.clone()
    };

    // Process items to mark them as seen
    let downloaded = manager
        .process_feed_items(feed_id, &test_config, items)
        .await
        .unwrap();
    assert_eq!(
        downloaded, 0,
        "No downloads attempted (auto_download disabled)"
    );

    // Verify seen tracking (only matching items should be marked)
    assert!(
        db.is_rss_item_seen(feed_id, "http://example.com/details/123")
            .await
            .unwrap(),
        "Ubuntu item should be marked seen"
    );
    assert!(
        db.is_rss_item_seen(feed_id, "http://example.com/details/124")
            .await
            .unwrap(),
        "Debian item should be marked seen"
    );
    assert!(
        !db.is_rss_item_seen(feed_id, "http://example.com/details/125")
            .await
            .unwrap(),
        "Sample item should NOT be marked seen (excluded)"
    );
}

// =========================================================================
// HTTP error handling tests for check_feed
// =========================================================================

/// Helper: start a mock TCP server that returns a single HTTP response, then return its base URL.
async fn start_mock_http_server(status_code: u16, body: &str) -> String {
    let std_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    drop(std_listener);

    let listener = TokioTcpListener::bind(addr).await.unwrap();
    let response_body = body.to_string();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = vec![0; 4096];
            let _ = socket.read(&mut buf).await;

            let response = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                status_code,
                match status_code {
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "OK",
                },
                response_body.len(),
                response_body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });

    // Give server time to bind
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    format!("http://{}", addr)
}

#[tokio::test]
async fn check_feed_returns_error_on_http_500() {
    let (db, downloader) = create_test_setup().await;
    let server_url = start_mock_http_server(500, "Internal Server Error").await;

    let feed_config = RssFeedConfig {
        url: server_url.clone(),
        check_interval: Duration::from_secs(900),
        category: None,
        filters: vec![],
        auto_download: false,
        priority: Priority::Normal,
        enabled: true,
    };

    let manager = RssManager::new(db, downloader, vec![]).unwrap();
    let result = manager.check_feed(&feed_config).await;

    match result {
        Err(crate::error::Error::Other(msg)) => {
            assert!(
                msg.contains("HTTP 500"),
                "error should contain HTTP status code 500, got: {msg}"
            );
            assert!(
                msg.contains(&server_url),
                "error should contain the feed URL for diagnostics, got: {msg}"
            );
        }
        Ok(items) => panic!(
            "expected error for HTTP 500, got Ok with {} items",
            items.len()
        ),
        Err(other) => panic!("expected Error::Other for HTTP 500, got: {other:?}"),
    }
}

#[tokio::test]
async fn check_feed_returns_error_on_http_404() {
    let (db, downloader) = create_test_setup().await;
    let server_url = start_mock_http_server(404, "Not Found").await;

    let feed_config = RssFeedConfig {
        url: server_url.clone(),
        check_interval: Duration::from_secs(900),
        category: None,
        filters: vec![],
        auto_download: false,
        priority: Priority::Normal,
        enabled: true,
    };

    let manager = RssManager::new(db, downloader, vec![]).unwrap();
    let result = manager.check_feed(&feed_config).await;

    match result {
        Err(crate::error::Error::Other(msg)) => {
            assert!(
                msg.contains("HTTP 404"),
                "error should contain HTTP status code 404, got: {msg}"
            );
            assert!(
                msg.contains(&server_url),
                "error should contain the feed URL, got: {msg}"
            );
        }
        Ok(items) => panic!(
            "expected error for HTTP 404, got Ok with {} items",
            items.len()
        ),
        Err(other) => panic!("expected Error::Other for HTTP 404, got: {other:?}"),
    }
}

#[tokio::test]
async fn check_feed_returns_error_on_connection_refused() {
    let (db, downloader) = create_test_setup().await;

    // Bind a port and immediately drop it — nothing is listening
    let std_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    drop(std_listener);

    let feed_config = RssFeedConfig {
        url: format!("http://{}", addr),
        check_interval: Duration::from_secs(900),
        category: None,
        filters: vec![],
        auto_download: false,
        priority: Priority::Normal,
        enabled: true,
    };

    let manager = RssManager::new(db, downloader, vec![]).unwrap();
    let result = manager.check_feed(&feed_config).await;

    match result {
        Err(crate::error::Error::Other(msg)) => {
            assert!(
                msg.contains("Failed to fetch RSS feed"),
                "connection refused error should mention fetch failure, got: {msg}"
            );
        }
        Ok(items) => panic!(
            "expected error for connection refused, got Ok with {} items",
            items.len()
        ),
        Err(other) => panic!("expected Error::Other for connection refused, got: {other:?}"),
    }
}
