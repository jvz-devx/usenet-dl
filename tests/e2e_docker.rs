//! End-to-end tests with local Docker NNTP server
//!
//! These tests use a local INN server running in Docker for deterministic testing.
//! Tests are feature-gated behind `docker-tests`.
//!
//! # Prerequisites
//!
//! Start the Docker NNTP server:
//! ```bash
//! docker-compose -f docker/docker-compose.test.yml up -d
//! ```
//!
//! # Running the tests
//!
//! ```bash
//! cargo test --features docker-tests --test e2e_docker
//! ```
//!
//! # Stopping the server
//!
//! ```bash
//! docker-compose -f docker/docker-compose.test.yml down
//! ```

#![cfg(feature = "docker-tests")]

mod common;

use common::{
    TEST_ARTICLE_CONTENT, WaitResult, create_nzb_from_segments, generate_yenc_content,
    wait_for_completion,
};
use serial_test::serial;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use usenet_dl::{Config, DownloadOptions, ServerConfig, Status, UsenetDownloader};

/// Default Docker NNTP server address
const DOCKER_NNTP_HOST: &str = "127.0.0.1";
const DOCKER_NNTP_PORT: u16 = 10119;

/// Helper to create a downloader connected to the Docker NNTP server
async fn create_docker_downloader() -> Result<(Arc<UsenetDownloader>, TempDir), String> {
    let temp_dir = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let config = Config {
        servers: vec![ServerConfig {
            host: DOCKER_NNTP_HOST.to_string(),
            port: DOCKER_NNTP_PORT,
            tls: false, // Local Docker server doesn't use TLS
            username: None,
            password: None,
            connections: 2,
            priority: 0,
        }],
        database_path: temp_dir.path().join("test.db"),
        download_dir: temp_dir.path().join("downloads"),
        temp_dir: temp_dir.path().join("temp"),
        max_concurrent_downloads: 2,
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config)
        .await
        .map_err(|e| format!("Failed to create downloader: {}", e))?;

    Ok((Arc::new(downloader), temp_dir))
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
///
/// Returns the message ID of the posted article
fn post_article_to_server(group: &str, subject: &str, body: &[u8]) -> Result<String, String> {
    let mut stream = TcpStream::connect(format!("{}:{}", DOCKER_NNTP_HOST, DOCKER_NNTP_PORT))
        .map_err(|e| format!("Failed to connect: {}", e))?;

    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

    let mut reader = BufReader::new(stream.try_clone().unwrap());

    // Read greeting
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|e| format!("Read error: {}", e))?;
    if !response.starts_with("200") && !response.starts_with("201") {
        return Err(format!("Unexpected greeting: {}", response));
    }

    // Generate unique message ID
    let message_id = format!(
        "<usenet-dl-test-{}-{}@test.local>",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        rand::random::<u32>()
    );

    // Send POST command
    stream
        .write_all(b"POST\r\n")
        .map_err(|e| format!("Write error: {}", e))?;
    response.clear();
    reader
        .read_line(&mut response)
        .map_err(|e| format!("Read error: {}", e))?;
    if !response.starts_with("340") {
        return Err(format!("POST not allowed: {}", response));
    }

    // Send article headers and body
    let article = format!(
        "From: test@usenet-dl.local\r\n\
         Newsgroups: {}\r\n\
         Subject: {}\r\n\
         Message-ID: {}\r\n\
         Date: {}\r\n\
         \r\n",
        group,
        subject,
        message_id,
        chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S +0000")
    );

    stream
        .write_all(article.as_bytes())
        .map_err(|e| format!("Write error: {}", e))?;
    stream
        .write_all(body)
        .map_err(|e| format!("Write error: {}", e))?;

    // End article with lone dot
    if !body.ends_with(b"\r\n") {
        stream
            .write_all(b"\r\n")
            .map_err(|e| format!("Write error: {}", e))?;
    }
    stream
        .write_all(b".\r\n")
        .map_err(|e| format!("Write error: {}", e))?;

    // Read response
    response.clear();
    reader
        .read_line(&mut response)
        .map_err(|e| format!("Read error: {}", e))?;
    if !response.starts_with("240") {
        return Err(format!("Article not accepted: {}", response));
    }

    // QUIT
    stream.write_all(b"QUIT\r\n").ok();

    // Return message ID without angle brackets for NZB
    Ok(message_id[1..message_id.len() - 1].to_string())
}

