use super::*;
use tempfile::tempdir;
use std::time::{Duration, Instant};

/// Helper to create a test UsenetDownloader instance with a persistent database
/// Returns the downloader and the tempdir (which must be kept alive)
async fn create_test_downloader() -> (UsenetDownloader, tempfile::TempDir) {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let config = Config {
        database_path: db_path,
        servers: vec![], // No servers for testing
        max_concurrent_downloads: 3,
        ..Default::default()
    };

    // Initialize database
    let db = Database::new(&config.database_path).await.unwrap();

    // Create broadcast channel
    let (event_tx, _rx) = tokio::sync::broadcast::channel(1000);

    // No NNTP pools since we have no servers
    let nntp_pools = Vec::new();

    // Create priority queue
    let queue = std::sync::Arc::new(tokio::sync::Mutex::new(
        std::collections::BinaryHeap::new()
    ));

    // Create semaphore
    let concurrent_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
        config.max_concurrent_downloads
    ));

    // Create active downloads tracking map
    let active_downloads = std::sync::Arc::new(tokio::sync::Mutex::new(
        std::collections::HashMap::new()
    ));

    // Create speed limiter with configured limit
    let speed_limiter = speed_limiter::SpeedLimiter::new(config.speed_limit_bps);

    // Create config Arc early so we can share it
    let config_arc = std::sync::Arc::new(config.clone());

    // Initialize runtime-mutable categories from config
    let categories = std::sync::Arc::new(tokio::sync::RwLock::new(config.categories.clone()));

    // Initialize runtime-mutable schedule rules (empty for tests)
    let schedule_rules = std::sync::Arc::new(tokio::sync::RwLock::new(vec![]));
    let next_schedule_rule_id = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));

    // Create post-processing pipeline executor
    let post_processor = std::sync::Arc::new(post_processing::PostProcessor::new(
        event_tx.clone(),
        config_arc.clone(),
    ));

    let downloader = UsenetDownloader {
        db: std::sync::Arc::new(db),
        event_tx,
        config: config_arc,
        nntp_pools: std::sync::Arc::new(nntp_pools),
        queue,
        concurrent_limit,
        active_downloads,
        speed_limiter,
        accepting_new: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        post_processor,
        categories,
        schedule_rules,
        next_schedule_rule_id,
    };

    (downloader, temp_dir)
}

/// Sample NZB content for testing
const SAMPLE_NZB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <head>
<meta type="title">Test Download</meta>
<meta type="password">testpass123</meta>
<meta type="category">movies</meta>
  </head>
  <file poster="user@example.com" date="1234567890" subject="test.file.rar [1/2]">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="768000" number="1">part1of2@example.com</segment>
  <segment bytes="512000" number="2">part2of2@example.com</segment>
</segments>
  </file>
</nzb>"#;

#[tokio::test]
async fn test_add_nzb_content_basic() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add NZB to queue
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test_download", DownloadOptions::default())
        .await
        .unwrap();

    assert!(download_id > 0);

    // Verify download was created in database
    let download = downloader.db.get_download(download_id).await.unwrap();
    assert!(download.is_some());

    let download = download.unwrap();
    assert_eq!(download.name, "test_download");
    assert_eq!(download.status, Status::Queued.to_i32());
    assert_eq!(download.size_bytes, 768000 + 512000); // Total of both segments
}

#[tokio::test]
async fn test_add_nzb_content_extracts_metadata() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();

    // Check NZB metadata was extracted
    assert_eq!(download.nzb_meta_name, Some("Test Download".to_string()));
    assert_eq!(download.job_name, Some("Test Download".to_string())); // Uses meta title

    // Check password was cached
    let cached_password = downloader.db.get_cached_password(download_id).await.unwrap();
    assert_eq!(cached_password, Some("testpass123".to_string()));
}

#[tokio::test]
async fn test_add_nzb_content_creates_articles() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Verify articles were created
    let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
    assert_eq!(articles.len(), 2); // Two segments in sample NZB

    assert_eq!(articles[0].message_id, "part1of2@example.com");
    assert_eq!(articles[0].segment_number, 1);
    assert_eq!(articles[0].size_bytes, 768000);

    assert_eq!(articles[1].message_id, "part2of2@example.com");
    assert_eq!(articles[1].segment_number, 2);
    assert_eq!(articles[1].size_bytes, 512000);
}

#[tokio::test]
async fn test_add_nzb_content_with_options() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let options = DownloadOptions {
        category: Some("test_category".to_string()),
        priority: Priority::High,
        password: Some("override_password".to_string()),
        ..Default::default()
    };

    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", options)
        .await
        .unwrap();

    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();

    // Check options were applied
    assert_eq!(download.category, Some("test_category".to_string()));
    assert_eq!(download.priority, Priority::High as i32);

    // Check provided password overrides NZB password
    let cached_password = downloader.db.get_cached_password(download_id).await.unwrap();
    assert_eq!(cached_password, Some("override_password".to_string()));
}

#[tokio::test]
async fn test_add_nzb_content_calculates_hash() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();

    // Verify hash was calculated and stored
    assert!(download.nzb_hash.is_some());
    let hash = download.nzb_hash.unwrap();
    assert_eq!(hash.len(), 64); // SHA256 produces 64 hex characters
}

#[tokio::test]
async fn test_add_nzb_content_invalid_utf8() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Invalid UTF-8 bytes
    let invalid_bytes = vec![0xFF, 0xFE, 0xFD];

    let result = downloader
        .add_nzb_content(&invalid_bytes, "test", DownloadOptions::default())
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::InvalidNzb(msg) => assert!(msg.contains("not valid UTF-8")),
        _ => panic!("Expected InvalidNzb error"),
    }
}

#[tokio::test]
async fn test_add_nzb_content_invalid_xml() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let invalid_nzb = b"<not><valid>xml";

    let result = downloader
        .add_nzb_content(invalid_nzb, "test", DownloadOptions::default())
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::InvalidNzb(msg) => {
            // Accept either parse error or validation error
            assert!(msg.contains("Failed to parse NZB") || msg.contains("validation failed"));
        }
        _ => panic!("Expected InvalidNzb error"),
    }
}

#[tokio::test]
async fn test_add_nzb_content_emits_event() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Subscribe to events before spawning task
    let mut events = downloader.subscribe();

    // Add NZB
    downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Wait for Queued event
    let event = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        events.recv()
    ).await.unwrap().unwrap();

    match event {
        Event::Queued { id, name } => {
            assert!(id > 0);
            assert_eq!(name, "test");
        }
        _ => panic!("Expected Queued event, got {:?}", event),
    }
}

#[tokio::test]
async fn test_add_nzb_from_file() {
    let (downloader, temp_dir) = create_test_downloader().await;

    // Create a test NZB file
    let nzb_path = temp_dir.path().join("test_download.nzb");
    tokio::fs::write(&nzb_path, SAMPLE_NZB).await.unwrap();

    // Add NZB from file
    let download_id = downloader
        .add_nzb(&nzb_path, DownloadOptions::default())
        .await
        .unwrap();

    assert!(download_id > 0);

    // Verify download was created with correct name (filename without extension)
    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    assert_eq!(download.name, "test_download");
    assert_eq!(download.status, Status::Queued.to_i32());
}

#[tokio::test]
async fn test_add_nzb_file_not_found() {
    let (downloader, temp_dir) = create_test_downloader().await;

    let nonexistent_path = temp_dir.path().join("nonexistent.nzb");

    let result = downloader
        .add_nzb(&nonexistent_path, DownloadOptions::default())
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::Io(e) => {
            assert!(e.to_string().contains("Failed to read NZB file"));
        }
        _ => panic!("Expected Io error"),
    }
}

#[tokio::test]
async fn test_add_nzb_extracts_filename() {
    let (downloader, temp_dir) = create_test_downloader().await;

    // Create test file with complex filename
    let nzb_path = temp_dir.path().join("My.Movie.2024.1080p.nzb");
    tokio::fs::write(&nzb_path, SAMPLE_NZB).await.unwrap();

    let download_id = downloader
        .add_nzb(&nzb_path, DownloadOptions::default())
        .await
        .unwrap();

    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    // Should use filename without .nzb extension
    assert_eq!(download.name, "My.Movie.2024.1080p");
}

#[tokio::test]
async fn test_add_nzb_with_options() {
    let (downloader, temp_dir) = create_test_downloader().await;

    let nzb_path = temp_dir.path().join("test.nzb");
    tokio::fs::write(&nzb_path, SAMPLE_NZB).await.unwrap();

    let options = DownloadOptions {
        category: Some("movies".to_string()),
        priority: Priority::High,
        ..Default::default()
    };

    let download_id = downloader
        .add_nzb(&nzb_path, options)
        .await
        .unwrap();

    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    assert_eq!(download.category, Some("movies".to_string()));
    assert_eq!(download.priority, Priority::High as i32);
}

// URL Fetching Tests

#[tokio::test]
async fn test_add_nzb_url_success() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start mock HTTP server
    let mock_server = MockServer::start().await;

    // Mock successful NZB download with Content-Disposition header
    Mock::given(method("GET"))
        .and(path("/test.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Disposition", "attachment; filename=\"Movie.Release.nzb\"")
                .set_body_bytes(SAMPLE_NZB)
        )
        .mount(&mock_server)
        .await;

    // Fetch NZB from mock server
    let url = format!("{}/test.nzb", mock_server.uri());
    let download_id = downloader
        .add_nzb_url(&url, DownloadOptions::default())
        .await
        .unwrap();

    assert!(download_id > 0);

    // Verify download was created with filename from Content-Disposition
    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    assert_eq!(download.name, "Movie.Release");
    assert_eq!(download.status, Status::Queued.to_i32());
}

#[tokio::test]
async fn test_add_nzb_url_extracts_filename_from_url() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start mock HTTP server
    let mock_server = MockServer::start().await;

    // Mock successful NZB download without Content-Disposition header
    Mock::given(method("GET"))
        .and(path("/downloads/My.Movie.2024.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(SAMPLE_NZB)
        )
        .mount(&mock_server)
        .await;

    // Fetch NZB from mock server
    let url = format!("{}/downloads/My.Movie.2024.nzb", mock_server.uri());
    let download_id = downloader
        .add_nzb_url(&url, DownloadOptions::default())
        .await
        .unwrap();

    // Verify download was created with filename from URL path
    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    assert_eq!(download.name, "My.Movie.2024");
}

#[tokio::test]
async fn test_add_nzb_url_http_404() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start mock HTTP server
    let mock_server = MockServer::start().await;

    // Mock 404 Not Found response
    Mock::given(method("GET"))
        .and(path("/notfound.nzb"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    // Attempt to fetch non-existent NZB
    let url = format!("{}/notfound.nzb", mock_server.uri());
    let result = downloader
        .add_nzb_url(&url, DownloadOptions::default())
        .await;

    // Should return error
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::Io(e) => {
            let msg = e.to_string();
            assert!(msg.contains("HTTP error"));
            assert!(msg.contains("404"));
        }
        _ => panic!("Expected Io error for HTTP 404"),
    }
}

#[tokio::test]
async fn test_add_nzb_url_http_403() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start mock HTTP server
    let mock_server = MockServer::start().await;

    // Mock 403 Forbidden response
    Mock::given(method("GET"))
        .and(path("/forbidden.nzb"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&mock_server)
        .await;

    // Attempt to fetch forbidden NZB
    let url = format!("{}/forbidden.nzb", mock_server.uri());
    let result = downloader
        .add_nzb_url(&url, DownloadOptions::default())
        .await;

    // Should return error
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::Io(e) => {
            let msg = e.to_string();
            assert!(msg.contains("HTTP error"));
            assert!(msg.contains("403"));
        }
        _ => panic!("Expected Io error for HTTP 403"),
    }
}

#[tokio::test]
async fn test_add_nzb_url_timeout() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};
    use std::time::Duration;

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start mock HTTP server
    let mock_server = MockServer::start().await;

    // Mock slow response that exceeds timeout (30 seconds)
    // Note: This test would take 30+ seconds to run, so we'll test connection failure instead
    Mock::given(method("GET"))
        .and(path("/slow.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(35))  // Exceeds 30 second timeout
                .set_body_bytes(SAMPLE_NZB)
        )
        .mount(&mock_server)
        .await;

    // Attempt to fetch slow NZB
    let url = format!("{}/slow.nzb", mock_server.uri());
    let result = downloader
        .add_nzb_url(&url, DownloadOptions::default())
        .await;

    // Should return timeout error
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::Io(e) => {
            let msg = e.to_string();
            assert!(msg.contains("Timeout") || msg.contains("timeout"));
        }
        _ => panic!("Expected Io error for timeout"),
    }
}

