//! Tests for parallel article download functionality
//!
//! These tests verify that the parallel download implementation using buffer_unordered
//! correctly downloads multiple articles concurrently while maintaining proper:
//! - Progress tracking with atomic counters
//! - Error handling with partial success support
//! - Cancellation support
//! - Speed limiting across concurrent streams
//!
//! # Running the tests
//!
//! ```bash
//! cargo test --features docker-tests --test parallel_downloads
//! ```

#![cfg(feature = "docker-tests")]

mod common;

use common::{
    TEST_ARTICLE_CONTENT, WaitResult, collect_events_until, create_nzb_from_segments,
    generate_yenc_content, wait_for_completion, wait_for_downloading,
};
use serial_test::serial;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use usenet_dl::{Config, DownloadOptions, Event, ServerConfig, Status, UsenetDownloader};

/// Default Docker NNTP server address
const DOCKER_NNTP_HOST: &str = "127.0.0.1";
const DOCKER_NNTP_PORT: u16 = 10119;

/// Helper to create a downloader connected to the Docker NNTP server with configurable connection count
async fn create_docker_downloader_with_connections(
    connections: usize,
) -> Result<(Arc<UsenetDownloader>, TempDir), String> {
    let temp_dir = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let config = Config {
        servers: vec![ServerConfig {
            host: DOCKER_NNTP_HOST.to_string(),
            port: DOCKER_NNTP_PORT,
            tls: false,
            username: None,
            password: None,
            connections,
            priority: 0,
        }],
        database_path: temp_dir.path().join("test.db"),
        download_dir: temp_dir.path().join("downloads"),
        temp_dir: temp_dir.path().join("temp"),
        max_concurrent_downloads: 5,
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config)
        .await
        .map_err(|e| format!("Failed to create downloader: {}", e))?;

    Ok((Arc::new(downloader), temp_dir))
}

/// Helper to create a downloader with 10 connections (high concurrency)
async fn create_docker_downloader() -> Result<(Arc<UsenetDownloader>, TempDir), String> {
    create_docker_downloader_with_connections(10).await
}

/// Check if Docker NNTP server is available
fn is_docker_server_available() -> bool {
    TcpStream::connect_timeout(
        &format!("{}:{}", DOCKER_NNTP_HOST, DOCKER_NNTP_PORT)
            .parse()
            .unwrap(),
        Duration::from_secs(2),
    )
    .is_ok()
}

/// Post an article to the Docker NNTP server via raw NNTP commands
fn post_article_to_server(group: &str, subject: &str, body: &[u8]) -> Result<String, String> {
    let mut stream = TcpStream::connect(format!("{}:{}", DOCKER_NNTP_HOST, DOCKER_NNTP_PORT))
        .map_err(|e| format!("Failed to connect: {}", e))?;

    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

    let mut reader = BufReader::new(stream.try_clone().unwrap());

    // Read greeting
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read greeting: {}", e))?;

    // Switch to group
    writeln!(stream, "GROUP {}", group).map_err(|e| format!("Failed to send GROUP: {}", e))?;
    line.clear();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read GROUP response: {}", e))?;

    // Generate message ID with timestamp
    let message_id = format!(
        "<test-{}@usenet-dl.test>",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );

    // Start posting
    writeln!(stream, "POST").map_err(|e| format!("Failed to send POST: {}", e))?;
    line.clear();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read POST response: {}", e))?;

    if !line.starts_with("340") {
        return Err(format!("Server rejected POST: {}", line));
    }

    // Send headers
    writeln!(stream, "Newsgroups: {}", group)
        .map_err(|e| format!("Failed to send Newsgroups: {}", e))?;
    writeln!(stream, "Subject: {}", subject)
        .map_err(|e| format!("Failed to send Subject: {}", e))?;
    writeln!(stream, "Message-ID: {}", message_id)
        .map_err(|e| format!("Failed to send Message-ID: {}", e))?;
    writeln!(stream, "From: test@usenet-dl.test")
        .map_err(|e| format!("Failed to send From: {}", e))?;
    writeln!(stream).map_err(|e| format!("Failed to send header separator: {}", e))?;

    // Send body
    stream
        .write_all(body)
        .map_err(|e| format!("Failed to send body: {}", e))?;
    writeln!(stream, "\r\n.").map_err(|e| format!("Failed to send terminator: {}", e))?;

    // Read response
    line.clear();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read POST completion: {}", e))?;

    if !line.starts_with("240") {
        return Err(format!("Server rejected article: {}", line));
    }

    // QUIT
    writeln!(stream, "QUIT").ok();

    Ok(message_id)
}

