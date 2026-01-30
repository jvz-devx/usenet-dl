use super::*;

#[tokio::test]
async fn test_pause_queue_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing POST /queue/pause endpoint...");

    // Setup
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config);

    // Subscribe to events to verify QueuePaused event is emitted
    let mut event_rx = downloader.subscribe();

    // Add a test download to the queue
    let nzb_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file subject="test">
<groups><group>alt.binaries.test</group></groups>
<segments>
  <segment bytes="1000" number="1">message-id-1@example.com</segment>
</segments>
  </file>
</nzb>"#;

    let download_id = downloader
        .add_nzb_content(
            nzb_content.as_bytes(),
            "test.nzb",
            crate::types::DownloadOptions::default(),
        )
        .await
        .unwrap();

    println!("  📝 Created test download with ID: {}", download_id);

    // Test: Pause the queue
    println!("  🔍 Test: Pause all downloads in queue");
    let request = Request::builder()
        .method("POST")
        .uri("/queue/pause")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "pause_queue should return 204 NO_CONTENT"
    );

    println!("    ✓ Returns 204 NO_CONTENT for successful queue pause");

    // Wait for and verify QueuePaused event was emitted
    tokio::select! {
        event = event_rx.recv() => {
            match event {
                Ok(crate::Event::QueuePaused) => {
                    println!("    ✓ QueuePaused event was emitted");
                }
                Ok(other) => {
                    // Might receive Queued event first, try one more time
                    if let Ok(crate::Event::QueuePaused) = event_rx.recv().await {
                        println!("    ✓ QueuePaused event was emitted");
                    } else {
                        panic!("Expected QueuePaused event, got: {:?}", other);
                    }
                }
                Err(e) => panic!("Failed to receive event: {}", e),
            }
        }
        _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
            panic!("Timeout waiting for QueuePaused event");
        }
    }

    // Verify the download is paused
    let download_info = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        download_info.status,
        crate::types::Status::Paused.to_i32(),
        "Download should be paused"
    );
    println!("    ✓ Download status is set to Paused");

    println!("✅ pause_queue endpoint test passed!");
    println!("   - Returns 204 NO_CONTENT for successful pause");
    println!("   - Emits QueuePaused event");
    println!("   - Sets all downloads to Paused status");
}

#[tokio::test]
async fn test_resume_queue_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing POST /queue/resume endpoint...");

    // Setup
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config);

    // Subscribe to events to verify QueueResumed event is emitted
    let mut event_rx = downloader.subscribe();

    // Add a test download to the queue
    let nzb_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file subject="test">
<groups><group>alt.binaries.test</group></groups>
<segments>
  <segment bytes="1000" number="1">message-id-1@example.com</segment>
</segments>
  </file>
</nzb>"#;

    let download_id = downloader
        .add_nzb_content(
            nzb_content.as_bytes(),
            "test.nzb",
            crate::types::DownloadOptions::default(),
        )
        .await
        .unwrap();

    println!("  📝 Created test download with ID: {}", download_id);

    // First, pause the download so we can resume it
    downloader.pause(download_id).await.unwrap();
    println!("  📝 Paused download to set up for resume test");

    // Verify the download is paused
    let download_info = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        download_info.status,
        crate::types::Status::Paused.to_i32(),
        "Download should be paused before resume test"
    );

    // Test: Resume the queue
    println!("  🔍 Test: Resume all downloads in queue");
    let request = Request::builder()
        .method("POST")
        .uri("/queue/resume")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "resume_queue should return 204 NO_CONTENT"
    );

    println!("    ✓ Returns 204 NO_CONTENT for successful queue resume");

    // Wait for and verify QueueResumed event was emitted
    tokio::select! {
        event = event_rx.recv() => {
            match event {
                Ok(crate::Event::QueueResumed) => {
                    println!("    ✓ QueueResumed event was emitted");
                }
                Ok(other) => {
                    // Might receive other events first, try a few more times
                    let mut found_resume = false;
                    for _ in 0..3 {
                        if let Ok(crate::Event::QueueResumed) = event_rx.recv().await {
                            found_resume = true;
                            println!("    ✓ QueueResumed event was emitted");
                            break;
                        }
                    }
                    if !found_resume {
                        panic!("Expected QueueResumed event, got: {:?}", other);
                    }
                }
                Err(e) => panic!("Failed to receive event: {}", e),
            }
        }
        _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
            panic!("Timeout waiting for QueueResumed event");
        }
    }

    // Verify the download is queued (resumed from paused)
    let download_info = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        download_info.status,
        crate::types::Status::Queued.to_i32(),
        "Download should be queued (resumed) after resume_all"
    );
    println!("    ✓ Download status is set to Queued");

    println!("✅ resume_queue endpoint test passed!");
    println!("   - Returns 204 NO_CONTENT for successful resume");
    println!("   - Emits QueueResumed event");
    println!("   - Sets all downloads to Queued status");
}