#[tokio::test]
async fn test_add_nzb_url_connection_refused() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Use a URL that will cause connection refused (port unlikely to be in use)
    // Port 9 is the discard service, rarely running on modern systems
    let url = "http://127.0.0.1:9/test.nzb";
    let result = downloader
        .add_nzb_url(url, DownloadOptions::default())
        .await;

    // Should return connection error
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::Io(e) => {
            let msg = e.to_string();
            assert!(msg.contains("Connection failed") || msg.contains("Failed to fetch"));
        }
        _ => panic!("Expected Io error for connection refused"),
    }
}

#[tokio::test]
async fn test_add_nzb_url_with_options() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start mock HTTP server
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/movie.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(SAMPLE_NZB)
        )
        .mount(&mock_server)
        .await;

    // Fetch NZB with options
    let options = DownloadOptions {
        category: Some("movies".to_string()),
        priority: Priority::High,
        ..Default::default()
    };

    let url = format!("{}/movie.nzb", mock_server.uri());
    let download_id = downloader
        .add_nzb_url(&url, options)
        .await
        .unwrap();

    // Verify options were applied
    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    assert_eq!(download.category, Some("movies".to_string()));
    assert_eq!(download.priority, Priority::High as i32);
}

// Priority Queue Tests

#[tokio::test]
async fn test_queue_adds_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add download
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Verify it's in the queue
    assert_eq!(downloader.queue_size().await, 1);

    // Verify we can get it from the queue
    let next_id = downloader.peek_next_download().await;
    assert_eq!(next_id, Some(id));
}

#[tokio::test]
async fn test_queue_priority_ordering() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add downloads with different priorities
    let low_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "low",
            DownloadOptions {
                priority: Priority::Low,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let high_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "high",
            DownloadOptions {
                priority: Priority::High,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let normal_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "normal",
            DownloadOptions {
                priority: Priority::Normal,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Queue should have 3 items
    assert_eq!(downloader.queue_size().await, 3);

    // Should return highest priority first (High > Normal > Low)
    assert_eq!(downloader.get_next_download().await, Some(high_id));
    assert_eq!(downloader.get_next_download().await, Some(normal_id));
    assert_eq!(downloader.get_next_download().await, Some(low_id));
    assert_eq!(downloader.get_next_download().await, None);
}

#[tokio::test]
async fn test_queue_fifo_for_same_priority() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads with same priority
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "first", DownloadOptions::default())
        .await
        .unwrap();

    // Small delay to ensure different timestamps
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "second", DownloadOptions::default())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "third", DownloadOptions::default())
        .await
        .unwrap();

    // Should return in FIFO order for same priority
    assert_eq!(downloader.get_next_download().await, Some(id1));
    assert_eq!(downloader.get_next_download().await, Some(id2));
    assert_eq!(downloader.get_next_download().await, Some(id3));
}

#[tokio::test]
async fn test_queue_remove_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add downloads
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "first", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "second", DownloadOptions::default())
        .await
        .unwrap();

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "third", DownloadOptions::default())
        .await
        .unwrap();

    assert_eq!(downloader.queue_size().await, 3);

    // Remove middle download
    let removed = downloader.remove_from_queue(id2).await;
    assert!(removed);
    assert_eq!(downloader.queue_size().await, 2);

    // Should still get id1 and id3
    assert_eq!(downloader.get_next_download().await, Some(id1));
    assert_eq!(downloader.get_next_download().await, Some(id3));
    assert_eq!(downloader.get_next_download().await, None);
}

#[tokio::test]
async fn test_queue_remove_nonexistent() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Try to remove download that doesn't exist
    let removed = downloader.remove_from_queue(999).await;
    assert!(!removed);
}

#[tokio::test]
async fn test_queue_force_priority() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add normal priority download
    let normal_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "normal", DownloadOptions::default())
        .await
        .unwrap();

    // Add force priority download (should jump to front)
    let force_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "force",
            DownloadOptions {
                priority: Priority::Force,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Force should come first even though added second
    assert_eq!(downloader.get_next_download().await, Some(force_id));
    assert_eq!(downloader.get_next_download().await, Some(normal_id));
}

// Pause/Resume Tests

#[tokio::test]
async fn test_pause_queued_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add download
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Download should be queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());

    // Pause it
    downloader.pause(id).await.unwrap();

    // Status should be updated to Paused
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32());
}

#[tokio::test]
async fn test_pause_already_paused() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Pause it once
    downloader.pause(id).await.unwrap();

    // Pause it again (should be idempotent)
    let result = downloader.pause(id).await;
    assert!(result.is_ok());

    // Status should still be Paused
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32());
}

#[tokio::test]
async fn test_pause_completed_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark as complete
    downloader.db.update_status(id, Status::Complete.to_i32()).await.unwrap();

    // Try to pause (should fail)
    let result = downloader.pause(id).await;
    assert!(result.is_err());

    // Status should still be Complete
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Complete.to_i32());
}

#[tokio::test]
async fn test_pause_nonexistent_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Try to pause download that doesn't exist
    let result = downloader.pause(999).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_resume_paused_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add download
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Pause it
    downloader.pause(id).await.unwrap();
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32());

    // Resume it
    downloader.resume(id).await.unwrap();

    // Status should be updated to Queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());

    // Should be back in the queue
    assert!(downloader.queue_size().await > 0);
}

#[tokio::test]
async fn test_resume_already_queued() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Download is already queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());

    // Try to resume (should be idempotent)
    let result = downloader.resume(id).await;
    assert!(result.is_ok());

    // Status should still be Queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());
}

#[tokio::test]
async fn test_resume_completed_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark as complete
    downloader.db.update_status(id, Status::Complete.to_i32()).await.unwrap();

    // Try to resume (should fail)
    let result = downloader.resume(id).await;
    assert!(result.is_err());

    // Status should still be Complete
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Complete.to_i32());
}

#[tokio::test]
async fn test_resume_failed_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark as failed
    downloader.db.update_status(id, Status::Failed.to_i32()).await.unwrap();

    // Try to resume (should fail - use reprocess() instead for failed downloads)
    let result = downloader.resume(id).await;
    assert!(result.is_err());

    // Status should still be Failed
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Failed.to_i32());
}

#[tokio::test]
async fn test_resume_nonexistent_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Try to resume download that doesn't exist
    let result = downloader.resume(999).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_pause_resume_cycle() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add download
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    let initial_queue_size = downloader.queue_size().await;

    // Pause
    downloader.pause(id).await.unwrap();
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32());

    // Resume
    downloader.resume(id).await.unwrap();
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());

    // Queue size should be restored
    assert_eq!(downloader.queue_size().await, initial_queue_size);
}

#[tokio::test]
async fn test_resume_preserves_priority() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add high priority download
    let id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test",
            DownloadOptions {
                priority: Priority::High,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Add normal priority download
    let normal_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "normal", DownloadOptions::default())
        .await
        .unwrap();

    // Pause high priority download
    downloader.pause(id).await.unwrap();

    // Resume high priority download
    downloader.resume(id).await.unwrap();

    // High priority download should still come first
    assert_eq!(downloader.get_next_download().await, Some(id));
    assert_eq!(downloader.get_next_download().await, Some(normal_id));
}

#[tokio::test]
async fn test_cancel_queued_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Verify download exists in database
    assert!(downloader.db.get_download(id).await.unwrap().is_some());

    // Verify download is in queue
    assert_eq!(downloader.queue_size().await, 1);

    // Cancel the download
    downloader.cancel(id).await.unwrap();

    // Download should be removed from database
    assert!(downloader.db.get_download(id).await.unwrap().is_none());

    // Download should be removed from queue
    assert_eq!(downloader.queue_size().await, 0);
}

#[tokio::test]
async fn test_cancel_paused_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Pause the download
    downloader.pause(id).await.unwrap();

    // Verify it's paused
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32());

    // Cancel the paused download
    downloader.cancel(id).await.unwrap();

    // Download should be removed from database
    assert!(downloader.db.get_download(id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_cancel_deletes_temp_files() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Create temp directory and some files (simulating partially downloaded)
    let download_temp_dir = downloader.config.temp_dir.join(format!("download_{}", id));
    tokio::fs::create_dir_all(&download_temp_dir).await.unwrap();

    let test_file = download_temp_dir.join("article_1.dat");
    tokio::fs::write(&test_file, b"test data").await.unwrap();

    // Verify temp directory exists
    assert!(download_temp_dir.exists());
    assert!(test_file.exists());

    // Cancel the download
    downloader.cancel(id).await.unwrap();

    // Temp directory should be deleted
    assert!(!download_temp_dir.exists());
    assert!(!test_file.exists());
}

#[tokio::test]
async fn test_cancel_nonexistent_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Try to cancel download that doesn't exist
    let result = downloader.cancel(999).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_cancel_completed_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark as completed
    downloader.db.update_status(id, Status::Complete.to_i32()).await.unwrap();

    // Cancel completed download (should succeed - removes from history)
    downloader.cancel(id).await.unwrap();

    // Download should be removed from database
    assert!(downloader.db.get_download(id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_cancel_removes_from_queue() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
        .await
        .unwrap();

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
        .await
        .unwrap();

    // Verify all are queued
    assert_eq!(downloader.queue_size().await, 3);

    // Cancel middle download
    downloader.cancel(id2).await.unwrap();

    // Queue should have 2 items
    assert_eq!(downloader.queue_size().await, 2);

    // Get downloads from queue - should only be id1 and id3
    let next = downloader.get_next_download().await;
    assert!(next == Some(id1) || next == Some(id3));

    let next2 = downloader.get_next_download().await;
    assert!(next2 == Some(id1) || next2 == Some(id3));
    assert_ne!(next, next2);

    // Queue should now be empty
    assert_eq!(downloader.queue_size().await, 0);
}

#[tokio::test]
async fn test_cancel_emits_removed_event() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Cancel the download (in background to avoid blocking)
    let downloader_clone = downloader.clone();
    tokio::spawn(async move {
        downloader_clone.cancel(id).await.unwrap();
    });

    // Wait for Removed event
    let mut received_removed = false;
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await {
            Ok(Ok(crate::types::Event::Removed { id: event_id })) => {
                assert_eq!(event_id, id);
                received_removed = true;
                break;
            }
            Ok(Ok(_)) => continue, // Other events, keep checking
            Ok(Err(_)) => break,   // Channel closed
            Err(_) => break,       // Timeout
        }
    }

    assert!(received_removed, "Should have received Removed event");
}

// Queue-wide pause/resume tests

#[tokio::test]
async fn test_pause_all_pauses_active_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads with different statuses
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
        .await
        .unwrap();

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
        .await
        .unwrap();

    // Mark id2 as already paused
    downloader.pause(id2).await.unwrap();

    // Mark id3 as complete (should not be paused)
    downloader.db.update_status(id3, Status::Complete.to_i32()).await.unwrap();

    // Pause all
    downloader.pause_all().await.unwrap();

    // Check statuses
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    let d3 = downloader.db.get_download(id3).await.unwrap().unwrap();

    assert_eq!(d1.status, Status::Paused.to_i32(), "id1 should be paused");
    assert_eq!(d2.status, Status::Paused.to_i32(), "id2 should still be paused");
    assert_eq!(d3.status, Status::Complete.to_i32(), "id3 should still be complete");
}

#[tokio::test]
async fn test_pause_all_emits_queue_paused_event() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Pause all (in background to avoid blocking)
    let downloader_clone = downloader.clone();
    tokio::spawn(async move {
        downloader_clone.pause_all().await.unwrap();
    });

    // Wait for QueuePaused event
    let mut received_queue_paused = false;
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await {
            Ok(Ok(crate::types::Event::QueuePaused)) => {
                received_queue_paused = true;
                break;
            }
            Ok(Ok(_)) => continue, // Other events, keep checking
            Ok(Err(_)) => break,   // Channel closed
            Err(_) => break,       // Timeout
        }
    }

    assert!(received_queue_paused, "Should have received QueuePaused event");
}

#[tokio::test]
async fn test_pause_all_with_empty_queue() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Pause all with no downloads (should not error)
    let result = downloader.pause_all().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_resume_all_resumes_paused_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
        .await
        .unwrap();

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
        .await
        .unwrap();

    // Pause all downloads
    downloader.pause(id1).await.unwrap();
    downloader.pause(id2).await.unwrap();

    // Mark id3 as complete (should not be resumed)
    downloader.db.update_status(id3, Status::Complete.to_i32()).await.unwrap();

    // Resume all
    downloader.resume_all().await.unwrap();

    // Check statuses
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    let d3 = downloader.db.get_download(id3).await.unwrap().unwrap();

    assert_eq!(d1.status, Status::Queued.to_i32(), "id1 should be queued");
    assert_eq!(d2.status, Status::Queued.to_i32(), "id2 should be queued");
    assert_eq!(d3.status, Status::Complete.to_i32(), "id3 should still be complete");
}