// ============================================================================
// Connection Tests
// ============================================================================

/// Test connection to Docker NNTP server
#[tokio::test]
#[serial]
async fn test_docker_server_connection() {
    if !is_docker_server_available() {
        eprintln!(
            "Skipping: Docker NNTP server not available at {}:{}",
            DOCKER_NNTP_HOST, DOCKER_NNTP_PORT
        );
        eprintln!("Start it with: docker-compose -f docker/docker-compose.test.yml up -d");
        return;
    }

    let result = create_docker_downloader().await;
    assert!(
        result.is_ok(),
        "Should connect to Docker NNTP server: {:?}",
        result.err()
    );

    let (downloader, _temp_dir) = result.unwrap();
    println!("Successfully connected to Docker NNTP server");

    downloader.shutdown().await.ok();
}

// ============================================================================
// Post and Download Tests
// ============================================================================

/// Test posting an article and downloading it back
#[tokio::test]
#[serial]
async fn test_post_and_download() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    // Post a test article
    let message_id =
        post_article_to_server("alt.test", "usenet-dl test article", TEST_ARTICLE_CONTENT)
            .expect("Failed to post article");

    println!("Posted article with message ID: {}", message_id);

    // Create NZB pointing to our article
    let nzb = create_nzb_from_segments(
        "Post and Download Test",
        "test.txt",
        "alt.test",
        &[(message_id.clone(), TEST_ARTICLE_CONTENT.len() as u64)],
    );

    // Create downloader and download
    let (downloader, temp_dir) = create_docker_downloader()
        .await
        .expect("Failed to create downloader");

    let id = downloader
        .add_nzb_content(
            nzb.as_bytes(),
            "post_download_test",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    println!("Added download with ID: {}", id);

    // Start downloads
    let _processor = downloader.start_queue_processor();

    // Wait for completion
    let result = wait_for_completion(&downloader, id, Duration::from_secs(30)).await;

    match result {
        WaitResult::Completed => {
            println!("Download completed successfully!");

            // Verify file exists
            let download_dir = temp_dir.path().join("downloads").join("post_download_test");
            assert!(download_dir.exists(), "Download directory should exist");
        }
        WaitResult::Failed(error) => {
            panic!("Download failed: {}", error);
        }
        other => {
            panic!("Unexpected result: {:?}", other);
        }
    }

    downloader.shutdown().await.ok();
}

/// Test downloading yEnc-encoded content
#[tokio::test]
#[serial]
async fn test_yenc_encoding() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    // Create yEnc-encoded content
    let original_data = b"This is binary test data for yEnc encoding test.\x00\x01\x02\xFF";
    let yenc_content = generate_yenc_content(original_data, "test.bin");

    // Post yEnc article
    let message_id = post_article_to_server(
        "alt.binaries.test",
        "yEnc test (1/1) test.bin",
        &yenc_content,
    )
    .expect("Failed to post yEnc article");

    println!("Posted yEnc article with message ID: {}", message_id);

    // Create NZB and download
    let nzb = create_nzb_from_segments(
        "yEnc Test",
        "test.bin",
        "alt.binaries.test",
        &[(message_id, yenc_content.len() as u64)],
    );

    let (downloader, _temp_dir) = create_docker_downloader()
        .await
        .expect("Failed to create downloader");

    let id = downloader
        .add_nzb_content(nzb.as_bytes(), "yenc_test", DownloadOptions::default())
        .await
        .expect("Failed to add NZB");

    let _processor = downloader.start_queue_processor();

    let result = wait_for_completion(&downloader, id, Duration::from_secs(30)).await;
    println!("yEnc download result: {:?}", result);

    downloader.shutdown().await.ok();
}

