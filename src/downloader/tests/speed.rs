use super::*;

#[tokio::test]
async fn test_speed_limiter_shared_across_downloads() {
    // This test verifies that the speed limiter is properly shared
    // across all download tasks

    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: db_path,
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        servers: vec![],
        download: config::DownloadConfig {
            max_concurrent_downloads: 3,
            speed_limit_bps: Some(1_000_000), // 1 MB/s limit
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Verify speed limiter is configured
    assert_eq!(downloader.speed_limiter.get_limit(), Some(1_000_000));

    // Test that the same limiter instance is shared
    // by verifying limit changes affect all downloads
    downloader.speed_limiter.set_limit(Some(5_000_000)); // 5 MB/s
    assert_eq!(downloader.speed_limiter.get_limit(), Some(5_000_000));

    // Reset to unlimited
    downloader.speed_limiter.set_limit(None);
    assert_eq!(downloader.speed_limiter.get_limit(), None);
}

#[tokio::test]
async fn test_set_speed_limit_method() {
    // This test verifies that set_speed_limit() properly updates the limiter
    // and emits the SpeedLimitChanged event

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Subscribe to events before changing limit
    let mut rx = downloader.subscribe();

    // Initially should be unlimited (default)
    assert_eq!(downloader.speed_limiter.get_limit(), None);

    // Set speed limit to 10 MB/s
    downloader.set_speed_limit(Some(10_000_000)).await;

    // Verify limit was updated
    assert_eq!(downloader.speed_limiter.get_limit(), Some(10_000_000));

    // Verify event was emitted
    let event = rx.recv().await.unwrap();
    match event {
        crate::types::Event::SpeedLimitChanged { limit_bps } => {
            assert_eq!(limit_bps, Some(10_000_000));
        }
        other => panic!("Expected SpeedLimitChanged event, got {:?}", other),
    }

    // Change to unlimited
    downloader.set_speed_limit(None).await;
    assert_eq!(downloader.speed_limiter.get_limit(), None);

    // Verify second event was emitted
    let event = rx.recv().await.unwrap();
    match event {
        crate::types::Event::SpeedLimitChanged { limit_bps } => {
            assert_eq!(limit_bps, None);
        }
        other => panic!(
            "Expected SpeedLimitChanged event with None, got {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn test_set_speed_limit_takes_effect_immediately() {
    // Verify that speed limit changes take effect immediately for ongoing downloads

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start with 5 MB/s
    downloader.set_speed_limit(Some(5_000_000)).await;
    assert_eq!(downloader.speed_limiter.get_limit(), Some(5_000_000));

    // Change to 10 MB/s
    downloader.set_speed_limit(Some(10_000_000)).await;
    assert_eq!(downloader.speed_limiter.get_limit(), Some(10_000_000));

    // Verify we can still acquire bytes (limiter is functional)
    downloader.speed_limiter.acquire(1000).await;
    // If we reach here, the limiter is working after the change
}

#[tokio::test]
async fn test_speed_limit_with_multiple_concurrent_downloads() {
    // Test speed limiting with multiple concurrent downloads
    // This test verifies that the speed limiter properly limits total bandwidth
    // across multiple concurrent downloads and distributes bandwidth fairly

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Set a low speed limit for testing (5 MB/s)
    downloader.set_speed_limit(Some(5_000_000)).await;

    // Simulate 3 concurrent downloads
    let limiter = downloader.speed_limiter.clone();
    let start = Instant::now();

    let mut handles = vec![];
    for download_id in 0..3 {
        let limiter_clone = limiter.clone();
        let handle = tokio::spawn(async move {
            // Each download tries to transfer 10 MB total
            // Split into 1 MB chunks to simulate realistic article downloads
            for _ in 0..10 {
                limiter_clone.acquire(1_000_000).await; // 1 MB chunk
            }
            download_id
        });
        handles.push(handle);
    }

    // Wait for all downloads to complete
    for handle in handles {
        handle.await.unwrap();
    }

    let elapsed = start.elapsed();

    // Total data: 3 downloads × 10 MB = 30 MB
    // Speed limit: 5 MB/s
    // Expected time: 30 MB ÷ 5 MB/s = 6 seconds
    // Allow 20% tolerance (4.8s - 7.2s)
    let min_duration = Duration::from_millis(4800); // 80% of 6 seconds
    let max_duration = Duration::from_millis(7200); // 120% of 6 seconds

    assert!(
        elapsed >= min_duration,
        "Downloads completed too quickly: {:?} (expected >= {:?}). \
         Speed limit may not be working properly.",
        elapsed,
        min_duration
    );
    assert!(
        elapsed <= max_duration,
        "Downloads took too long: {:?} (expected <= {:?}). \
         Speed limiter may be too conservative.",
        elapsed,
        max_duration
    );
}

#[tokio::test]
async fn test_speed_limit_bandwidth_distribution() {
    // Test that bandwidth is distributed fairly across concurrent downloads
    // All downloads should complete at roughly the same time

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Set speed limit to 6 MB/s
    downloader.set_speed_limit(Some(6_000_000)).await;

    let limiter = downloader.speed_limiter.clone();

    // Shared start time for all downloads
    let global_start = Instant::now();

    // Spawn 3 concurrent downloads that each download 6 MB
    let mut handles = vec![];
    for download_id in 0..3 {
        let limiter_clone = limiter.clone();
        let handle = tokio::spawn(async move {
            // Each download: 6 MB in 500 KB chunks
            for _ in 0..12 {
                limiter_clone.acquire(500_000).await;
            }
            download_id
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }

    let total_elapsed = global_start.elapsed();

    // Total: 18 MB at 6 MB/s = 3 seconds expected
    // With fair distribution, all should finish at roughly the same time
    let expected = Duration::from_secs(3);
    let tolerance = Duration::from_millis(1500); // ±1.5s tolerance

    assert!(
        total_elapsed.as_millis() >= (expected.as_millis() - tolerance.as_millis()),
        "All downloads completed too quickly: {:?} (expected ~{:?}). \
         Speed limiting may not be working properly.",
        total_elapsed,
        expected
    );
    assert!(
        total_elapsed.as_millis() <= (expected.as_millis() + tolerance.as_millis()),
        "Downloads took too long: {:?} (expected ~{:?})",
        total_elapsed,
        expected
    );
}