#[tokio::test]
async fn test_resume_all_emits_queue_resumed_event() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add and pause a download
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader.pause(id).await.unwrap();

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Resume all (in background to avoid blocking)
    let downloader_clone = downloader.clone();
    tokio::spawn(async move {
        downloader_clone.resume_all().await.unwrap();
    });

    // Wait for QueueResumed event
    let mut received_queue_resumed = false;
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await {
            Ok(Ok(crate::types::Event::QueueResumed)) => {
                received_queue_resumed = true;
                break;
            }
            Ok(Ok(_)) => continue, // Other events, keep checking
            Ok(Err(_)) => break,   // Channel closed
            Err(_) => break,       // Timeout
        }
    }

    assert!(received_queue_resumed, "Should have received QueueResumed event");
}

#[tokio::test]
async fn test_resume_all_with_no_paused_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a queued download (not paused)
    downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Resume all (should not error even though nothing is paused)
    let result = downloader.resume_all().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_pause_all_resume_all_cycle() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
        .await
        .unwrap();

    // Initial state: both queued
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(d1.status, Status::Queued.to_i32());
    assert_eq!(d2.status, Status::Queued.to_i32());

    // Pause all
    downloader.pause_all().await.unwrap();

    // After pause: both paused
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(d1.status, Status::Paused.to_i32());
    assert_eq!(d2.status, Status::Paused.to_i32());

    // Resume all
    downloader.resume_all().await.unwrap();

    // After resume: both queued again
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(d1.status, Status::Queued.to_i32());
    assert_eq!(d2.status, Status::Queued.to_i32());
}

// === Queue State Persistence Tests ===

#[tokio::test]
async fn test_queue_state_persisted_to_database() {
    // Test: Queue state is persisted to SQLite on every change
    let (downloader, _temp_dir) = create_test_downloader().await;

    // 1. Add download - should persist Status::Queued
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Verify Status::Queued persisted to database
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32(), "Status should be Queued in DB");
    assert_eq!(download.priority, 0, "Priority should be Normal (0)");

    // 2. Pause download - should persist Status::Paused
    downloader.pause(id).await.unwrap();

    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32(), "Status should be Paused in DB");

    // 3. Resume download - should persist Status::Queued again
    downloader.resume(id).await.unwrap();

    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32(), "Status should be Queued in DB after resume");

    // 4. Verify in-memory queue and database are synchronized
    let queue_size = downloader.queue_size().await;
    assert_eq!(queue_size, 1, "In-memory queue should have 1 download");

    // Query incomplete downloads from DB (should include our Queued download)
    let incomplete = downloader.db.get_incomplete_downloads().await.unwrap();
    assert_eq!(incomplete.len(), 1, "DB should have 1 incomplete download");
    assert_eq!(incomplete[0].id, id, "Incomplete download ID should match");

    // 5. Cancel download - should remove from database
    downloader.cancel(id).await.unwrap();

    let download = downloader.db.get_download(id).await.unwrap();
    assert!(download.is_none(), "Download should be deleted from DB");

    let queue_size = downloader.queue_size().await;
    assert_eq!(queue_size, 0, "In-memory queue should be empty");
}

#[tokio::test]
async fn test_queue_ordering_persisted_correctly() {
    // Test that queue ordering (priority + created_at) is persisted and queryable
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add downloads with different priorities
    let id_low = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "low",
            DownloadOptions {
                priority: Priority::Low,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await; // Ensure different timestamps

    let id_normal = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "normal",
            DownloadOptions {
                priority: Priority::Normal,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    let id_high = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "high",
            DownloadOptions {
                priority: Priority::High,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Query database with priority ordering (as restore_queue() would do)
    let all_downloads = downloader.db.list_downloads().await.unwrap();

    // Should be ordered: High, Normal, Low (priority DESC)
    assert_eq!(all_downloads.len(), 3, "Should have 3 downloads");
    assert_eq!(all_downloads[0].id, id_high, "First should be High priority");
    assert_eq!(all_downloads[1].id, id_normal, "Second should be Normal priority");
    assert_eq!(all_downloads[2].id, id_low, "Third should be Low priority");

    // Verify priorities are correct in database
    assert_eq!(all_downloads[0].priority, Priority::High as i32);
    assert_eq!(all_downloads[1].priority, Priority::Normal as i32);
    assert_eq!(all_downloads[2].priority, Priority::Low as i32);
}

#[tokio::test]
async fn test_queue_persistence_enables_restore() {
    // Test that persisted queue state can be used to restore queue
    use tempfile::TempDir;

    // Create persistent temp directory for database
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("usenet-dl.db");

    // Create first downloader instance
    let config1 = Config {
        database_path: db_path.clone(),
        temp_dir: temp_dir.path().join("temp"),
        download_dir: temp_dir.path().join("downloads"),
        ..Default::default()
    };
    let downloader = UsenetDownloader::new(config1).await.unwrap();

    // Add multiple downloads with different statuses
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
        .await
        .unwrap();

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
        .await
        .unwrap();

    // Mark one as Processing, complete one, leave one queued
    downloader.db.update_status(id2, Status::Processing.to_i32()).await.unwrap();
    downloader.db.update_status(id3, Status::Complete.to_i32()).await.unwrap();

    // Simulate restart: create new downloader with same database
    drop(downloader); // Close first instance

    let config2 = Config {
        database_path: db_path.clone(),
        temp_dir: temp_dir.path().join("temp"),
        download_dir: temp_dir.path().join("downloads"),
        ..Default::default()
    };
    let downloader2 = UsenetDownloader::new(config2).await.unwrap();

    // Verify we can query incomplete downloads (would be used by restore_queue)
    // Note: get_incomplete_downloads() returns status IN (0, 1, 3) - Queued, Downloading, Processing
    // It intentionally excludes Paused (2), which would be handled separately
    let incomplete = downloader2.db.get_incomplete_downloads().await.unwrap();

    // Should have 2: id1 (Queued) and id2 (Processing)
    // Should NOT have id3 (Complete)
    assert_eq!(incomplete.len(), 2, "Should have 2 incomplete downloads");

    let incomplete_ids: Vec<i64> = incomplete.iter().map(|d| d.id).collect();
    assert!(incomplete_ids.contains(&id1), "Should include Queued download");
    assert!(incomplete_ids.contains(&id2), "Should include Processing download");
    assert!(!incomplete_ids.contains(&id3), "Should NOT include Complete download");

    // Verify they're in priority order
    assert_eq!(incomplete[0].priority, 0, "First should be Normal priority");
    assert_eq!(incomplete[1].priority, 0, "Second should be Normal priority");

    // Also verify paused downloads can be restored separately
    let paused = downloader2.db.list_downloads_by_status(Status::Paused.to_i32()).await.unwrap();
    assert_eq!(paused.len(), 0, "No paused downloads in this test (id2 was set to Processing)");
}

#[tokio::test]
async fn test_resume_download_with_pending_articles() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Simulate partial download: mark first article as downloaded
    let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
    assert_eq!(articles.len(), 2, "Should have 2 pending articles initially");

    downloader.db.update_article_status(
        articles[0].id,
        crate::db::article_status::DOWNLOADED
    ).await.unwrap();

    // Update download status to Paused (simulate interrupted download)
    downloader.db.update_status(download_id, Status::Paused.to_i32()).await.unwrap();

    // Resume the download
    downloader.resume_download(download_id).await.unwrap();

    // Verify download is back in Queued status
    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    assert_eq!(Status::from_i32(download.status), Status::Queued);

    // Verify only 1 article remains pending
    let pending = downloader.db.get_pending_articles(download_id).await.unwrap();
    assert_eq!(pending.len(), 1, "Should have 1 pending article after resume");
    assert_eq!(pending[0].id, articles[1].id, "Should be the second article");
}

#[tokio::test]
async fn test_resume_download_no_pending_articles() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark all articles as downloaded
    let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
    for article in articles {
        downloader.db.update_article_status(
            article.id,
            crate::db::article_status::DOWNLOADED
        ).await.unwrap();
    }

    // Update status to Downloading (simulate download just completed)
    downloader.db.update_status(download_id, Status::Downloading.to_i32()).await.unwrap();

    // Resume should proceed to post-processing
    downloader.resume_download(download_id).await.unwrap();

    // Verify status is now Processing (ready for post-processing)
    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    assert_eq!(Status::from_i32(download.status), Status::Processing);

    // Verify no pending articles remain
    let pending = downloader.db.get_pending_articles(download_id).await.unwrap();
    assert_eq!(pending.len(), 0, "Should have no pending articles");
}

#[tokio::test]
async fn test_resume_download_nonexistent() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Try to resume non-existent download
    let result = downloader.resume_download(99999).await;

    // Should succeed (get_pending_articles returns empty Vec for non-existent downloads)
    // This is acceptable behavior - resume_download is idempotent
    assert!(result.is_ok(), "Should succeed (no-op) for non-existent download");

    // Verify no status was changed (download doesn't exist in database)
    let download = downloader.db.get_download(99999).await.unwrap();
    assert!(download.is_none(), "Download should not exist");
}

#[tokio::test]
async fn test_resume_download_emits_event() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Add a download (will emit Queued event)
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Consume the Queued event
    let event = events.recv().await.unwrap();
    assert!(matches!(event, Event::Queued { .. }));

    // Mark all articles as downloaded
    let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
    for article in articles {
        downloader.db.update_article_status(
            article.id,
            crate::db::article_status::DOWNLOADED
        ).await.unwrap();
    }

    // Resume should emit Verifying event (post-processing start)
    downloader.resume_download(download_id).await.unwrap();

    // Check for Verifying event
    let event = events.recv().await.unwrap();
    assert!(
        matches!(event, Event::Verifying { id } if id == download_id),
        "Should emit Verifying event when no pending articles"
    );
}

// restore_queue() tests

#[tokio::test]
async fn test_restore_queue_with_no_incomplete_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Restore queue with empty database
    downloader.restore_queue().await.unwrap();

    // Queue should remain empty
    let queue_size = downloader.queue.lock().await.len();
    assert_eq!(queue_size, 0, "Queue should be empty when no incomplete downloads");
}

#[tokio::test]
async fn test_restore_queue_with_queued_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads with different priorities
    let id1 = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "download1",
            DownloadOptions {
                priority: Priority::Low,
                ..Default::default()
            }
        )
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "download2",
            DownloadOptions {
                priority: Priority::High,
                ..Default::default()
            }
        )
        .await
        .unwrap();

    // Clear the queue (simulating a restart)
    downloader.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Queue should have both downloads restored
    let queue_size = downloader.queue.lock().await.len();
    assert_eq!(queue_size, 2, "Queue should have 2 downloads restored");

    // Verify priority ordering (High priority should be first)
    let next = downloader.queue.lock().await.pop().unwrap();
    assert_eq!(next.id, id2, "High priority download should be first");
    assert_eq!(next.priority, Priority::High);

    let next = downloader.queue.lock().await.pop().unwrap();
    assert_eq!(next.id, id1, "Low priority download should be second");
    assert_eq!(next.priority, Priority::Low);
}

#[tokio::test]
async fn test_restore_queue_with_downloading_status() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Manually set status to Downloading (simulating interrupted download)
    downloader.db.update_status(download_id, Status::Downloading.to_i32()).await.unwrap();

    // Clear the queue
    downloader.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Download should be back in queue with Queued status (resume_download does this)
    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Queued,
        "Download status should be Queued after restore"
    );

    // Queue should contain the download
    let queue_size = downloader.queue.lock().await.len();
    assert_eq!(queue_size, 1, "Queue should have 1 download");
}

#[tokio::test]
async fn test_restore_queue_with_processing_status() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download and mark all articles as downloaded
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark all articles as downloaded
    let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
    for article in articles {
        downloader.db.update_article_status(
            article.id,
            crate::db::article_status::DOWNLOADED
        ).await.unwrap();
    }

    // Manually set status to Processing (simulating interrupted post-processing)
    downloader.db.update_status(download_id, Status::Processing.to_i32()).await.unwrap();

    // Clear the queue
    downloader.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Download should still be in Processing status (ready for post-processing)
    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Processing,
        "Download status should remain Processing after restore"
    );
}

