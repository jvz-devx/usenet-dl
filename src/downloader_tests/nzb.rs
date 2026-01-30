use super::*;

#[tokio::test]
async fn test_add_nzb_content_basic() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add NZB to queue
    let download_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test_download",
            DownloadOptions::default(),
        )
        .await
        .unwrap();

    assert!(download_id.0 > 0);

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

    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();

    // Check NZB metadata was extracted
    assert_eq!(download.nzb_meta_name, Some("Test Download".to_string()));
    assert_eq!(download.job_name, Some("Test Download".to_string())); // Uses meta title

    // Check password was cached
    let cached_password = downloader
        .db
        .get_cached_password(download_id)
        .await
        .unwrap();
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
    let articles = downloader
        .db
        .get_pending_articles(download_id)
        .await
        .unwrap();
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

    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();

    // Check options were applied
    assert_eq!(download.category, Some("test_category".to_string()));
    assert_eq!(download.priority, Priority::High as i32);

    // Check provided password overrides NZB password
    let cached_password = downloader
        .db
        .get_cached_password(download_id)
        .await
        .unwrap();
    assert_eq!(cached_password, Some("override_password".to_string()));
}

#[tokio::test]
async fn test_add_nzb_content_calculates_hash() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();

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
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();

    match event {
        Event::Queued { id, name } => {
            assert!(id.0 > 0);
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

    assert!(download_id.0 > 0);

    // Verify download was created with correct name (filename without extension)
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
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

    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
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

    let download_id = downloader.add_nzb(&nzb_path, options).await.unwrap();

    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(download.category, Some("movies".to_string()));
    assert_eq!(download.priority, Priority::High as i32);
}

// URL Fetching Tests

#[tokio::test]
async fn test_add_nzb_url_success() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start mock HTTP server
    let mock_server = MockServer::start().await;

    // Mock successful NZB download with Content-Disposition header
    Mock::given(method("GET"))
        .and(path("/test.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(
                    "Content-Disposition",
                    "attachment; filename=\"Movie.Release.nzb\"",
                )
                .set_body_bytes(SAMPLE_NZB),
        )
        .mount(&mock_server)
        .await;

    // Fetch NZB from mock server
    let url = format!("{}/test.nzb", mock_server.uri());
    let download_id = downloader
        .add_nzb_url(&url, DownloadOptions::default())
        .await
        .unwrap();

    assert!(download_id.0 > 0);

    // Verify download was created with filename from Content-Disposition
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(download.name, "Movie.Release");
    assert_eq!(download.status, Status::Queued.to_i32());
}

#[tokio::test]
async fn test_add_nzb_url_extracts_filename_from_url() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start mock HTTP server
    let mock_server = MockServer::start().await;

    // Mock successful NZB download without Content-Disposition header
    Mock::given(method("GET"))
        .and(path("/downloads/My.Movie.2024.nzb"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(SAMPLE_NZB))
        .mount(&mock_server)
        .await;

    // Fetch NZB from mock server
    let url = format!("{}/downloads/My.Movie.2024.nzb", mock_server.uri());
    let download_id = downloader
        .add_nzb_url(&url, DownloadOptions::default())
        .await
        .unwrap();

    // Verify download was created with filename from URL path
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(download.name, "My.Movie.2024");
}

#[tokio::test]
async fn test_add_nzb_url_http_404() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start mock HTTP server
    let mock_server = MockServer::start().await;

    // Mock slow response that exceeds timeout (30 seconds)
    // Note: This test would take 30+ seconds to run, so we'll test connection failure instead
    Mock::given(method("GET"))
        .and(path("/slow.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(35)) // Exceeds 30 second timeout
                .set_body_bytes(SAMPLE_NZB),
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Start mock HTTP server
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/movie.nzb"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(SAMPLE_NZB))
        .mount(&mock_server)
        .await;

    // Fetch NZB with options
    let options = DownloadOptions {
        category: Some("movies".to_string()),
        priority: Priority::High,
        ..Default::default()
    };

    let url = format!("{}/movie.nzb", mock_server.uri());
    let download_id = downloader.add_nzb_url(&url, options).await.unwrap();

    // Verify options were applied
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(download.category, Some("movies".to_string()));
    assert_eq!(download.priority, Priority::High as i32);
}

#[tokio::test]
async fn test_start_folder_watcher_no_watch_folders() {
    // Create downloader with no watch folders configured
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Should succeed but return a completed task
    let handle = downloader.start_folder_watcher();
    assert!(
        handle.is_ok(),
        "start_folder_watcher should succeed with no watch folders"
    );

    // The task should complete immediately
    let result = tokio::time::timeout(Duration::from_millis(100), handle.unwrap()).await;
    assert!(
        result.is_ok(),
        "Task should complete immediately with no watch folders"
    );
}

#[tokio::test]
async fn test_start_folder_watcher_with_configured_folders() {
    let temp_dir = tempdir().unwrap();
    let watch_path = temp_dir.path().join("watch");

    // Create config with watch folder
    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: temp_dir.path().join("test.db"),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        servers: vec![],
        automation: config::AutomationConfig {
            watch_folders: vec![config::WatchFolderConfig {
                path: watch_path.clone(),
                after_import: config::WatchFolderAction::Delete,
                category: Some("test".to_string()),
                scan_interval: Duration::from_secs(5),
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start folder watcher
    let handle = downloader.start_folder_watcher();
    assert!(
        handle.is_ok(),
        "start_folder_watcher should succeed: {:?}",
        handle.err()
    );

    // Verify watch directory was created
    assert!(
        watch_path.exists(),
        "Watch folder should be created by start()"
    );

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
        persistence: crate::config::PersistenceConfig {
            database_path: temp_dir.path().join("test.db"),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        servers: vec![],
        automation: config::AutomationConfig {
            watch_folders: vec![config::WatchFolderConfig {
                path: watch_path.clone(),
                after_import: config::WatchFolderAction::MoveToProcessed,
                category: None,
                scan_interval: Duration::from_secs(5),
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Start folder watcher - should create the directory
    let handle = downloader.start_folder_watcher();
    assert!(
        handle.is_ok(),
        "start_folder_watcher should create missing directories: {:?}",
        handle.err()
    );

    // Verify directory was created
    assert!(watch_path.exists(), "Watch folder should be auto-created");

    // Abort the task
    handle.unwrap().abort();
}

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