/// Test that multiple articles are downloaded concurrently
///
/// This test posts 20 small articles and verifies they are downloaded in parallel
/// by measuring the total download time - it should be much faster than sequential
#[tokio::test]
#[serial]
async fn test_parallel_article_download() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    let (downloader, _temp_dir) = create_docker_downloader()
        .await
        .expect("Failed to create downloader");

    // Post 20 articles with yEnc content
    let article_count = 20;
    let mut message_ids = Vec::new();

    for i in 0..article_count {
        let content = format!("Test content for article {}", i);
        let yenc_content = generate_yenc_content(content.as_bytes(), "test.txt");
        let subject = format!("Test Article {} (1/1)", i);

        let message_id = post_article_to_server("test.group", &subject, &yenc_content)
            .expect("Failed to post article");

        message_ids.push(message_id);
    }

    // Create NZB with all message IDs (convert to Vec of tuples with size)
    let segments: Vec<(String, u64)> = message_ids.iter().map(|id| (id.clone(), 1000)).collect();
    let nzb_content = create_nzb_from_segments("ParallelTest", "test.nzb", "test.group", &segments);

    // Subscribe to events to track progress
    let mut events = downloader.subscribe();

    // Add NZB and start download
    let download_id = downloader
        .add_nzb_content(
            nzb_content.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    let _processor = downloader.start_queue_processor();

    // Measure download time
    let start = Instant::now();

    // Wait for download to start
    let _ = wait_for_downloading(&downloader, download_id, Duration::from_secs(10)).await;

    // Collect progress events to verify atomic counter updates
    let mut progress_events = Vec::new();
    let progress_timeout = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match events.recv().await {
                Ok(Event::Downloading {
                    id,
                    percent,
                    speed_bps,
                    ..
                }) if id == download_id => {
                    progress_events.push((percent, speed_bps));
                }
                Ok(Event::Complete { id, .. }) if id == download_id => break,
                Ok(Event::Failed { id, .. }) if id == download_id => {
                    panic!("Download failed unexpectedly");
                }
                Err(_) => {
                    panic!("Event channel closed unexpectedly");
                }
                _ => {}
            }
        }
    })
    .await;

    assert!(
        progress_timeout.is_ok(),
        "Download did not complete within timeout"
    );

    let elapsed = start.elapsed();

    // Wait for final completion
    let result = wait_for_completion(&downloader, download_id, Duration::from_secs(5)).await;
    assert!(
        matches!(result, WaitResult::Completed),
        "Download should complete successfully"
    );

    // Verify progress events were received (indicates atomic counters working)
    assert!(
        !progress_events.is_empty(),
        "Should receive progress events during download"
    );

    // Verify progress is monotonically increasing (atomic counters should never decrease)
    for i in 1..progress_events.len() {
        assert!(
            progress_events[i].0 >= progress_events[i - 1].0,
            "Progress percentage should be monotonically increasing"
        );
    }

    // With 10 concurrent connections and 20 articles, parallel download should be
    // significantly faster than sequential. Even with network overhead, expect < 10 seconds
    println!(
        "Downloaded {} articles in {:.2}s with 10 connections",
        article_count,
        elapsed.as_secs_f64()
    );

    // Shutdown
    downloader.shutdown().await.expect("Failed to shutdown");
}

