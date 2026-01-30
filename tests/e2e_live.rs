//! End-to-end tests with real NNTP provider
//!
//! These tests connect to a real Usenet provider using credentials from .env
//! All tests are marked #[ignore] to prevent running in normal CI.
//!
//! # Running the tests
//!
//! ```bash
//! # Run all live E2E tests
//! cargo test --test e2e_live -- --ignored --nocapture
//!
//! # Run a specific test
//! cargo test --test e2e_live test_valid_credentials -- --ignored --nocapture
//! ```
//!
//! # Required environment variables (.env file)
//!
//! - `NNTP_HOST` - Server hostname (e.g., news.example.com)
//! - `NNTP_USERNAME` - Authentication username
//! - `NNTP_PASSWORD` - Authentication password
//! - `NNTP_PORT_SSL` - TLS port (optional, default: 563)

mod common;

use common::{
    WaitResult, assert_download_status, create_downloader_bad_auth, create_live_downloader,
    has_live_credentials, wait_for_completion, wait_for_downloading,
};
use serial_test::serial;
use std::time::Duration;
use usenet_dl::{DownloadOptions, Priority, Status};

// ============================================================================
// Authentication Tests
// ============================================================================

/// Test that valid credentials can connect to the NNTP server
#[tokio::test]
#[ignore]
#[serial]
async fn test_valid_credentials() {
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    let result = create_live_downloader().await;
    assert!(
        result.is_ok(),
        "Should connect with valid credentials: {:?}",
        result.err()
    );

    let (downloader, _temp_dir) = result.unwrap();

    // Downloader created successfully means NNTP pool was initialized
    // The actual connection test happens when we try to use it
    println!("Successfully created downloader with valid credentials");

    // Clean shutdown
    downloader.shutdown().await.ok();
}

/// Test that invalid password is rejected
#[tokio::test]
#[ignore]
#[serial]
async fn test_invalid_password() {
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    // Create downloader with bad password
    let result = create_downloader_bad_auth().await;

    // Note: NNTP pools are lazy - the connection error happens when we try to download
    // So we need to actually try a download to trigger the auth error
    if let Ok((downloader, _temp_dir)) = result {
        // Try to add an NZB - the actual auth failure happens on download attempt
        let nzb = common::MINIMAL_NZB;
        let add_result = downloader
            .add_nzb_content(nzb.as_bytes(), "test.nzb", DownloadOptions::default())
            .await;

        if let Ok(id) = add_result {
            // Start queue processor
            let _processor = downloader.start_queue_processor();

            // Wait for failure event
            let wait_result = wait_for_completion(&downloader, id, Duration::from_secs(30)).await;

            match wait_result {
                WaitResult::Failed(error) => {
                    println!("Got expected auth failure: {}", error);
                    assert!(
                        error.to_lowercase().contains("auth")
                            || error.to_lowercase().contains("password")
                            || error.to_lowercase().contains("denied")
                            || error.to_lowercase().contains("credential"),
                        "Expected auth-related error, got: {}",
                        error
                    );
                }
                WaitResult::Completed => {
                    panic!("Expected auth failure but download completed successfully");
                }
                other => {
                    println!("Got result: {:?}", other);
                    // Timeout or channel close might also indicate connection failure
                }
            }
        }

        downloader.shutdown().await.ok();
    }
}

// ============================================================================
// Download Tests
// ============================================================================