#[tokio::test]
async fn test_queue_stats_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing GET /queue/stats endpoint...");

    // Setup
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config);

    // Test 1: Empty queue
    println!("  🔍 Test 1: Empty queue returns zeroed statistics");
    let request = Request::builder()
        .method("GET")
        .uri("/queue/stats")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "queue_stats should return 200 OK"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let stats: crate::types::QueueStats = serde_json::from_slice(&body).unwrap();

    assert_eq!(stats.total, 0, "Empty queue should have 0 total downloads");
    assert_eq!(
        stats.queued, 0,
        "Empty queue should have 0 queued downloads"
    );
    assert_eq!(
        stats.downloading, 0,
        "Empty queue should have 0 downloading"
    );
    assert_eq!(stats.paused, 0, "Empty queue should have 0 paused");
    assert_eq!(stats.processing, 0, "Empty queue should have 0 processing");
    assert_eq!(stats.total_speed_bps, 0, "Empty queue should have 0 speed");
    assert_eq!(
        stats.total_size_bytes, 0,
        "Empty queue should have 0 total size"
    );
    assert_eq!(
        stats.downloaded_bytes, 0,
        "Empty queue should have 0 downloaded bytes"
    );
    assert_eq!(
        stats.overall_progress, 0.0,
        "Empty queue should have 0% progress"
    );
    assert!(
        stats.accepting_new,
        "Should be accepting new downloads by default"
    );

    println!("    ✓ Empty queue returns all-zero statistics");

    // Test 2: Add downloads with different statuses
    println!("  🔍 Test 2: Queue with multiple downloads");

    // Add download 1 (will be queued)
    let nzb_content_1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file subject="test1">
<groups><group>alt.binaries.test</group></groups>
<segments>
  <segment bytes="1000" number="1">message-id-1@example.com</segment>
</segments>
  </file>
</nzb>"#;

    let _download_id_1 = downloader
        .add_nzb_content(
            nzb_content_1.as_bytes(),
            "test1.nzb",
            crate::types::DownloadOptions::default(),
        )
        .await
        .unwrap();

    // Add download 2 (will be queued)
    let nzb_content_2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file subject="test2">
<groups><group>alt.binaries.test</group></groups>
<segments>
  <segment bytes="2000" number="1">message-id-2@example.com</segment>
</segments>
  </file>
</nzb>"#;

    let download_id_2 = downloader
        .add_nzb_content(
            nzb_content_2.as_bytes(),
            "test2.nzb",
            crate::types::DownloadOptions::default(),
        )
        .await
        .unwrap();

    println!("  📝 Created 2 test downloads");

    // Pause download 2 to create a paused item
    downloader.pause(download_id_2).await.unwrap();
    println!("  📝 Paused download 2");

    // Query queue stats again
    let request = Request::builder()
        .method("GET")
        .uri("/queue/stats")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let stats: crate::types::QueueStats = serde_json::from_slice(&body).unwrap();

    println!(
        "  📊 Queue stats: total={}, queued={}, paused={}",
        stats.total, stats.queued, stats.paused
    );

    assert_eq!(stats.total, 2, "Should have 2 total downloads");
    assert_eq!(stats.queued, 1, "Should have 1 queued download");
    assert_eq!(stats.paused, 1, "Should have 1 paused download");
    assert_eq!(
        stats.downloading, 0,
        "Should have 0 downloading (no servers)"
    );
    assert_eq!(stats.processing, 0, "Should have 0 processing");
    assert_eq!(
        stats.total_size_bytes, 3000,
        "Should have 3000 bytes total (1000 + 2000)"
    );
    assert!(
        stats.accepting_new,
        "Should still be accepting new downloads"
    );

    println!("    ✓ Queue with multiple downloads shows correct counts");

    // Test 3: Verify speed limit is reflected in stats
    println!("  🔍 Test 3: Speed limit is reflected in stats");

    // Set a speed limit
    downloader.set_speed_limit(Some(1_000_000)).await;

    let request = Request::builder()
        .method("GET")
        .uri("/queue/stats")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let stats: crate::types::QueueStats = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        stats.speed_limit_bps,
        Some(1_000_000),
        "Speed limit should be reflected in stats"
    );
    println!("    ✓ Speed limit is correctly reflected in stats");

    println!("✅ queue_stats endpoint test passed!");
    println!("   - Returns 200 OK with valid JSON");
    println!("   - Empty queue returns all-zero statistics");
    println!("   - Queue with downloads shows correct counts by status");
    println!("   - Speed limit is reflected in response");
    println!("   - Total size and progress are calculated correctly");
}