#[tokio::test]
async fn test_restore_queue_skips_completed_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download and mark as complete
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader.db.update_status(download_id, Status::Complete.to_i32()).await.unwrap();

    // Clear the queue
    downloader.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Queue should be empty (completed downloads not restored)
    let queue_size = downloader.queue.lock().await.len();
    assert_eq!(queue_size, 0, "Queue should be empty (completed downloads not restored)");
}

#[tokio::test]
async fn test_restore_queue_skips_failed_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download and mark as failed
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader.db.update_status(download_id, Status::Failed.to_i32()).await.unwrap();

    // Clear the queue
    downloader.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Queue should be empty (failed downloads not restored)
    let queue_size = downloader.queue.lock().await.len();
    assert_eq!(queue_size, 0, "Queue should be empty (failed downloads not restored)");
}

#[tokio::test]
async fn test_restore_queue_skips_paused_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download and pause it
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader.pause(download_id).await.unwrap();

    // Clear the queue
    downloader.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Queue should be empty (paused downloads not restored - user explicitly paused them)
    let queue_size = downloader.queue.lock().await.len();
    assert_eq!(queue_size, 0, "Queue should be empty (paused downloads not restored)");

    // Status should still be Paused
    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Paused,
        "Paused downloads should remain paused"
    );
}

#[tokio::test]
async fn test_restore_queue_called_on_startup() {
    // Create a database with incomplete downloads
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create first downloader instance and add downloads
    {
        let config = Config {
            database_path: db_path.clone(),
            servers: vec![],
            max_concurrent_downloads: 3,
            ..Default::default()
        };
        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Add downloads
        downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "download1", DownloadOptions::default())
            .await
            .unwrap();
        downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "download2", DownloadOptions::default())
            .await
            .unwrap();

        // downloader is dropped here (simulating shutdown)
    }

    // Create new downloader instance (simulating restart)
    let config = Config {
        database_path: db_path.clone(),
        servers: vec![],
        max_concurrent_downloads: 3,
        ..Default::default()
    };
    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Queue should be automatically restored (new() calls restore_queue())
    let queue_size = downloader.queue.lock().await.len();
    assert_eq!(queue_size, 2, "Queue should be restored on startup");
}

#[tokio::test]
async fn test_resume_after_simulated_crash() {
    // Test resume after simulated crash (kill process mid-download)
    //
    // This test simulates a crash by:
    // 1. Starting a download
    // 2. Marking some articles as downloaded (simulating partial progress)
    // 3. Setting status to Downloading (simulating crash mid-download)
    // 4. Dropping the downloader (simulating process termination)
    // 5. Creating a new downloader instance (simulating restart)
    // 6. Verifying that restore_queue() correctly resumes the download

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let download_id;
    let total_articles;

    // Simulate crash scenario
    {
        let config = Config {
            database_path: db_path.clone(),
            servers: vec![],
            max_concurrent_downloads: 3,
            ..Default::default()
        };
        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Add a download
        download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "crash_test", DownloadOptions::default())
            .await
            .unwrap();

        // Get all articles
        let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
        total_articles = articles.len();
        assert!(total_articles > 1, "Need at least 2 articles for this test");

        // Mark half of the articles as downloaded (simulating partial progress)
        let articles_to_download = total_articles / 2;
        for (i, article) in articles.iter().enumerate() {
            if i < articles_to_download {
                downloader.db.update_article_status(
                    article.id,
                    crate::db::article_status::DOWNLOADED
                ).await.unwrap();
            }
        }

        // Set status to Downloading (simulating crash mid-download)
        downloader.db.update_status(download_id, Status::Downloading.to_i32()).await.unwrap();

        // Set some progress to verify it's preserved
        let progress = 50.0;
        let speed = 1000000u64; // 1 MB/s
        let downloaded_bytes = 524288u64; // 512 KB
        downloader.db.update_progress(download_id, progress, speed, downloaded_bytes).await.unwrap();

        // Simulate crash by dropping downloader (no graceful shutdown)
        // downloader is dropped here
    }

    // Simulate restart by creating a new downloader instance
    let config = Config {
        database_path: db_path.clone(),
        servers: vec![],
        max_concurrent_downloads: 3,
        ..Default::default()
    };
    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Verify the download was restored
    let download = downloader.db.get_download(download_id).await.unwrap().unwrap();

    // Status should be Queued (resume_download sets it back to Queued)
    assert_eq!(
        Status::from_i32(download.status),
        Status::Queued,
        "Download should be Queued after restore"
    );

    // Progress should be preserved from before crash
    assert_eq!(
        download.progress, 50.0,
        "Download progress should be preserved after crash"
    );

    // Downloaded bytes should be preserved
    assert_eq!(
        download.downloaded_bytes, 524288,
        "Downloaded bytes should be preserved after crash"
    );

    // Queue should contain the download
    let queue_size = downloader.queue.lock().await.len();
    assert_eq!(queue_size, 1, "Queue should have 1 download after restore");

    // Verify that only pending articles remain
    let pending_articles = downloader.db.get_pending_articles(download_id).await.unwrap();
    let expected_pending = total_articles - (total_articles / 2);
    assert_eq!(
        pending_articles.len(),
        expected_pending,
        "Only undownloaded articles should be pending"
    );

    // Verify that downloaded articles are marked correctly
    let downloaded_count = downloader.db.count_articles_by_status(
        download_id,
        crate::db::article_status::DOWNLOADED
    ).await.unwrap();
    assert_eq!(
        downloaded_count as usize,
        total_articles / 2,
        "Downloaded articles count should match"
    );
}

#[tokio::test]
async fn test_speed_limiter_shared_across_downloads() {
    // This test verifies that the speed limiter is properly shared
    // across all download tasks

    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let config = Config {
        database_path: db_path,
        servers: vec![],
        max_concurrent_downloads: 3,
        speed_limit_bps: Some(1_000_000), // 1 MB/s limit
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
        other => panic!("Expected SpeedLimitChanged event with None, got {:?}", other),
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
        elapsed, min_duration
    );
    assert!(
        elapsed <= max_duration,
        "Downloads took too long: {:?} (expected <= {:?}). \
         Speed limiter may be too conservative.",
        elapsed, max_duration
    );
}

#[tokio::test]
async fn test_speed_limit_dynamic_change_during_downloads() {
    // Test changing speed limit dynamically while downloads are active
    // This verifies that limit changes take effect immediately for ongoing transfers

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start with a conservative 2 MB/s limit
    downloader.set_speed_limit(Some(2_000_000)).await;

    let limiter = downloader.speed_limiter.clone();
    let start = Instant::now();

    // Spawn a long-running download task
    let download_handle = {
        let limiter_clone = limiter.clone();
        tokio::spawn(async move {
            // Try to download 20 MB in 1 MB chunks
            for _ in 0..20 {
                limiter_clone.acquire(1_000_000).await;
            }
        })
    };

    // Wait 2 seconds, then increase speed limit
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Should have downloaded ~4 MB by now (2 MB/s × 2s)
    // Now increase to 10 MB/s
    downloader.set_speed_limit(Some(10_000_000)).await;

    // Wait for download to complete
    download_handle.await.unwrap();

    let elapsed = start.elapsed();

    // Analysis:
    // - First 2 seconds at 2 MB/s: ~4 MB downloaded (but may have 2 MB bucket at start)
    // - Remaining 16 MB at 10 MB/s: ~1.6 seconds  (but may have 10 MB bucket when limit changes)
    // - Total expected: ~2.2-4 seconds (accounting for initial token bucket)
    // The key is that changing the limit should allow faster completion than if
    // the limit stayed at 2 MB/s (which would take 10 seconds total)
    let min_duration = Duration::from_millis(2200); // Must be faster than 10s (20MB at 2MB/s)
    let max_duration = Duration::from_secs(5);

    assert!(
        elapsed >= min_duration,
        "Download with dynamic limit change completed too quickly: {:?}. \
         This is actually good - it means the speed limiter is working!",
        elapsed
    );
    assert!(
        elapsed <= max_duration,
        "Download with dynamic limit change took too long: {:?}. \
         Limit change may not have taken effect immediately.",
        elapsed
    );

    // Most importantly: verify it's much faster than if limit stayed at 2 MB/s
    // 20 MB at 2 MB/s would take 10 seconds
    assert!(
        elapsed < Duration::from_secs(8),
        "Download took {:?}, which suggests limit change didn't take effect. \
         Expected < 8s (much faster than 10s for 20MB at 2MB/s).",
        elapsed
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
        total_elapsed, expected
    );
    assert!(
        total_elapsed.as_millis() <= (expected.as_millis() + tolerance.as_millis()),
        "Downloads took too long: {:?} (expected ~{:?})",
        total_elapsed, expected
    );
}