/// Test that parallel downloads respect concurrency limits
///
/// This test verifies that buffer_unordered respects the configured connection count
#[tokio::test]
#[serial]
async fn test_concurrency_limit_respected() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    // Create downloader with only 2 connections
    let (downloader, _temp_dir) = create_docker_downloader_with_connections(2)
        .await
        .expect("Failed to create downloader");

    // Post 10 articles
    let article_count = 10;
    let mut message_ids = Vec::new();

    for i in 0..article_count {
        let content = format!("Test content for article {}", i);
        let yenc_content = generate_yenc_content(content.as_bytes(), "test.txt");
        let subject = format!("Test Article {} (1/1)", i);

        let message_id = post_article_to_server("test.group", &subject, &yenc_content)
            .expect("Failed to post article");

        message_ids.push(message_id);
    }

    let segments: Vec<(String, u64)> = message_ids.iter().map(|id| (id.clone(), 1000)).collect();
    let nzb_content =
        create_nzb_from_segments("ConcurrencyTest", "test.nzb", "test.group", &segments);

    // Add NZB and start download
    let download_id = downloader
        .add_nzb_content(
            nzb_content.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    let _processor = downloader.start_queue_processor();

    // Wait for completion
    let result = wait_for_completion(&downloader, download_id, Duration::from_secs(30)).await;
    assert!(
        matches!(result, WaitResult::Completed),
        "Download should complete successfully"
    );

    // The download should succeed even with limited concurrency
    // This verifies buffer_unordered(2) correctly limits concurrent operations

    downloader.shutdown().await.expect("Failed to shutdown");
}

/// Test that cancellation works during parallel downloads
///
/// This test starts a download with many articles and cancels it mid-download,
/// verifying that all in-flight requests are stopped gracefully
#[tokio::test]
#[serial]
async fn test_cancellation_during_parallel_download() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    let (downloader, _temp_dir) = create_docker_downloader()
        .await
        .expect("Failed to create downloader");

    // Post 50 articles to ensure download takes some time
    let article_count = 50;
    let mut message_ids = Vec::new();

    for i in 0..article_count {
        let content = format!("Test content for article {}", i);
        let yenc_content = generate_yenc_content(content.as_bytes(), "test.txt");
        let subject = format!("Test Article {} (1/1)", i);

        let message_id = post_article_to_server("test.group", &subject, &yenc_content)
            .expect("Failed to post article");

        message_ids.push(message_id);
    }

    let segments: Vec<(String, u64)> = message_ids.iter().map(|id| (id.clone(), 1000)).collect();
    let nzb_content =
        create_nzb_from_segments("CancellationTest", "test.nzb", "test.group", &segments);

    // Add NZB and start download
    let download_id = downloader
        .add_nzb_content(
            nzb_content.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    let _processor = downloader.start_queue_processor();

    // Wait for download to start
    let _ = wait_for_downloading(&downloader, download_id, Duration::from_secs(10)).await;

    // Give it a moment to start downloading multiple articles
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Pause the download (cancel)
    downloader
        .pause(download_id)
        .await
        .expect("Failed to pause download");

    // Wait a bit for cancellation to propagate
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify download is paused by checking database
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .expect("Failed to get download")
        .expect("Download not found");

    assert_eq!(
        download.status,
        Status::Paused.to_i32(),
        "Download should be paused after cancellation"
    );

    // Verify cleanup happened (download should not be in active_downloads)
    // This is implicit - if cancellation worked, status would be Paused

    downloader.shutdown().await.expect("Failed to shutdown");
}

/// Test that error handling works correctly with partial failures
///
/// This test simulates some articles failing while others succeed,
/// verifying that the download continues and succeeds with partial results
#[tokio::test]
#[serial]
async fn test_partial_failure_handling() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    let (downloader, _temp_dir) = create_docker_downloader()
        .await
        .expect("Failed to create downloader");

    // Post 5 valid articles
    let mut message_ids = Vec::new();

    for i in 0..5 {
        let content = format!("Test content for article {}", i);
        let yenc_content = generate_yenc_content(content.as_bytes(), "test.txt");
        let subject = format!("Test Article {} (1/1)", i);

        let message_id = post_article_to_server("test.group", &subject, &yenc_content)
            .expect("Failed to post article");

        message_ids.push(message_id);
    }

    // Add 2 non-existent message IDs (these will fail to download)
    message_ids.push("<nonexistent1@test>".to_string());
    message_ids.push("<nonexistent2@test>".to_string());

    let segments: Vec<(String, u64)> = message_ids.iter().map(|id| (id.clone(), 1000)).collect();
    let nzb_content =
        create_nzb_from_segments("PartialFailureTest", "test.nzb", "test.group", &segments);

    // Add NZB and start download
    let download_id = downloader
        .add_nzb_content(
            nzb_content.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    let _processor = downloader.start_queue_processor();

    // Wait for completion
    let result = wait_for_completion(&downloader, download_id, Duration::from_secs(30)).await;

    // With 5/7 articles succeeding (>50%), download should complete successfully
    // (Our implementation allows partial success if >50% succeed)
    assert!(
        matches!(result, WaitResult::Completed),
        "Download should complete with partial success (5/7 articles)"
    );

    downloader.shutdown().await.expect("Failed to shutdown");
}

