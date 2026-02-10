use super::*;

#[tokio::test]
async fn test_start_scheduler_with_rules() {
    let temp_dir = tempdir().unwrap();

    // Create config with schedule rules
    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: temp_dir.path().join("test.db"),
            schedule_rules: vec![config::ScheduleRule {
                name: "Test Rule".to_string(),
                days: vec![], // All days
                start_time: "09:00".to_string(),
                end_time: "17:00".to_string(),
                action: config::ScheduleAction::SpeedLimit {
                    limit_bps: 1_000_000,
                },
                enabled: true,
            }],
            categories: std::collections::HashMap::new(),
        },
        servers: vec![],
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start scheduler
    let handle = downloader.start_scheduler();

    // Let the scheduler task start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the task is still running (it shouldn't complete immediately)
    assert!(
        !handle.is_finished(),
        "Scheduler should be running with configured rules"
    );

    // Abort the task
    handle.abort();
}

#[tokio::test]
async fn test_start_scheduler_respects_shutdown() {
    let temp_dir = tempdir().unwrap();

    // Create config with schedule rules
    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: temp_dir.path().join("test.db"),
            schedule_rules: vec![config::ScheduleRule {
                name: "Test Rule".to_string(),
                days: vec![],
                start_time: "09:00".to_string(),
                end_time: "17:00".to_string(),
                action: config::ScheduleAction::Unlimited,
                enabled: true,
            }],
            categories: std::collections::HashMap::new(),
        },
        servers: vec![],
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Trigger shutdown before starting the task
    downloader
        .queue_state
        .accepting_new
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Start scheduler
    let handle = downloader.start_scheduler();

    // Task should exit gracefully immediately without waiting the full minute
    let result = tokio::time::timeout(Duration::from_secs(1), handle).await;
    assert!(result.is_ok(), "Scheduler should exit on shutdown signal");
}