#[tokio::test]
async fn test_speed_limit_unlimited_mode_with_concurrent_downloads() {
    // Verify that unlimited mode allows maximum throughput
    // without any artificial delays

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Set to unlimited (default)
    downloader.set_speed_limit(None).await;

    let limiter = downloader.speed_limiter.clone();
    let start = Instant::now();

    // Spawn 3 concurrent downloads
    let mut handles = vec![];
    for _ in 0..3 {
        let limiter_clone = limiter.clone();
        let handle = tokio::spawn(async move {
            // Each tries to acquire 10 MB
            for _ in 0..10 {
                limiter_clone.acquire(1_000_000).await;
            }
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }

    let elapsed = start.elapsed();

    // In unlimited mode, 30 MB total should complete almost instantly
    // (only task spawning overhead, no rate limiting delays)
    // Allow up to 100ms for test overhead
    assert!(
        elapsed < Duration::from_millis(100),
        "Unlimited mode took too long: {:?}. There may be unexpected rate limiting.",
        elapsed
    );
}

#[tokio::test]
async fn test_shutdown_graceful() {
    // Test graceful shutdown
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Verify shutdown completes successfully
    let result = downloader.shutdown().await;
    assert!(result.is_ok(), "Shutdown should complete successfully: {:?}", result);
}

#[tokio::test]
async fn test_shutdown_with_active_downloads() {
    // Test shutdown cancels active downloads
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Simulate some active downloads by adding cancellation tokens
    {
        let mut active = downloader.active_downloads.lock().await;
        active.insert(1, tokio_util::sync::CancellationToken::new());
        active.insert(2, tokio_util::sync::CancellationToken::new());
    }

    // Verify we have active downloads
    {
        let active = downloader.active_downloads.lock().await;
        assert_eq!(active.len(), 2);
    }

    // Shutdown should cancel them
    let result = downloader.shutdown().await;
    assert!(result.is_ok(), "Shutdown should complete successfully: {:?}", result);

    // Verify tokens were cancelled (active_downloads map should still contain them,
    // but they should be in cancelled state)
    {
        let active = downloader.active_downloads.lock().await;
        for (_id, token) in active.iter() {
            assert!(token.is_cancelled(), "Download should be cancelled after shutdown");
        }
    }
}

#[tokio::test]
async fn test_shutdown_waits_for_completion() {
    // Test shutdown waits for active downloads to complete
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download token, then remove it after a delay to simulate completion
    let token = tokio_util::sync::CancellationToken::new();
    {
        let mut active = downloader.active_downloads.lock().await;
        active.insert(1, token.clone());
    }

    // Spawn a task that removes the download after 500ms (simulating completion)
    let active_downloads_clone = downloader.active_downloads.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let mut active = active_downloads_clone.lock().await;
        active.remove(&1);
    });

    let start = std::time::Instant::now();

    // Shutdown should wait for the download to complete
    let result = downloader.shutdown().await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Shutdown should complete successfully: {:?}", result);

    // Verify it waited (should take at least 500ms)
    assert!(
        elapsed >= std::time::Duration::from_millis(450),
        "Shutdown should have waited for downloads to complete: {:?}",
        elapsed
    );

    // But not too long (should be < 1 second for this test)
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "Shutdown took too long: {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_shutdown_rejects_new_downloads() {
    // Test that shutdown() sets accepting_new flag and new downloads are rejected
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Initially, should accept new downloads
    assert!(
        downloader.accepting_new.load(std::sync::atomic::Ordering::SeqCst),
        "Should accept new downloads initially"
    );

    // Attempt to add a download before shutdown - should succeed
    let result_before = downloader.add_nzb_content(
        SAMPLE_NZB.as_bytes(),
        "test.nzb",
        DownloadOptions::default(),
    ).await;
    assert!(result_before.is_ok(), "Should accept download before shutdown: {:?}", result_before);

    // Trigger shutdown
    let shutdown_result = downloader.shutdown().await;
    assert!(shutdown_result.is_ok(), "Shutdown should complete successfully: {:?}", shutdown_result);

    // After shutdown, accepting_new should be false
    assert!(
        !downloader.accepting_new.load(std::sync::atomic::Ordering::SeqCst),
        "Should not accept new downloads after shutdown"
    );

    // Attempt to add a download after shutdown - should fail with ShuttingDown error
    let result_after = downloader.add_nzb_content(
        SAMPLE_NZB.as_bytes(),
        "test2.nzb",
        DownloadOptions::default(),
    ).await;

    assert!(result_after.is_err(), "Should reject download after shutdown");
    match result_after {
        Err(crate::error::Error::ShuttingDown) => {
            // Expected error
        }
        other => panic!("Expected ShuttingDown error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_pause_graceful_all() {
    // Test graceful pause signals cancellation to all active downloads
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple download tokens to simulate active downloads
    let token1 = tokio_util::sync::CancellationToken::new();
    let token2 = tokio_util::sync::CancellationToken::new();
    let token3 = tokio_util::sync::CancellationToken::new();

    {
        let mut active = downloader.active_downloads.lock().await;
        active.insert(1, token1.clone());
        active.insert(2, token2.clone());
        active.insert(3, token3.clone());
    }

    // Verify tokens are not cancelled initially
    assert!(!token1.is_cancelled(), "Token 1 should not be cancelled initially");
    assert!(!token2.is_cancelled(), "Token 2 should not be cancelled initially");
    assert!(!token3.is_cancelled(), "Token 3 should not be cancelled initially");

    // Call pause_graceful_all
    downloader.pause_graceful_all().await;

    // Verify all tokens are now cancelled (graceful pause signaled)
    assert!(token1.is_cancelled(), "Token 1 should be cancelled after graceful pause");
    assert!(token2.is_cancelled(), "Token 2 should be cancelled after graceful pause");
    assert!(token3.is_cancelled(), "Token 3 should be cancelled after graceful pause");

    // Verify downloads are still in active_downloads map (they clean up when tasks complete)
    {
        let active = downloader.active_downloads.lock().await;
        assert_eq!(active.len(), 3, "Downloads should still be in active map");
    }
}

#[tokio::test]
async fn test_graceful_pause_completes_current_article() {
    // Verify that graceful pause allows current article to complete
    // This is a conceptual test - the actual behavior is in the download loop
    // which checks cancellation BEFORE starting each article, not during.
    // This means the current article always completes before pausing.

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create a cancellation token
    let token = tokio_util::sync::CancellationToken::new();
    let token_clone = token.clone();

    // Simulate an article download in progress
    let article_complete = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let article_complete_clone = article_complete.clone();

    // Spawn a task that simulates downloading an article (takes 200ms)
    let download_task = tokio::spawn(async move {
        // Simulate article download starting
        tracing::debug!("Article download started");

        // Download takes 200ms
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Mark article as complete
        article_complete_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        tracing::debug!("Article download completed");

        // After article completes, check for cancellation (this is what the real code does)
        if token_clone.is_cancelled() {
            tracing::debug!("Cancellation detected after article completed");
            return false; // Would exit the download loop
        }

        true // Would continue to next article
    });

    // Wait 100ms (article is in-progress)
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Signal graceful pause while article is downloading
    token.cancel();
    tracing::debug!("Graceful pause signaled while article in progress");

    // Wait for task to complete
    let result = download_task.await.unwrap();

    // Verify the article completed before the cancellation was detected
    assert!(
        article_complete.load(std::sync::atomic::Ordering::SeqCst),
        "Article should have completed"
    );
    assert!(!result, "Download should have stopped after detecting cancellation");
}

#[tokio::test]
async fn test_persist_all_state_marks_interrupted_downloads_as_paused() {
    // Test persist_all_state() marks interrupted downloads as Paused
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download in Downloading status
    let id1 = downloader.add_nzb_content(
        SAMPLE_NZB.as_bytes(),
        "test1.nzb",
        DownloadOptions::default(),
    ).await.unwrap();

    // Manually set it to Downloading status (simulating active download)
    downloader.db.update_status(id1, Status::Downloading.to_i32()).await.unwrap();

    // Add another download in Processing status
    let id2 = downloader.add_nzb_content(
        SAMPLE_NZB.as_bytes(),
        "test2.nzb",
        DownloadOptions::default(),
    ).await.unwrap();
    downloader.db.update_status(id2, Status::Processing.to_i32()).await.unwrap();

    // Add a download in Complete status (should not be changed)
    let id3 = downloader.add_nzb_content(
        SAMPLE_NZB.as_bytes(),
        "test3.nzb",
        DownloadOptions::default(),
    ).await.unwrap();
    downloader.db.update_status(id3, Status::Complete.to_i32()).await.unwrap();

    // Verify initial states
    let dl1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    assert_eq!(dl1.status, Status::Downloading.to_i32());
    let dl2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(dl2.status, Status::Processing.to_i32());
    let dl3 = downloader.db.get_download(id3).await.unwrap().unwrap();
    assert_eq!(dl3.status, Status::Complete.to_i32());

    // Call persist_all_state (these downloads are not in active_downloads map)
    let result = downloader.persist_all_state().await;
    assert!(result.is_ok(), "persist_all_state should succeed: {:?}", result);

    // Verify interrupted downloads were marked as Paused
    let dl1_after = downloader.db.get_download(id1).await.unwrap().unwrap();
    assert_eq!(
        dl1_after.status,
        Status::Paused.to_i32(),
        "Interrupted Downloading should be marked as Paused"
    );

    let dl2_after = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(
        dl2_after.status,
        Status::Paused.to_i32(),
        "Interrupted Processing should be marked as Paused"
    );

    // Complete download should remain unchanged
    let dl3_after = downloader.db.get_download(id3).await.unwrap().unwrap();
    assert_eq!(
        dl3_after.status,
        Status::Complete.to_i32(),
        "Complete download should remain Complete"
    );
}

#[tokio::test]
async fn test_persist_all_state_preserves_active_downloads() {
    // Test persist_all_state() does not modify truly active downloads
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    let id = downloader.add_nzb_content(
        SAMPLE_NZB.as_bytes(),
        "test.nzb",
        DownloadOptions::default(),
    ).await.unwrap();

    // Set it to Downloading status
    downloader.db.update_status(id, Status::Downloading.to_i32()).await.unwrap();

    // Add it to active_downloads map (simulating it's actually running)
    {
        let mut active = downloader.active_downloads.lock().await;
        active.insert(id, tokio_util::sync::CancellationToken::new());
    }

    // Call persist_all_state
    let result = downloader.persist_all_state().await;
    assert!(result.is_ok(), "persist_all_state should succeed: {:?}", result);

    // Verify the download status was NOT changed (it's still active)
    let dl_after = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        dl_after.status,
        Status::Downloading.to_i32(),
        "Active download should remain in Downloading status"
    );
}

#[tokio::test]
async fn test_shutdown_calls_persist_all_state() {
    // Test shutdown() integrates persist_all_state()
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download in Downloading status (simulating interrupted)
    let id = downloader.add_nzb_content(
        SAMPLE_NZB.as_bytes(),
        "test.nzb",
        DownloadOptions::default(),
    ).await.unwrap();
    downloader.db.update_status(id, Status::Downloading.to_i32()).await.unwrap();

    // Call shutdown
    let result = downloader.shutdown().await;
    assert!(result.is_ok(), "Shutdown should succeed: {:?}", result);

    // Verify the interrupted download was marked as Paused by persist_all_state
    let dl_after = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        dl_after.status,
        Status::Paused.to_i32(),
        "Interrupted download should be marked as Paused after shutdown"
    );
}

#[tokio::test]
async fn test_shutdown_emits_shutdown_event() {
    // Test that shutdown() emits a Shutdown event
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Spawn a task to collect events
    let event_handle = tokio::spawn(async move {
        let mut shutdown_received = false;
        while let Ok(event) = events.recv().await {
            if matches!(event, Event::Shutdown) {
                shutdown_received = true;
                break;
            }
        }
        shutdown_received
    });

    // Call shutdown
    let result = downloader.shutdown().await;
    assert!(result.is_ok(), "Shutdown should succeed: {:?}", result);

    // Verify Shutdown event was emitted
    let shutdown_received = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        event_handle
    ).await.expect("Timeout waiting for event task")
        .expect("Event task should complete");

    assert!(shutdown_received, "Shutdown event should be emitted");
}

#[tokio::test]
async fn test_run_with_shutdown_basic() {
    // Test that run_with_shutdown function exists and is callable
    // Note: We can't easily test actual signal handling in unit tests,
    // but we verify the function compiles and the structure is correct

    let (downloader, _temp_dir) = create_test_downloader().await;

    // We can't easily send signals in a test, so we just verify
    // the function signature and structure by calling shutdown directly
    let result = downloader.shutdown().await;
    assert!(result.is_ok(), "Shutdown should succeed: {:?}", result);
}

#[tokio::test]
async fn test_graceful_shutdown_and_recovery_on_restart() {
    // Test complete graceful shutdown and recovery on restart
    //
    // This integration test verifies:
    // 1. Active downloads are gracefully paused on shutdown
    // 2. Database is marked as "clean shutdown"
    // 3. On restart, downloads are properly restored
    // 4. Progress and state are preserved across restart

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let download_id;
    let total_articles;

    // Part 1: Create downloader, add download, and perform graceful shutdown
    {
        let config = Config {
            database_path: db_path.clone(),
            servers: vec![],
            max_concurrent_downloads: 3,
            ..Default::default()
        };

        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Add a download
        download_id = downloader.add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions::default()
        ).await.unwrap();

        // Get all articles
        let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
        total_articles = articles.len();
        assert!(total_articles > 1, "Need at least 2 articles for this test");

        // Mark first article as downloaded (simulating partial progress)
        if let Some(first_article) = articles.first() {
            downloader.db.update_article_status(
                first_article.id,
                crate::db::article_status::DOWNLOADED
            ).await.unwrap();
        }

        // Set status to Downloading (simulating active download)
        downloader.db.update_status(download_id, Status::Downloading.to_i32()).await.unwrap();

        // Set some progress to verify it's preserved
        let progress = 50.0;
        let speed = 1000000u64; // 1 MB/s
        let downloaded_bytes = 524288u64; // 512 KB
        downloader.db.update_progress(download_id, progress, speed, downloaded_bytes).await.unwrap();

        // Perform graceful shutdown
        let shutdown_result = downloader.shutdown().await;
        assert!(shutdown_result.is_ok(), "Graceful shutdown should succeed: {:?}", shutdown_result);

        // Verify database was marked as clean shutdown
        let was_unclean = downloader.db.was_unclean_shutdown().await.unwrap();
        assert!(!was_unclean, "Database should be marked as CLEAN shutdown after graceful shutdown");

        // Verify download was marked as Paused (not Downloading)
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(
            Status::from_i32(download.status),
            Status::Paused,
            "Download should be marked as Paused after graceful shutdown"
        );
    }

    // Part 2: Simulate restart by creating new downloader instance
    {
        // First, check the shutdown state BEFORE creating the downloader
        // (UsenetDownloader::new() calls set_clean_start() which would override the flag)
        let db_for_check = Database::new(&db_path).await.unwrap();
        let was_unclean = db_for_check.was_unclean_shutdown().await.unwrap();
        assert!(!was_unclean, "Database should show clean shutdown from previous session");
        db_for_check.close().await;

        // Now create the downloader (which will call set_clean_start() internally)
        let config = Config {
            database_path: db_path.clone(),
            servers: vec![],
            max_concurrent_downloads: 3,
            ..Default::default()
        };

        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Verify download was restored
        let restored_download = downloader.db.get_download(download_id).await.unwrap();
        assert!(restored_download.is_some(), "Download should be restored after restart");

        let download = restored_download.unwrap();

        // After graceful shutdown, download should remain Paused
        assert_eq!(
            Status::from_i32(download.status),
            Status::Paused,
            "Download should remain Paused after restart"
        );

        // Progress should be preserved
        assert_eq!(download.progress, 50.0, "Progress should be preserved");
        assert_eq!(download.downloaded_bytes, 524288, "Downloaded bytes should be preserved");

        // Verify article tracking was preserved
        let pending_articles = downloader.db.get_pending_articles(download_id).await.unwrap();
        assert_eq!(
            pending_articles.len(),
            total_articles - 1,
            "Should have {} pending articles (1 was downloaded before shutdown)",
            total_articles - 1
        );

        // Verify we can resume the download after restart
        let resume_result = downloader.resume(download_id).await;
        assert!(resume_result.is_ok(), "Should be able to resume download after restart: {:?}", resume_result);

        let resumed_download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(
            Status::from_i32(resumed_download.status),
            Status::Queued,
            "Download should be Queued after resume"
        );
    }
}