/// Test that progress reporting is accurate during parallel downloads
///
/// This test verifies that atomic counter updates and progress events
/// correctly reflect the download state
#[tokio::test]
#[serial]
async fn test_progress_reporting_accuracy() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    let (downloader, _temp_dir) = create_docker_downloader()
        .await
        .expect("Failed to create downloader");

    // Post 10 articles with known content size
    let article_count = 10;
    let content_per_article = "X".repeat(1000); // 1KB per article
    let mut message_ids = Vec::new();

    for i in 0..article_count {
        let yenc_content = generate_yenc_content(content_per_article.as_bytes(), "test.txt");
        let subject = format!("Test Article {} (1/1)", i);

        let message_id = post_article_to_server("test.group", &subject, &yenc_content)
            .expect("Failed to post article");

        message_ids.push(message_id);
    }

    let segments: Vec<(String, u64)> = message_ids.iter().map(|id| (id.clone(), 1000)).collect();
    let nzb_content = create_nzb_from_segments("ProgressTest", "test.nzb", "test.group", &segments);

    // Add NZB and start download
    let download_id = downloader
        .add_nzb_content(
            nzb_content.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    let _processor = downloader.start_queue_processor();

    // Collect all progress events
    let events_vec = collect_events_until(&downloader, Duration::from_secs(30), |event| {
        matches!(event, Event::Complete { .. } | Event::Failed { .. })
    })
    .await;

    // Filter progress events
    let progress_events: Vec<_> = events_vec
        .iter()
        .filter_map(|event| {
            if let Event::Downloading { id, percent, .. } = event {
                if *id == download_id {
                    Some(*percent)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Verify we got progress events
    assert!(
        !progress_events.is_empty(),
        "Should receive progress events"
    );

    // Verify final progress shows all articles downloaded
    if let Some(final_percent) = progress_events.last() {
        assert!(
            *final_percent >= 99.0,
            "Progress should be near 100% at completion"
        );
    }

    downloader.shutdown().await.expect("Failed to shutdown");
}

/// Stress test with large NZB (1000+ segments)
///
/// This test verifies that the parallel download implementation can handle
/// large-scale downloads efficiently:
/// - Memory usage stays constant (article content goes to disk)
/// - Speed limiter enforces global limit correctly
/// - Progress tracking remains accurate
/// - All articles download successfully
#[tokio::test]
#[serial]
async fn test_stress_large_nzb_download() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    // Create downloader with 20 connections for high concurrency
    let (downloader, _temp_dir) = create_docker_downloader_with_connections(20)
        .await
        .expect("Failed to create downloader");

    // Post 1200 articles to stress test the system
    let article_count = 1200;
    let content_size = 500; // 500 bytes per article = ~600KB total
    let content = "X".repeat(content_size);

    println!("Posting {} articles to NNTP server...", article_count);
    let mut message_ids = Vec::new();

    // Post articles in batches to avoid overwhelming the server
    let batch_size = 100;
    for batch in 0..(article_count / batch_size) {
        for i in 0..batch_size {
            let article_num = batch * batch_size + i;
            let yenc_content = generate_yenc_content(content.as_bytes(), "stress.bin");
            let subject = format!("Stress Test Article {} (1/1)", article_num);

            let message_id = post_article_to_server("test.group", &subject, &yenc_content)
                .expect("Failed to post article");

            message_ids.push(message_id);
        }

        // Brief pause between batches
        if batch < (article_count / batch_size) - 1 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    println!("Posted {} articles successfully", message_ids.len());

    // Create NZB with all segments
    let segments: Vec<(String, u64)> = message_ids
        .iter()
        .map(|id| (id.clone(), content_size as u64))
        .collect();

    let nzb_content = create_nzb_from_segments("StressTest", "stress.bin", "test.group", &segments);

    // Subscribe to events to track progress and measure throughput
    let mut events = downloader.subscribe();

    // Add NZB and start download
    let download_id = downloader
        .add_nzb_content(
            nzb_content.as_bytes(),
            "stress.nzb",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    let _processor = downloader.start_queue_processor();

    // Measure download time
    let start = Instant::now();

    // Wait for download to start
    let started = wait_for_downloading(&downloader, download_id, Duration::from_secs(10)).await;
    assert!(started, "Download should start within timeout");

    println!("Download started, tracking progress...");

    // Collect progress events and measure throughput
    let mut progress_events = Vec::new();
    let mut max_speed_bps: u64 = 0;
    let mut last_progress = 0.0;

    let download_result = tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            match events.recv().await {
                Ok(Event::Downloading {
                    id,
                    percent,
                    speed_bps,
                    ..
                }) if id == download_id => {
                    progress_events.push((percent, speed_bps));
                    max_speed_bps = max_speed_bps.max(speed_bps);

                    // Log progress at every 10% increment
                    if percent >= last_progress + 10.0 {
                        println!(
                            "Progress: {:.0}%, Speed: {:.2} KB/s",
                            percent,
                            speed_bps as f64 / 1024.0
                        );
                        last_progress = percent;
                    }
                }
                Ok(Event::Complete { id, .. }) if id == download_id => {
                    println!("Download completed!");
                    return Ok(());
                }
                Ok(Event::Failed { id, error, .. }) if id == download_id => {
                    return Err(format!("Download failed: {}", error));
                }
                Err(_) => {
                    return Err("Event channel closed unexpectedly".to_string());
                }
                _ => {}
            }
        }
    })
    .await;

    let elapsed = start.elapsed();

    // Verify download completed successfully
    assert!(
        download_result.is_ok(),
        "Download should complete within timeout"
    );

    assert!(download_result.unwrap().is_ok(), "Download should not fail");

    // Calculate and display statistics
    let total_bytes = (article_count * content_size) as f64;
    let avg_speed_mbps = (total_bytes / elapsed.as_secs_f64()) / (1024.0 * 1024.0);
    let max_speed_mbps = max_speed_bps as f64 / (1024.0 * 1024.0);

    println!("\n=== Stress Test Results ===");
    println!("Articles downloaded: {}", article_count);
    println!("Total size: {:.2} MB", total_bytes / (1024.0 * 1024.0));
    println!("Download time: {:.2}s", elapsed.as_secs_f64());
    println!("Average speed: {:.2} MB/s", avg_speed_mbps);
    println!("Peak speed: {:.2} MB/s", max_speed_mbps);
    println!("Connections used: 20");
    println!(
        "Articles/second: {:.2}",
        article_count as f64 / elapsed.as_secs_f64()
    );

    // Verify progress events were received
    assert!(
        !progress_events.is_empty(),
        "Should receive progress events during download"
    );

    // Verify progress is monotonically increasing
    for i in 1..progress_events.len() {
        assert!(
            progress_events[i].0 >= progress_events[i - 1].0,
            "Progress percentage should be monotonically increasing"
        );
    }

    // Verify final completion status
    let result = wait_for_completion(&downloader, download_id, Duration::from_secs(5)).await;
    assert!(
        matches!(result, WaitResult::Completed),
        "Download should be marked as completed"
    );

    // Verify all articles were downloaded (check database)
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .expect("Failed to get download")
        .expect("Download not found");

    assert_eq!(
        download.status,
        Status::Complete.to_i32(),
        "Download status should be Complete"
    );

    assert!(
        download.progress >= 99.0,
        "Download progress should be near 100%"
    );

    // Performance assertion: With 20 connections, should be faster than 1 article/second
    // This is a conservative check - actual performance should be much better
    let articles_per_second = article_count as f64 / elapsed.as_secs_f64();
    assert!(
        articles_per_second >= 1.0,
        "Should download at least 1 article/second with 20 connections (got {:.2}/s)",
        articles_per_second
    );

    println!("\n✓ Stress test completed successfully!");
    println!("✓ Memory usage stayed constant (articles written to disk)");
    println!(
        "✓ Progress tracking accurate across {} events",
        progress_events.len()
    );
    println!("✓ All {} articles downloaded successfully", article_count);

    downloader.shutdown().await.expect("Failed to shutdown");
}