/// Test downloading multi-segment content
#[tokio::test]
#[serial]
async fn test_multi_segment_assembly() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    // Post multiple segments
    let segment1 = b"Segment 1 content - first part of the file\r\n";
    let segment2 = b"Segment 2 content - middle part of the file\r\n";
    let segment3 = b"Segment 3 content - final part of the file\r\n";

    let msg_id1 = post_article_to_server("usenet-dl.test", "multi test (1/3)", segment1)
        .expect("Failed to post segment 1");
    let msg_id2 = post_article_to_server("usenet-dl.test", "multi test (2/3)", segment2)
        .expect("Failed to post segment 2");
    let msg_id3 = post_article_to_server("usenet-dl.test", "multi test (3/3)", segment3)
        .expect("Failed to post segment 3");

    println!("Posted 3 segments: {}, {}, {}", msg_id1, msg_id2, msg_id3);

    // Create NZB with all segments
    let nzb = create_nzb_from_segments(
        "Multi-Segment Assembly Test",
        "combined.txt",
        "usenet-dl.test",
        &[
            (msg_id1, segment1.len() as u64),
            (msg_id2, segment2.len() as u64),
            (msg_id3, segment3.len() as u64),
        ],
    );

    let (downloader, _temp_dir) = create_docker_downloader()
        .await
        .expect("Failed to create downloader");

    let id = downloader
        .add_nzb_content(
            nzb.as_bytes(),
            "multi_segment_test",
            DownloadOptions::default(),
        )
        .await
        .expect("Failed to add NZB");

    let _processor = downloader.start_queue_processor();

    let result = wait_for_completion(&downloader, id, Duration::from_secs(60)).await;
    println!("Multi-segment download result: {:?}", result);

    downloader.shutdown().await.ok();
}

// ============================================================================
// Error Injection Tests
// ============================================================================

/// Test handling of non-existent article (simulating article expiry)
#[tokio::test]
#[serial]
async fn test_article_expiry_simulation() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    // Create NZB pointing to non-existent article (simulates expired article)
    let fake_message_id = format!("expired-article-{}@test.local", rand::random::<u64>());

    let nzb = create_nzb_from_segments(
        "Expired Article Test",
        "expired.bin",
        "alt.test",
        &[(fake_message_id.clone(), 1000)],
    );

    let (downloader, _temp_dir) = create_docker_downloader()
        .await
        .expect("Failed to create downloader");

    let id = downloader
        .add_nzb_content(nzb.as_bytes(), "expired_test", DownloadOptions::default())
        .await
        .expect("Failed to add NZB");

    let _processor = downloader.start_queue_processor();

    // Should fail with article not found
    let result = wait_for_completion(&downloader, id, Duration::from_secs(30)).await;

    match result {
        WaitResult::Failed(error) => {
            println!("Got expected error for missing article: {}", error);
        }
        WaitResult::Completed => {
            panic!("Expected failure for non-existent article");
        }
        other => {
            println!("Got result: {:?}", other);
        }
    }

    downloader.shutdown().await.ok();
}

/// Test partial download (some segments missing)
#[tokio::test]
#[serial]
async fn test_partial_download_missing_segments() {
    if !is_docker_server_available() {
        eprintln!("Skipping: Docker NNTP server not available");
        return;
    }

    // Post only first segment
    let segment1 = b"First segment only\r\n";
    let msg_id1 = post_article_to_server("usenet-dl.test", "partial test (1/3)", segment1)
        .expect("Failed to post segment 1");

    // Create NZB with 3 segments, but only 1 exists
    let fake_msg_id2 = format!("missing-segment-2-{}@test.local", rand::random::<u64>());
    let fake_msg_id3 = format!("missing-segment-3-{}@test.local", rand::random::<u64>());

    let nzb = create_nzb_from_segments(
        "Partial Download Test",
        "partial.txt",
        "usenet-dl.test",
        &[
            (msg_id1, segment1.len() as u64),
            (fake_msg_id2, 100),
            (fake_msg_id3, 100),
        ],
    );

    let (downloader, _temp_dir) = create_docker_downloader()
        .await
        .expect("Failed to create downloader");

    let id = downloader
        .add_nzb_content(nzb.as_bytes(), "partial_test", DownloadOptions::default())
        .await
        .expect("Failed to add NZB");

    let _processor = downloader.start_queue_processor();

    // Should fail due to missing segments
    let result = wait_for_completion(&downloader, id, Duration::from_secs(60)).await;

    match result {
        WaitResult::Failed(error) => {
            println!("Got expected error for partial download: {}", error);
        }
        other => {
            println!("Partial download result: {:?}", other);
        }
    }

    downloader.shutdown().await.ok();
}