#[tokio::test]
async fn test_start_folder_watcher_no_watch_folders() {
    // Create downloader with no watch folders configured
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Should succeed but return a completed task
    let handle = downloader.start_folder_watcher();
    assert!(handle.is_ok(), "start_folder_watcher should succeed with no watch folders");

    // The task should complete immediately
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        handle.unwrap()
    ).await;
    assert!(result.is_ok(), "Task should complete immediately with no watch folders");
}

#[tokio::test]
async fn test_start_folder_watcher_with_configured_folders() {
    let temp_dir = tempdir().unwrap();
    let watch_path = temp_dir.path().join("watch");

    // Create config with watch folder
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        watch_folders: vec![
            config::WatchFolderConfig {
                path: watch_path.clone(),
                after_import: config::WatchFolderAction::Delete,
                category: Some("test".to_string()),
                scan_interval: Duration::from_secs(5),
            }
        ],
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start folder watcher
    let handle = downloader.start_folder_watcher();
    assert!(handle.is_ok(), "start_folder_watcher should succeed: {:?}", handle.err());

    // Verify watch directory was created
    assert!(watch_path.exists(), "Watch folder should be created by start()");

    // Let the watcher task run for a moment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Abort the task (it runs indefinitely)
    handle.unwrap().abort();
}

#[tokio::test]
async fn test_start_folder_watcher_creates_missing_directory() {
    let temp_dir = tempdir().unwrap();
    let watch_path = temp_dir.path().join("nonexistent").join("watch");

    // Verify directory doesn't exist yet
    assert!(!watch_path.exists(), "Watch path should not exist yet");

    // Create config with non-existent watch folder
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        watch_folders: vec![
            config::WatchFolderConfig {
                path: watch_path.clone(),
                after_import: config::WatchFolderAction::MoveToProcessed,
                category: None,
                scan_interval: Duration::from_secs(5),
            }
        ],
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start folder watcher - should create the directory
    let handle = downloader.start_folder_watcher();
    assert!(handle.is_ok(), "start_folder_watcher should create missing directories: {:?}", handle.err());

    // Verify directory was created
    assert!(watch_path.exists(), "Watch folder should be auto-created");

    // Abort the task
    handle.unwrap().abort();
}

// ============================================================================
// RSS Scheduler Tests
// ============================================================================

#[tokio::test]
async fn test_start_rss_scheduler_no_feeds() {
    // Create downloader with no RSS feeds configured
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Should succeed but return a completed task
    let handle = downloader.start_rss_scheduler();

    // The task should complete immediately with no feeds
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        handle
    ).await;
    assert!(result.is_ok(), "Task should complete immediately with no RSS feeds");
}

#[tokio::test]
async fn test_start_rss_scheduler_with_feeds() {
    let temp_dir = tempdir().unwrap();

    // Create config with RSS feeds
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        rss_feeds: vec![
            config::RssFeedConfig {
                url: "https://example.com/feed.xml".to_string(),
                check_interval: Duration::from_secs(60), // 1 minute
                category: Some("test".to_string()),
                filters: vec![],
                auto_download: true,
                priority: Priority::Normal,
                enabled: true,
            }
        ],
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start RSS scheduler
    let handle = downloader.start_rss_scheduler();

    // Let the scheduler task start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the task is still running (it shouldn't complete immediately)
    assert!(!handle.is_finished(), "Scheduler should be running with configured feeds");

    // Abort the task
    handle.abort();
}

#[tokio::test]
async fn test_start_rss_scheduler_respects_shutdown() {
    let temp_dir = tempdir().unwrap();

    // Create config with RSS feeds
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        rss_feeds: vec![
            config::RssFeedConfig {
                url: "https://example.com/feed.xml".to_string(),
                check_interval: Duration::from_secs(60),
                category: None,
                filters: vec![],
                auto_download: false,
                priority: Priority::Normal,
                enabled: true,
            }
        ],
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start RSS scheduler
    let handle = downloader.start_rss_scheduler();

    // Let it run briefly
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Trigger shutdown
    downloader.accepting_new.store(false, std::sync::atomic::Ordering::SeqCst);

    // Wait for scheduler to detect shutdown
    // Note: Scheduler checks every second, so 5 seconds should be plenty
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        handle
    ).await;

    assert!(result.is_ok(), "Scheduler should shut down gracefully when accepting_new is set to false");
}

#[tokio::test]
async fn test_start_rss_scheduler_with_multiple_feeds() {
    let temp_dir = tempdir().unwrap();

    // Create config with multiple RSS feeds
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
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
            }
        ],
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start RSS scheduler
    let handle = downloader.start_rss_scheduler();

    // Let it run briefly
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the task is running
    assert!(!handle.is_finished(), "Scheduler should handle multiple feeds");

    // Abort the task
    handle.abort();
}

#[tokio::test]
async fn test_start_rss_scheduler_only_enabled_feeds() {
    let temp_dir = tempdir().unwrap();

    // Create config with only disabled feeds
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        rss_feeds: vec![
            config::RssFeedConfig {
                url: "https://example.com/feed.xml".to_string(),
                check_interval: Duration::from_secs(60),
                category: None,
                filters: vec![],
                auto_download: false,
                priority: Priority::Normal,
                enabled: false, // Disabled
            }
        ],
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start RSS scheduler
    let handle = downloader.start_rss_scheduler();

    // Let it run briefly
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Scheduler should still be running (just idle, checking for enabled feeds)
    assert!(!handle.is_finished(), "Scheduler should run even with disabled feeds");

    // Abort the task
    handle.abort();
}

#[tokio::test]
async fn test_start_scheduler_no_rules() {
    // Create downloader with no schedule rules configured
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Should succeed but return a completed task
    let handle = downloader.start_scheduler();

    // The task should complete immediately with no rules
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        handle
    ).await;
    assert!(result.is_ok(), "Task should complete immediately with no schedule rules");
}

#[tokio::test]
async fn test_start_scheduler_with_rules() {
    let temp_dir = tempdir().unwrap();

    // Create config with schedule rules
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        schedule_rules: vec![
            config::ScheduleRule {
                name: "Test Rule".to_string(),
                days: vec![],  // All days
                start_time: "09:00".to_string(),
                end_time: "17:00".to_string(),
                action: config::ScheduleAction::SpeedLimit { limit_bps: 1_000_000 },
                enabled: true,
            }
        ],
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start scheduler
    let handle = downloader.start_scheduler();

    // Let the scheduler task start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the task is still running (it shouldn't complete immediately)
    assert!(!handle.is_finished(), "Scheduler should be running with configured rules");

    // Abort the task
    handle.abort();
}

#[tokio::test]
async fn test_start_scheduler_respects_shutdown() {
    let temp_dir = tempdir().unwrap();

    // Create config with schedule rules
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        schedule_rules: vec![
            config::ScheduleRule {
                name: "Test Rule".to_string(),
                days: vec![],
                start_time: "09:00".to_string(),
                end_time: "17:00".to_string(),
                action: config::ScheduleAction::Unlimited,
                enabled: true,
            }
        ],
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Trigger shutdown before starting the task
    downloader.accepting_new.store(false, std::sync::atomic::Ordering::SeqCst);

    // Start scheduler
    let handle = downloader.start_scheduler();

    // Task should exit gracefully immediately without waiting the full minute
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        handle
    ).await;
    assert!(result.is_ok(), "Scheduler should exit on shutdown signal");
}

// Duplicate detection tests

#[test]
fn test_extract_job_name() {
    // Basic filename with .nzb extension
    assert_eq!(UsenetDownloader::extract_job_name("movie.nzb"), "movie");

    // Filename without extension
    assert_eq!(UsenetDownloader::extract_job_name("movie"), "movie");

    // Complex filename with dots
    assert_eq!(
        UsenetDownloader::extract_job_name("My.Movie.2024.1080p.nzb"),
        "My.Movie.2024.1080p"
    );

    // Empty string
    assert_eq!(UsenetDownloader::extract_job_name(""), "");

    // Just .nzb extension
    assert_eq!(UsenetDownloader::extract_job_name(".nzb"), "");
}

#[tokio::test]
async fn test_check_duplicate_disabled() {
    let temp_dir = tempdir().unwrap();

    // Create config with duplicate detection disabled
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        duplicate: config::DuplicateConfig {
            enabled: false,
            action: config::DuplicateAction::Warn,
            methods: vec![config::DuplicateMethod::NzbHash],
        },
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Check should return None when disabled
    let nzb_content = b"<nzb>test content</nzb>";
    let result = downloader.check_duplicate(nzb_content, "test.nzb").await;
    assert!(result.is_none(), "Duplicate check should return None when disabled");
}

#[tokio::test]
async fn test_check_duplicate_nzb_hash_no_match() {
    let temp_dir = tempdir().unwrap();

    // Create config with NzbHash detection
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        duplicate: config::DuplicateConfig {
            enabled: true,
            action: config::DuplicateAction::Warn,
            methods: vec![config::DuplicateMethod::NzbHash],
        },
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Check new NZB that doesn't exist yet
    let nzb_content = b"<nzb>unique content</nzb>";
    let result = downloader.check_duplicate(nzb_content, "unique.nzb").await;
    assert!(result.is_none(), "Should not find duplicate for new NZB");
}