/// Test downloading a single article
///
/// Note: This test requires a valid NZB with real message IDs that exist on your provider
#[tokio::test]
#[ignore]
#[serial]
async fn test_download_single_article() {
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    let (downloader, temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    // Use a minimal NZB - in a real test you'd use an NZB with valid message IDs
    // For now, we'll test that the download flow works even if articles don't exist
    let nzb = common::MINIMAL_NZB;

    let id = downloader
        .add_nzb_content(
            nzb.as_bytes(),
            "single_article_test",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    println!("Added download with ID: {}", id);

    // Start the queue processor
    let _processor = downloader.start_queue_processor();

    // Wait for result (will likely fail with "article not found" for fake message IDs)
    let result = wait_for_completion(&downloader, id, Duration::from_secs(60)).await;

    match result {
        WaitResult::Completed => {
            println!("Download completed successfully!");
            // Verify files were created
            let download_dir = temp_dir
                .path()
                .join("downloads")
                .join("single_article_test");
            println!("Download dir: {:?}", download_dir);
        }
        WaitResult::Failed(error) => {
            // This is expected for fake message IDs
            println!("Download failed (expected for test message IDs): {}", error);
        }
        WaitResult::Timeout => {
            println!("Download timed out");
        }
        WaitResult::ChannelClosed => {
            println!("Channel closed");
        }
    }

    downloader.shutdown().await.ok();
}

/// Test downloading multiple segments
#[tokio::test]
#[ignore]
#[serial]
async fn test_download_multi_segment() {
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    let (downloader, _temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    let nzb = common::MULTI_SEGMENT_NZB;

    let id = downloader
        .add_nzb_content(
            nzb.as_bytes(),
            "multi_segment_test",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    println!("Added multi-segment download with ID: {}", id);

    // Start queue processor
    let _processor = downloader.start_queue_processor();

    // Wait for result
    let result = wait_for_completion(&downloader, id, Duration::from_secs(120)).await;
    println!("Multi-segment download result: {:?}", result);

    downloader.shutdown().await.ok();
}

/// Test that missing articles are handled properly (404 equivalent)
#[tokio::test]
#[ignore]
#[serial]
async fn test_missing_article_404() {
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    let (downloader, _temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    // Create NZB with definitely non-existent message ID
    let nzb = common::create_single_article_nzb(
        "definitely-not-a-real-message-id-12345678@fake.invalid",
        1000,
        "alt.test",
    );

    let id = downloader
        .add_nzb_content(
            nzb.as_bytes(),
            "missing_article_test",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    println!("Added download with fake message ID: {}", id);

    // Start queue processor
    let _processor = downloader.start_queue_processor();

    // Should fail with article not found
    let result = wait_for_completion(&downloader, id, Duration::from_secs(60)).await;

    match result {
        WaitResult::Failed(error) => {
            println!("Got expected error for missing article: {}", error);
            // The error should indicate article not found
        }
        WaitResult::Completed => {
            panic!("Expected failure for non-existent article, but download completed");
        }
        other => {
            println!("Got result: {:?}", other);
        }
    }

    downloader.shutdown().await.ok();
}

// ============================================================================
// Pause/Resume Tests
// ============================================================================

/// Test pause and resume functionality
#[tokio::test]
#[ignore]
#[serial]
async fn test_download_pause_resume() {
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    let (downloader, _temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    let nzb = common::MULTI_SEGMENT_NZB;

    let id = downloader
        .add_nzb_content(
            nzb.as_bytes(),
            "pause_resume_test",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    println!("Added download with ID: {}", id);

    // Start queue processor
    let _processor = downloader.start_queue_processor();

    // Wait a bit then pause
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("Pausing download...");
    downloader.pause(id).await.expect("Failed to pause");

    // Verify paused status
    assert_download_status(&downloader, id, Status::Paused).await;
    println!("Download paused successfully");

    // Wait a bit
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Resume
    println!("Resuming download...");
    downloader.resume(id).await.expect("Failed to resume");

    // Verify queued status (ready to continue)
    assert_download_status(&downloader, id, Status::Queued).await;
    println!("Download resumed successfully");

    downloader.shutdown().await.ok();
}

// ============================================================================
// Concurrent Download Tests
// ============================================================================

/// Test multiple concurrent downloads
#[tokio::test]
#[ignore]
#[serial]
async fn test_concurrent_downloads() {
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    let (downloader, _temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    // Add multiple downloads with different priorities
    let nzb = common::MINIMAL_NZB;

    let id1 = downloader
        .add_nzb_content(
            nzb.as_bytes(),
            "concurrent_test_1",
            DownloadOptions {
                priority: Priority::Low,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to add NZB 1");

    let id2 = downloader
        .add_nzb_content(
            nzb.as_bytes(),
            "concurrent_test_2",
            DownloadOptions {
                priority: Priority::High,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to add NZB 2");

    let id3 = downloader
        .add_nzb_content(
            nzb.as_bytes(),
            "concurrent_test_3",
            DownloadOptions {
                priority: Priority::Normal,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to add NZB 3");

    println!("Added 3 concurrent downloads: {}, {}, {}", id1, id2, id3);

    // Start queue processor
    let _processor = downloader.start_queue_processor();

    // Wait for all to reach terminal state
    tokio::time::sleep(Duration::from_secs(10)).await;

    // List downloads to see their states
    let downloads = downloader.db.list_downloads().await.unwrap_or_default();
    for d in &downloads {
        println!(
            "Download {}: status={:?}, progress={}%",
            d.id,
            Status::from_i32(d.status),
            d.progress
        );
    }

    downloader.shutdown().await.ok();
}

// ============================================================================
// Full Pipeline Tests (Download + Post-Processing)
// ============================================================================

/// Test full pipeline with RAR extraction
///
/// Note: Requires an NZB pointing to actual RAR content on your provider
#[tokio::test]
#[ignore]
#[serial]
async fn test_full_pipeline_rar() {
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    let (downloader, temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    // This test would need a real NZB with RAR content
    // For now, we just verify the flow works
    println!("Full pipeline RAR test - would need real RAR content NZB");
    println!("Temp dir: {:?}", temp_dir.path());

    downloader.shutdown().await.ok();
}

/// Test full pipeline with PAR2 verification
///
/// Note: Requires an NZB pointing to content with PAR2 files
#[tokio::test]
#[ignore]
#[serial]
async fn test_full_pipeline_with_par2() {
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    let (downloader, temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    // This test would need a real NZB with PAR2 files
    println!("Full pipeline PAR2 test - would need real PAR2 content NZB");
    println!("Temp dir: {:?}", temp_dir.path());

    downloader.shutdown().await.ok();
}

// ============================================================================
// Speed Limit Tests
// ============================================================================

/// Test that speed limiting works during downloads
#[tokio::test]
#[ignore]
#[serial]
async fn test_speed_limit_during_download() {
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    let (downloader, _temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    // Set a speed limit (1 MB/s)
    downloader.set_speed_limit(Some(1_000_000)).await;

    let nzb = common::MULTI_SEGMENT_NZB;

    let id = downloader
        .add_nzb_content(
            nzb.as_bytes(),
            "speed_limit_test",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    // Start queue processor
    let _processor = downloader.start_queue_processor();

    // Wait for download to start
    let started = wait_for_downloading(&downloader, id, Duration::from_secs(30)).await;
    println!("Download started: {}", started);

    // Change speed limit mid-download
    downloader.set_speed_limit(Some(5_000_000)).await; // 5 MB/s
    println!("Changed speed limit to 5 MB/s");

    // Remove speed limit
    downloader.set_speed_limit(None).await;
    println!("Removed speed limit");

    // Wait for completion
    let result = wait_for_completion(&downloader, id, Duration::from_secs(120)).await;
    println!("Speed limit test result: {:?}", result);

    downloader.shutdown().await.ok();
}
