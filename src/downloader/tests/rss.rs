use super::*;

#[tokio::test]
async fn test_start_rss_scheduler_with_feeds() {
    let temp_dir = tempdir().unwrap();

    // Create config with RSS feeds
    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: temp_dir.path().join("test.db"),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        servers: vec![],
        automation: config::AutomationConfig {
            rss_feeds: vec![config::RssFeedConfig {
                url: "https://example.com/feed.xml".to_string(),
                check_interval: Duration::from_secs(60), // 1 minute
                category: Some("test".to_string()),
                filters: vec![],
                auto_download: true,
                priority: Priority::Normal,
                enabled: true,
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start RSS scheduler
    let handle = downloader.start_rss_scheduler();

    // Let the scheduler task start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the task is still running (it shouldn't complete immediately)
    assert!(
        !handle.is_finished(),
        "Scheduler should be running with configured feeds"
    );

    // Abort the task
    handle.abort();
}

#[tokio::test]
async fn test_start_rss_scheduler_respects_shutdown() {
    let temp_dir = tempdir().unwrap();

    // Create config with RSS feeds
    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: temp_dir.path().join("test.db"),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        servers: vec![],
        automation: config::AutomationConfig {
            rss_feeds: vec![config::RssFeedConfig {
                url: "https://example.com/feed.xml".to_string(),
                check_interval: Duration::from_secs(60),
                category: None,
                filters: vec![],
                auto_download: false,
                priority: Priority::Normal,
                enabled: true,
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start RSS scheduler
    let handle = downloader.start_rss_scheduler();

    // Let it run briefly
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Trigger shutdown
    downloader
        .queue_state
        .accepting_new
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Wait for scheduler to detect shutdown
    // Note: Scheduler checks every second, so 5 seconds should be plenty
    let result = tokio::time::timeout(Duration::from_secs(5), handle).await;

    assert!(
        result.is_ok(),
        "Scheduler should shut down gracefully when accepting_new is set to false"
    );
}

#[tokio::test]
async fn test_start_rss_scheduler_with_multiple_feeds() {
    let temp_dir = tempdir().unwrap();

    // Create config with multiple RSS feeds
    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: temp_dir.path().join("test.db"),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        servers: vec![],
        automation: config::AutomationConfig {
            rss_feeds: vec![
                config::RssFeedConfig {
                    url: "https://example.com/feed1.xml".to_string(),
                    check_interval: Duration::from_secs(30),
                    category: Some("movies".to_string()),
                    filters: vec![],
                    auto_download: true,
                    priority: Priority::High,
                    enabled: true,
                },
                config::RssFeedConfig {
                    url: "https://example.com/feed2.xml".to_string(),
                    check_interval: Duration::from_secs(60),
                    category: Some("tv".to_string()),
                    filters: vec![],
                    auto_download: false,
                    priority: Priority::Normal,
                    enabled: false, // Disabled feed should be skipped
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start RSS scheduler
    let handle = downloader.start_rss_scheduler();

    // Let it run briefly
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the task is running
    assert!(
        !handle.is_finished(),
        "Scheduler should handle multiple feeds"
    );

    // Abort the task
    handle.abort();
}