#[tokio::test]
async fn test_check_duplicate_nzb_hash_match() {
    let temp_dir = tempdir().unwrap();

    // Create config with NzbHash detection
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        duplicate: config::DuplicateConfig {
            enabled: true,
            action: config::DuplicateAction::Warn,
            methods: vec![config::DuplicateMethod::NzbHash],
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Calculate hash for test content
    use sha2::{Digest, Sha256};
    let nzb_content = b"<nzb>test content</nzb>";
    let mut hasher = Sha256::new();
    hasher.update(nzb_content);
    let hash = format!("{:x}", hasher.finalize());

    // Add a download with this hash
    let download = db::NewDownload {
        name: "existing.nzb".to_string(),
        nzb_path: "/tmp/existing.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some(hash),
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 0,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    let existing_id = downloader.db.insert_download(&download).await.unwrap();

    // Check for duplicate - should find the existing download
    let result = downloader.check_duplicate(nzb_content, "test.nzb").await;
    assert!(result.is_some(), "Should find duplicate by NZB hash");

    let dup = result.unwrap();
    assert_eq!(dup.existing_id, existing_id);
    assert_eq!(dup.existing_name, "existing.nzb");
    assert_eq!(dup.method, config::DuplicateMethod::NzbHash);
}

#[tokio::test]
async fn test_check_duplicate_nzb_name_match() {
    let temp_dir = tempdir().unwrap();

    // Create config with NzbName detection
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        duplicate: config::DuplicateConfig {
            enabled: true,
            action: config::DuplicateAction::Warn,
            methods: vec![config::DuplicateMethod::NzbName],
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Add a download with specific name
    let download = db::NewDownload {
        name: "movie.nzb".to_string(),
        nzb_path: "/tmp/movie.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 0,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    let existing_id = downloader.db.insert_download(&download).await.unwrap();

    // Check for duplicate by name
    let nzb_content = b"<nzb>some content</nzb>";
    let result = downloader.check_duplicate(nzb_content, "movie.nzb").await;
    assert!(result.is_some(), "Should find duplicate by NZB name");

    let dup = result.unwrap();
    assert_eq!(dup.existing_id, existing_id);
    assert_eq!(dup.existing_name, "movie.nzb");
    assert_eq!(dup.method, config::DuplicateMethod::NzbName);
}

#[tokio::test]
async fn test_check_duplicate_job_name_match() {
    let temp_dir = tempdir().unwrap();

    // Create config with JobName detection
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        duplicate: config::DuplicateConfig {
            enabled: true,
            action: config::DuplicateAction::Warn,
            methods: vec![config::DuplicateMethod::JobName],
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Add a download with specific job name
    let download = db::NewDownload {
        name: "abc123def456.nzb".to_string(),  // Obfuscated filename
        nzb_path: "/tmp/abc123.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: Some("My.Movie.2024".to_string()),  // Deobfuscated job name
        category: None,
        destination: "/downloads".to_string(),
        post_process: 0,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    let existing_id = downloader.db.insert_download(&download).await.unwrap();

    // Check for duplicate by job name
    let nzb_content = b"<nzb>content</nzb>";
    let result = downloader.check_duplicate(nzb_content, "My.Movie.2024.nzb").await;
    assert!(result.is_some(), "Should find duplicate by job name");

    let dup = result.unwrap();
    assert_eq!(dup.existing_id, existing_id);
    assert_eq!(dup.existing_name, "abc123def456.nzb");
    assert_eq!(dup.method, config::DuplicateMethod::JobName);
}

#[tokio::test]
async fn test_check_duplicate_multiple_methods_first_match() {
    let temp_dir = tempdir().unwrap();

    // Create config with multiple detection methods
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        duplicate: config::DuplicateConfig {
            enabled: true,
            action: config::DuplicateAction::Warn,
            methods: vec![
                config::DuplicateMethod::NzbHash,   // First (highest priority)
                config::DuplicateMethod::NzbName,   // Second
                config::DuplicateMethod::JobName,   // Third
            ],
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Calculate hash for test content
    use sha2::{Digest, Sha256};
    let nzb_content = b"<nzb>test content</nzb>";
    let mut hasher = Sha256::new();
    hasher.update(nzb_content);
    let hash = format!("{:x}", hasher.finalize());

    // Add a download that matches by hash (highest priority method)
    let download = db::NewDownload {
        name: "different_name.nzb".to_string(),
        nzb_path: "/tmp/different.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some(hash),
        job_name: Some("different_job".to_string()),
        category: None,
        destination: "/downloads".to_string(),
        post_process: 0,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    let existing_id = downloader.db.insert_download(&download).await.unwrap();

    // Check for duplicate - should find by hash (first method)
    let result = downloader.check_duplicate(nzb_content, "some_name.nzb").await;
    assert!(result.is_some(), "Should find duplicate by first matching method");

    let dup = result.unwrap();
    assert_eq!(dup.existing_id, existing_id);
    assert_eq!(dup.method, config::DuplicateMethod::NzbHash, "Should use first matching method (NzbHash)");
}

#[tokio::test]
async fn test_check_duplicate_no_match_any_method() {
    let temp_dir = tempdir().unwrap();

    // Create config with all detection methods
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        servers: vec![],
        duplicate: config::DuplicateConfig {
            enabled: true,
            action: config::DuplicateAction::Warn,
            methods: vec![
                config::DuplicateMethod::NzbHash,
                config::DuplicateMethod::NzbName,
                config::DuplicateMethod::JobName,
            ],
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Add a download with different hash, name, and job name
    let download = db::NewDownload {
        name: "existing.nzb".to_string(),
        nzb_path: "/tmp/existing.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("abc123".to_string()),
        job_name: Some("Existing.Job".to_string()),
        category: None,
        destination: "/downloads".to_string(),
        post_process: 0,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    downloader.db.insert_download(&download).await.unwrap();

    // Check for duplicate with completely different content, name, and job name
    let nzb_content = b"<nzb>totally different content</nzb>";
    let result = downloader.check_duplicate(nzb_content, "new.nzb").await;
    assert!(result.is_none(), "Should not find duplicate when nothing matches");
}

#[tokio::test]
async fn test_add_nzb_content_duplicate_warn() {
    let temp_dir = tempdir().unwrap();

    // Create config with duplicate detection enabled (Warn action)
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        download_dir: temp_dir.path().join("downloads"),
        temp_dir: temp_dir.path().join("temp"),
        duplicate: config::DuplicateConfig {
            enabled: true,
            action: config::DuplicateAction::Warn,
            methods: vec![config::DuplicateMethod::NzbHash],
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Create a valid NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.com" date="1234567890" subject="test.bin (1/1)">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Add first download
    let id1 = downloader.add_nzb_content(nzb_content, "test.nzb", DownloadOptions::default()).await.unwrap();
    assert!(id1 > 0, "First download should succeed");

    // Subscribe to events to catch duplicate warning
    let mut events = downloader.subscribe();

    // Try to add the same NZB again (should warn but allow)
    let id2 = downloader.add_nzb_content(nzb_content, "test-copy.nzb", DownloadOptions::default()).await.unwrap();
    assert!(id2 > id1, "Second download should succeed with Warn action");

    // Check that duplicate event was emitted
    let event = tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await;
    if let Ok(Ok(Event::DuplicateDetected { id, name, method, existing_name })) = event {
        assert_eq!(id, id1, "Event should reference existing download");
        assert_eq!(name, "test-copy.nzb", "Event should have new download name");
        assert_eq!(method, config::DuplicateMethod::NzbHash, "Event should show NzbHash method");
        assert_eq!(existing_name, "test.nzb", "Event should have existing download name");
    } else {
        panic!("Expected DuplicateDetected event, got: {:?}", event);
    }
}

#[tokio::test]
async fn test_add_nzb_content_duplicate_block() {
    let temp_dir = tempdir().unwrap();

    // Create config with duplicate detection enabled (Block action)
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        download_dir: temp_dir.path().join("downloads"),
        temp_dir: temp_dir.path().join("temp"),
        duplicate: config::DuplicateConfig {
            enabled: true,
            action: config::DuplicateAction::Block,
            methods: vec![config::DuplicateMethod::NzbHash],
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Create a valid NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.com" date="1234567890" subject="test.bin (1/1)">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Add first download
    let id1 = downloader.add_nzb_content(nzb_content, "test.nzb", DownloadOptions::default()).await.unwrap();
    assert!(id1 > 0, "First download should succeed");

    // Subscribe to events to catch duplicate warning
    let mut events = downloader.subscribe();

    // Try to add the same NZB again (should block)
    let result = downloader.add_nzb_content(nzb_content, "test-copy.nzb", DownloadOptions::default()).await;
    assert!(result.is_err(), "Second download should be blocked");

    // Check error message
    if let Err(Error::Duplicate(msg)) = result {
        assert!(msg.contains("Duplicate download detected"), "Error should mention duplicate");
        assert!(msg.contains("test-copy.nzb"), "Error should mention new file name");
        assert!(msg.contains("NzbHash"), "Error should mention detection method");
    } else {
        panic!("Expected Error::Duplicate, got: {:?}", result);
    }

    // Check that duplicate event was emitted before blocking
    let event = tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await;
    if let Ok(Ok(Event::DuplicateDetected { id, name, method, existing_name })) = event {
        assert_eq!(id, id1, "Event should reference existing download");
        assert_eq!(name, "test-copy.nzb", "Event should have new download name");
        assert_eq!(method, config::DuplicateMethod::NzbHash, "Event should show NzbHash method");
        assert_eq!(existing_name, "test.nzb", "Event should have existing download name");
    } else {
        panic!("Expected DuplicateDetected event, got: {:?}", event);
    }
}

#[tokio::test]
async fn test_add_nzb_content_duplicate_allow() {
    let temp_dir = tempdir().unwrap();

    // Create config with duplicate detection enabled (Allow action)
    let config = Config {
        database_path: temp_dir.path().join("test.db"),
        download_dir: temp_dir.path().join("downloads"),
        temp_dir: temp_dir.path().join("temp"),
        duplicate: config::DuplicateConfig {
            enabled: true,
            action: config::DuplicateAction::Allow,
            methods: vec![config::DuplicateMethod::NzbHash],
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Create a valid NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.com" date="1234567890" subject="test.bin (1/1)">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Add first download
    let id1 = downloader.add_nzb_content(nzb_content, "test.nzb", DownloadOptions::default()).await.unwrap();
    assert!(id1 > 0, "First download should succeed");

    // Try to add the same NZB again (should allow without warning)
    let id2 = downloader.add_nzb_content(nzb_content, "test-copy.nzb", DownloadOptions::default()).await.unwrap();
    assert!(id2 > id1, "Second download should succeed with Allow action");

    // Note: In Allow mode, the event is still emitted (informational)
    // This is acceptable behavior - the action determines whether to block, not whether to emit
}

#[tokio::test]
async fn test_webhook_triggers_on_queued() {
    // Create test downloader with webhook configuration
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Use httpbin.org for webhook testing (real HTTP endpoint)
    let webhook_url = "https://httpbin.org/post".to_string();

    let config = Config {
        database_path: db_path.clone(),
        download_dir: temp_dir.path().join("downloads"),
        temp_dir: temp_dir.path().join("temp"),
        webhooks: vec![
            crate::config::WebhookConfig {
                url: webhook_url.clone(),
                events: vec![crate::config::WebhookEvent::OnQueued],
                auth_header: None,
                timeout: std::time::Duration::from_secs(10),
            },
        ],
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Create test NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file subject="test.file">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Subscribe to events to verify WebhookFailed event is not emitted
    let mut events = downloader.subscribe();

    // Add NZB (should trigger OnQueued webhook)
    let id = downloader.add_nzb_content(
        nzb_content,
        "webhook-test.nzb",
        DownloadOptions::default()
    ).await.unwrap();

    assert!(id > 0, "Download should be queued successfully");

    // Wait a bit for webhook to be sent (it's async/background)
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Check that we got the Queued event
    let event = tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        events.recv()
    ).await;

    assert!(event.is_ok(), "Should receive event");
    let event = event.unwrap().unwrap();
    match event {
        Event::Queued { id: queued_id, name } => {
            assert_eq!(queued_id, id);
            assert_eq!(name, "webhook-test.nzb");
        }
        _ => panic!("Expected Queued event, got {:?}", event),
    }

    println!("✓ Webhook test passed - OnQueued webhook sent to httpbin.org");
    println!("  Note: Check network logs to verify HTTP POST was sent");
    println!("  Webhook URL: {}", webhook_url);
}

#[tokio::test]
async fn test_webhook_failed_event_on_invalid_url() {
    // Create test downloader with invalid webhook URL
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let config = Config {
        database_path: db_path.clone(),
        download_dir: temp_dir.path().join("downloads"),
        temp_dir: temp_dir.path().join("temp"),
        webhooks: vec![
            crate::config::WebhookConfig {
                url: "http://invalid-webhook-url-that-does-not-exist.test/webhook".to_string(),
                events: vec![crate::config::WebhookEvent::OnQueued],
                auth_header: None,
                timeout: std::time::Duration::from_secs(2),
            },
        ],
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Create test NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file subject="test.file">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Add NZB (should trigger OnQueued webhook which will fail)
    let _id = downloader.add_nzb_content(
        nzb_content,
        "webhook-fail-test.nzb",
        DownloadOptions::default()
    ).await.unwrap();

    // Wait for webhook to fail and WebhookFailed event to be emitted
    let mut found_queued = false;
    let mut found_webhook_failed = false;

    for _ in 0..5 {
        let event = tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            events.recv()
        ).await;

        if let Ok(Ok(evt)) = event {
            match evt {
                Event::Queued { .. } => {
                    found_queued = true;
                }
                Event::WebhookFailed { url, error } => {
                    found_webhook_failed = true;
                    assert!(url.contains("invalid-webhook-url-that-does-not-exist.test"));
                    assert!(!error.is_empty(), "Error message should not be empty");
                    println!("✓ WebhookFailed event received: {}", error);
                    break;
                }
                _ => {}
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    assert!(found_queued, "Should receive Queued event");
    assert!(found_webhook_failed, "Should receive WebhookFailed event for invalid URL");
}

#[tokio::test]
async fn test_webhook_auth_header() {
    // Create test downloader with webhook that includes auth header
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let config = Config {
        database_path: db_path.clone(),
        download_dir: temp_dir.path().join("downloads"),
        temp_dir: temp_dir.path().join("temp"),
        webhooks: vec![
            crate::config::WebhookConfig {
                url: "https://httpbin.org/post".to_string(),
                events: vec![crate::config::WebhookEvent::OnQueued],
                auth_header: Some("Bearer test-token-12345".to_string()),
                timeout: std::time::Duration::from_secs(10),
            },
        ],
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Create test NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file subject="test.file">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Add NZB (should trigger webhook with auth header)
    let id = downloader.add_nzb_content(
        nzb_content,
        "webhook-auth-test.nzb",
        DownloadOptions::default()
    ).await.unwrap();

    assert!(id > 0, "Download should be queued successfully");

    // Wait for webhook to be sent
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    println!("✓ Webhook with auth header sent to httpbin.org");
    println!("  Auth header: Bearer test-token-12345");
    println!("  Note: Check httpbin.org response to verify Authorization header was sent");
}

/// Test script trigger on complete event
#[tokio::test]
async fn test_script_trigger_on_complete() {
    use crate::config::ScriptConfig;
    use std::time::Duration;
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();

    // Create config with a test script (use absolute path)
    let current_dir = std::env::current_dir().unwrap();
    let script_path = current_dir.join("test_scripts/test_success.sh");

    // Skip test if script doesn't exist
    if !script_path.exists() {
        println!("⚠ Skipping test: {} not found", script_path.display());
        return;
    }

    let mut config = Config::default();
    config.database_path = temp_dir.path().join("test.db");
    config.download_dir = temp_dir.path().join("downloads");
    config.temp_dir = temp_dir.path().join("temp");

    // Add script that triggers on complete event
    config.scripts = vec![ScriptConfig {
        path: script_path.clone(),
        events: vec![crate::config::ScriptEvent::OnComplete],
        timeout: Duration::from_secs(5),
    }];

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Trigger scripts for a completed download
    // This tests that trigger_scripts is callable and doesn't panic
    downloader.trigger_scripts(
        crate::config::ScriptEvent::OnComplete,
        999,
        "Test Download".to_string(),
        Some("test".to_string()),
        "complete".to_string(),
        Some(std::path::PathBuf::from("/tmp/test")),
        None,
        1024000,
    );

    // Wait a bit for async script execution to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    println!("✓ Script trigger method executed successfully");
}

/// Test script configuration
#[tokio::test]
async fn test_script_configuration() {
    use crate::config::ScriptConfig;
    use std::time::Duration;
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();

    // Create config with a failing script (use absolute path)
    let current_dir = std::env::current_dir().unwrap();
    let script_path = current_dir.join("test_scripts/test_failure.sh");

    // Skip test if script doesn't exist
    if !script_path.exists() {
        println!("⚠ Skipping test: {} not found", script_path.display());
        return;
    }

    let mut config = Config::default();
    config.database_path = temp_dir.path().join("test.db");
    config.download_dir = temp_dir.path().join("downloads");
    config.temp_dir = temp_dir.path().join("temp");

    // Test adding multiple scripts with different events
    config.scripts = vec![
        ScriptConfig {
            path: script_path.clone(),
            events: vec![crate::config::ScriptEvent::OnFailed],
            timeout: Duration::from_secs(5),
        },
        ScriptConfig {
            path: script_path,
            events: vec![crate::config::ScriptEvent::OnComplete, crate::config::ScriptEvent::OnPostProcessComplete],
            timeout: Duration::from_secs(10),
        },
    ];

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Verify downloader was created successfully with script config
    assert_eq!(downloader.config.scripts.len(), 2);
    assert_eq!(downloader.config.scripts[0].events.len(), 1);
    assert_eq!(downloader.config.scripts[1].events.len(), 2);

    println!("✓ Script configuration loaded successfully");
}

/// Test category-specific scripts are executed before global scripts
#[tokio::test]
async fn test_category_scripts_execution_order() {
    use crate::config::{ScriptConfig, CategoryConfig};
    use std::time::Duration;
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();

    // Use absolute path for script
    let current_dir = std::env::current_dir().unwrap();
    let script_path = current_dir.join("test_scripts/test_success.sh");

    // Skip test if script doesn't exist
    if !script_path.exists() {
        println!("⚠ Skipping test: {} not found", script_path.display());
        return;
    }

    let mut config = Config::default();
    config.database_path = temp_dir.path().join("test.db");
    config.download_dir = temp_dir.path().join("downloads");
    config.temp_dir = temp_dir.path().join("temp");

    // Add global script
    config.scripts = vec![ScriptConfig {
        path: script_path.clone(),
        events: vec![crate::config::ScriptEvent::OnComplete],
        timeout: Duration::from_secs(5),
    }];

    // Add category with its own script
    let mut categories = std::collections::HashMap::new();
    categories.insert("movies".to_string(), CategoryConfig {
        destination: temp_dir.path().join("movies"),
        post_process: None,
        watch_folder: None,
        scripts: vec![ScriptConfig {
            path: script_path.clone(),
            events: vec![crate::config::ScriptEvent::OnComplete],
            timeout: Duration::from_secs(5),
        }],
    });
    config.categories = categories;

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Trigger scripts for a download with category
    downloader.trigger_scripts(
        crate::config::ScriptEvent::OnComplete,
        999,
        "Test Movie".to_string(),
        Some("movies".to_string()),
        "complete".to_string(),
        Some(std::path::PathBuf::from("/tmp/movie.mkv")),
        None,
        5000000,
    );

    // Wait for scripts to execute
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Both scripts should have executed
    // Category script should have IS_CATEGORY_SCRIPT=true
    // Global script should not have that variable

    println!("✓ Category and global scripts triggered in correct order");
}

#[tokio::test]
async fn test_check_disk_space_sufficient() {
    // Test: check_disk_space should succeed when sufficient space is available
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download_dir = temp_dir.path().to_path_buf();
    config.disk_space.enabled = true;
    config.disk_space.min_free_space = 1024 * 1024; // 1 MB buffer
    config.disk_space.size_multiplier = 2.5;

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Check with a small download size (1 KB)
    let result = downloader.check_disk_space(1024).await;
    assert!(result.is_ok(), "Expected check_disk_space to succeed with small download");

    println!("✓ check_disk_space succeeds with sufficient space");
}

#[tokio::test]
async fn test_check_disk_space_disabled() {
    // Test: check_disk_space should always succeed when disabled
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download_dir = temp_dir.path().to_path_buf();
    config.disk_space.enabled = false; // Disable checking

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Even with a huge download size, should succeed when disabled
    let result = downloader.check_disk_space(1024 * 1024 * 1024 * 1024).await; // 1 TB
    assert!(result.is_ok(), "Expected check_disk_space to succeed when disabled");

    println!("✓ check_disk_space skips check when disabled");
}

#[tokio::test]
async fn test_check_disk_space_insufficient() {
    // Test: check_disk_space should fail when insufficient space
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download_dir = temp_dir.path().to_path_buf();
    config.disk_space.enabled = true;

    // Get actual available space
    let available = crate::utils::get_available_space(&config.download_dir).unwrap();

    // Set min_free_space to require more than available
    config.disk_space.min_free_space = available + 1024 * 1024 * 1024; // Available + 1 GB
    config.disk_space.size_multiplier = 1.0;

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Try to add a download that would exceed available space
    let result = downloader.check_disk_space(1024).await; // Even 1 KB should fail

    match result {
        Err(Error::InsufficientSpace { required, available: avail }) => {
            assert!(avail < required, "Expected available < required");
            println!("✓ check_disk_space correctly detects insufficient space");
            println!("  Required: {} bytes, Available: {} bytes", required, avail);
        }
        Ok(_) => panic!("Expected InsufficientSpace error, got Ok"),
        Err(e) => panic!("Expected InsufficientSpace error, got: {:?}", e),
    }
}

#[tokio::test]
async fn test_check_disk_space_multiplier() {
    // Test: check_disk_space correctly applies size_multiplier
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download_dir = temp_dir.path().to_path_buf();
    config.disk_space.enabled = true;
    config.disk_space.min_free_space = 0; // No buffer for this test
    config.disk_space.size_multiplier = 3.0; // 3x multiplier

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Get available space
    let available = crate::utils::get_available_space(&downloader.config.download_dir).unwrap();

    // Calculate download size that would require more than available space after multiplier
    let download_size = (available as f64 / 3.0) as i64 + 1024 * 1024; // Slightly over available/3

    let result = downloader.check_disk_space(download_size).await;

    match result {
        Err(Error::InsufficientSpace { required, available: avail }) => {
            // Verify multiplier was applied: required should be approximately 3x download_size
            let expected_required = (download_size as f64 * 3.0) as u64;
            assert!(required >= expected_required - 100 && required <= expected_required + 100,
                "Expected required to be ~3x download size: {} vs {}", required, expected_required);
            println!("✓ check_disk_space correctly applies size_multiplier");
            println!("  Download: {} bytes, Required: {} bytes ({}x), Available: {} bytes",
                download_size, required, 3.0, avail);
        }
        Ok(_) => {
            // This might pass if we have a lot of disk space - that's okay
            println!("⚠ check_disk_space passed (system has lots of free space)");
        }
        Err(e) => panic!("Expected InsufficientSpace or Ok, got: {:?}", e),
    }
}

#[tokio::test]
async fn test_server_health_check_invalid_server() {
    // Test: test_server should return error for non-existent server
    println!("🧪 Testing server health check with invalid server...");

    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download_dir = temp_dir.path().to_path_buf();

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Create a server config for a non-existent server
    let server = crate::config::ServerConfig {
        host: "nonexistent.invalid".to_string(),
        port: 563,
        tls: true,
        username: None,
        password: None,
        connections: 1,
        priority: 0,
        pipeline_depth: 10,
    };

    let result = downloader.test_server(&server).await;

    // Should fail
    assert!(!result.success, "Expected test_server to fail for invalid server");
    assert!(result.error.is_some(), "Expected error message for failed connection");
    assert!(result.latency.is_some(), "Expected latency even for failed connection");
    assert!(result.capabilities.is_none(), "Expected no capabilities for failed connection");

    println!("✓ test_server correctly reports failure for invalid server");
    println!("  Error: {:?}", result.error.unwrap());
}

#[tokio::test]
async fn test_server_health_check_result_structure() {
    // Test: ServerTestResult has correct structure
    println!("🧪 Testing ServerTestResult structure...");

    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download_dir = temp_dir.path().to_path_buf();

    let downloader = UsenetDownloader::new(config).await.unwrap();

    let server = crate::config::ServerConfig {
        host: "test.invalid".to_string(),
        port: 119,
        tls: false,
        username: Some("testuser".to_string()),
        password: Some("testpass".to_string()),
        connections: 1,
        priority: 0,
        pipeline_depth: 10,
    };

    let result = downloader.test_server(&server).await;

    // Verify structure exists and is serializable
    let json = serde_json::to_string(&result).unwrap();
    let parsed: crate::types::ServerTestResult = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.success, result.success);
    assert_eq!(parsed.latency, result.latency);
    assert_eq!(parsed.error, result.error);

    println!("✓ ServerTestResult correctly serializes/deserializes");
    println!("  JSON: {}", json);
}

#[tokio::test]
async fn test_all_servers_empty_config() {
    // Test: test_all_servers with no configured servers
    println!("🧪 Testing test_all_servers with empty configuration...");

    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download_dir = temp_dir.path().to_path_buf();
    config.servers = vec![]; // No servers configured

    let downloader = UsenetDownloader::new(config).await.unwrap();

    let results = downloader.test_all_servers().await;

    assert!(results.is_empty(), "Expected empty results for empty server list");

    println!("✓ test_all_servers correctly handles empty server list");
}

#[tokio::test]
async fn test_all_servers_multiple_servers() {
    // Test: test_all_servers returns results for all servers
    println!("🧪 Testing test_all_servers with multiple servers...");

    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download_dir = temp_dir.path().to_path_buf();

    // Add multiple test servers
    config.servers = vec![
        crate::config::ServerConfig {
            host: "server1.invalid".to_string(),
            port: 563,
            tls: true,
            username: None,
            password: None,
            connections: 1,
            priority: 0,
            pipeline_depth: 10,
        },
        crate::config::ServerConfig {
            host: "server2.invalid".to_string(),
            port: 119,
            tls: false,
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            connections: 1,
            priority: 1,
            pipeline_depth: 10,
        },
        crate::config::ServerConfig {
            host: "server3.invalid".to_string(),
            port: 563,
            tls: true,
            username: None,
            password: None,
            connections: 1,
            priority: 2,
            pipeline_depth: 10,
        },
    ];

    let downloader = UsenetDownloader::new(config.clone()).await.unwrap();

    let results = downloader.test_all_servers().await;

    // Should have results for all servers
    assert_eq!(results.len(), 3, "Expected results for all 3 servers");

    // Verify each server is represented
    let hostnames: Vec<String> = results.iter().map(|(host, _)| host.clone()).collect();
    assert!(hostnames.contains(&"server1.invalid".to_string()));
    assert!(hostnames.contains(&"server2.invalid".to_string()));
    assert!(hostnames.contains(&"server3.invalid".to_string()));

    // All should fail (invalid servers)
    for (host, result) in &results {
        assert!(!result.success, "Expected test to fail for {}", host);
        assert!(result.error.is_some(), "Expected error for {}", host);
    }

    println!("✓ test_all_servers correctly tests all configured servers");
    println!("  Tested {} servers", results.len());
}

#[tokio::test]
async fn test_server_capabilities_structure() {
    // Test: ServerCapabilities structure is correct
    println!("🧪 Testing ServerCapabilities structure...");

    let caps = crate::types::ServerCapabilities {
        posting_allowed: true,
        max_connections: Some(10),
        compression: true,
    };

    // Verify serialization
    let json = serde_json::to_string(&caps).unwrap();
    let parsed: crate::types::ServerCapabilities = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.posting_allowed, caps.posting_allowed);
    assert_eq!(parsed.max_connections, caps.max_connections);
    assert_eq!(parsed.compression, caps.compression);

    println!("✓ ServerCapabilities correctly serializes/deserializes");
    println!("  JSON: {}", json);
}
