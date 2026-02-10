use super::*;
use crate::types::DownloadId;

#[tokio::test]
async fn test_list_downloads_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add some test downloads to the database
    use crate::db::NewDownload;

    let new_download1 = NewDownload {
        name: "Test Download 1".to_string(),
        nzb_path: "/tmp/test1.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("hash1".to_string()),
        job_name: Some("Test Download 1".to_string()),
        category: Some("movies".to_string()),
        destination: "/downloads".to_string(),
        post_process: 4,               // UnpackAndCleanup
        priority: 0,                   // Normal
        status: 0,                     // Queued
        size_bytes: 1024 * 1024 * 100, // 100 MB
    };

    let new_download2 = NewDownload {
        name: "Test Download 2".to_string(),
        nzb_path: "/tmp/test2.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("hash2".to_string()),
        job_name: Some("Test Download 2".to_string()),
        category: Some("tv".to_string()),
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 1,                   // High
        status: 1,                     // Downloading
        size_bytes: 1024 * 1024 * 500, // 500 MB
    };

    // Insert downloads into database
    downloader.db.insert_download(&new_download1).await.unwrap();
    downloader.db.insert_download(&new_download2).await.unwrap();

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader, config);

    // Make a request to list downloads
    let request = Request::builder()
        .uri("/downloads")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Check that response is successful
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "list_downloads should return 200 OK"
    );

    // Parse response body
    use axum::body::to_bytes;
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let downloads: Vec<crate::types::DownloadInfo> =
        serde_json::from_slice(&body).expect("Response should be valid JSON");

    // Verify we got both downloads
    assert_eq!(
        downloads.len(),
        2,
        "Should return both downloads that were created"
    );

    // Verify download details
    let download1 = downloads
        .iter()
        .find(|d| d.name == "Test Download 1")
        .unwrap();
    assert_eq!(download1.category, Some("movies".to_string()));
    assert_eq!(download1.status, crate::types::Status::Queued);
    assert_eq!(download1.priority, crate::types::Priority::Normal);
    assert_eq!(download1.size_bytes, 1024 * 1024 * 100);

    let download2 = downloads
        .iter()
        .find(|d| d.name == "Test Download 2")
        .unwrap();
    assert_eq!(download2.category, Some("tv".to_string()));
    assert_eq!(download2.status, crate::types::Status::Downloading);
    assert_eq!(download2.priority, crate::types::Priority::High);
    assert_eq!(download2.size_bytes, 1024 * 1024 * 500);

    println!("✅ list_downloads endpoint test passed!");
    println!("   - Returned {} downloads", downloads.len());
    println!("   - Status codes and data structure validated");
}

#[tokio::test]
async fn test_get_download_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a test download to the database
    use crate::db::NewDownload;

    let new_download = NewDownload {
        name: "Test Download".to_string(),
        nzb_path: "/tmp/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("test_hash".to_string()),
        job_name: Some("Test Download".to_string()),
        category: Some("movies".to_string()),
        destination: "/downloads".to_string(),
        post_process: 4,               // UnpackAndCleanup
        priority: 0,                   // Normal
        status: 0,                     // Queued
        size_bytes: 1024 * 1024 * 100, // 100 MB
    };

    // Insert download and get its ID
    let download_id = downloader.db.insert_download(&new_download).await.unwrap();

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app_clone = create_router(downloader.clone(), config.clone());

    // Test 1: Get existing download
    let request = Request::builder()
        .uri(format!("/downloads/{}", download_id))
        .body(Body::empty())
        .unwrap();

    let response = app_clone.oneshot(request).await.unwrap();

    // Check that response is successful
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "get_download should return 200 OK for existing download"
    );

    // Parse response body
    use axum::body::to_bytes;
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let download_info: crate::types::DownloadInfo =
        serde_json::from_slice(&body).expect("Response should be valid JSON");

    // Verify download details
    assert_eq!(download_info.id, download_id);
    assert_eq!(download_info.name, "Test Download");
    assert_eq!(download_info.category, Some("movies".to_string()));
    assert_eq!(download_info.status, crate::types::Status::Queued);
    assert_eq!(download_info.priority, crate::types::Priority::Normal);
    assert_eq!(download_info.size_bytes, 1024 * 1024 * 100);

    println!("✅ get_download endpoint test (existing download) passed!");
    println!("   - Download ID: {}", download_info.id);
    println!("   - Download name: {}", download_info.name);

    // Test 2: Get non-existent download (should return 404)
    let app_clone2 = create_router(downloader, config);
    let request = Request::builder()
        .uri("/downloads/99999")
        .body(Body::empty())
        .unwrap();

    let response = app_clone2.oneshot(request).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "get_download should return 404 for non-existent download"
    );

    println!("✅ get_download endpoint test (non-existent download) passed!");
    println!("   - Correctly returns 404 for missing download");
}

#[tokio::test]
async fn test_add_download_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt; // for oneshot()

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create a minimal valid NZB file content
    let nzb_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.com" date="1234567890" subject="Test File">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="100000" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Create multipart form data manually
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let body = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"test.nzb\"\r\n\
         Content-Type: application/x-nzb\r\n\
         \r\n\
         {nzb_content}\r\n\
         --{boundary}\r\n\
         Content-Disposition: form-data; name=\"options\"\r\n\
         \r\n\
         {{\"category\":\"movies\",\"priority\":\"high\"}}\r\n\
         --{boundary}--\r\n",
        boundary = boundary,
        nzb_content = nzb_content
    );

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config.clone());

    // Test: Upload NZB file with options
    let request = Request::builder()
        .method("POST")
        .uri("/downloads")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Check that response is 201 CREATED
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "add_download should return 201 CREATED for valid NZB"
    );

    // Parse response body to get download ID
    use axum::body::to_bytes;
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let response_json: serde_json::Value =
        serde_json::from_slice(&body).expect("Response should be valid JSON");

    let download_id = response_json["id"]
        .as_i64()
        .expect("Response should contain download ID");

    println!("✅ add_download endpoint test passed!");
    println!("   - Download ID created: {}", download_id);

    // Verify download was actually added to database
    let download = downloader
        .db
        .get_download(DownloadId(download_id))
        .await
        .unwrap()
        .expect("Download should exist in database");

    assert_eq!(download.name, "test.nzb");
    assert_eq!(download.category, Some("movies".to_string()));
    assert_eq!(download.priority, 1); // High priority

    println!("   - Download verified in database");
    println!("   - Name: {}", download.name);
    println!("   - Category: {:?}", download.category);
    println!("   - Priority: {} (High)", download.priority);

    // Test 2: Upload NZB without options (should use defaults)
    let nzb_content2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.com" date="1234567890" subject="Test File 2">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="200000" number="1">test-message-id-2@example.com</segment>
</segments>
  </file>
</nzb>"#;

    let body2 = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"test2.nzb\"\r\n\
         Content-Type: application/x-nzb\r\n\
         \r\n\
         {nzb_content}\r\n\
         --{boundary}--\r\n",
        boundary = boundary,
        nzb_content = nzb_content2
    );

    let app2 = create_router(downloader.clone(), config.clone());
    let request2 = Request::builder()
        .method("POST")
        .uri("/downloads")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body2))
        .unwrap();

    let response2 = app2.oneshot(request2).await.unwrap();

    assert_eq!(
        response2.status(),
        StatusCode::CREATED,
        "add_download should work without options field"
    );

    println!("✅ add_download endpoint test (no options) passed!");

    // Test 3: Missing file should return 400 BAD_REQUEST
    let body3 = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"other\"\r\n\
         \r\n\
         not a file\r\n\
         --{boundary}--\r\n",
        boundary = boundary
    );

    let app3 = create_router(downloader, config);
    let request3 = Request::builder()
        .method("POST")
        .uri("/downloads")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body3))
        .unwrap();

    let response3 = app3.oneshot(request3).await.unwrap();

    assert_eq!(
        response3.status(),
        StatusCode::BAD_REQUEST,
        "add_download should return 400 when file field is missing"
    );

    println!("✅ add_download endpoint test (missing file) passed!");
    println!("   - Correctly returns 400 for missing file field");
}

#[tokio::test]
async fn test_add_download_url_endpoint() {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing POST /downloads/url endpoint...");

    // NOTE: This test requires a mock HTTP server or will skip if unable to create one
    // For now, we'll test the error cases which don't require actual network calls

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = Arc::clone(&downloader.config);
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: Missing URL field
    println!("  📝 Test 1: Missing URL field");
    let request1 = Request::builder()
        .method("POST")
        .uri("/downloads/url")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"options": {}}"#))
        .unwrap();

    let response1 = app.clone().oneshot(request1).await.unwrap();
    assert_eq!(
        response1.status(),
        StatusCode::BAD_REQUEST,
        "Should return 400 when URL is missing"
    );

    let body1 = to_bytes(response1.into_body(), usize::MAX).await.unwrap();
    let json1: serde_json::Value = serde_json::from_slice(&body1).unwrap();
    assert_eq!(json1["error"]["code"], "missing_url");

    println!("    ✓ Returns 400 BAD_REQUEST when URL is missing");

    // Test 2: Invalid options JSON
    println!("  📝 Test 2: Invalid download options");
    let request2 = Request::builder()
        .method("POST")
        .uri("/downloads/url")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"url": "https://example.com/test.nzb", "options": "invalid"}"#,
        ))
        .unwrap();

    let response2 = app.clone().oneshot(request2).await.unwrap();
    assert_eq!(
        response2.status(),
        StatusCode::BAD_REQUEST,
        "Should return 400 when options are invalid"
    );

    let body2 = to_bytes(response2.into_body(), usize::MAX).await.unwrap();
    let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
    assert_eq!(json2["error"]["code"], "invalid_options");

    println!("    ✓ Returns 400 BAD_REQUEST when options are invalid");

    // Test 3: Invalid/unreachable URL (will fail in add_nzb_url)
    println!("  📝 Test 3: Invalid URL");
    let request3 = Request::builder()
        .method("POST")
        .uri("/downloads/url")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"url": "http://invalid-nonexistent-domain-12345.com/test.nzb"}"#,
        ))
        .unwrap();

    let response3 = app.clone().oneshot(request3).await.unwrap();
    // Should return 400 for network/IO error
    assert_eq!(
        response3.status(),
        StatusCode::BAD_REQUEST,
        "Should return 400 when URL is unreachable"
    );

    let body3 = to_bytes(response3.into_body(), usize::MAX).await.unwrap();
    let json3: serde_json::Value = serde_json::from_slice(&body3).unwrap();
    // Error code can be io_error, network_error, or add_failed depending on the error type
    assert!(
        json3["error"]["code"] == "io_error"
            || json3["error"]["code"] == "network_error"
            || json3["error"]["code"] == "add_failed",
        "Expected io_error, network_error, or add_failed, got: {}",
        json3["error"]["code"]
    );

    println!("    ✓ Returns 400 BAD_REQUEST when URL is invalid/unreachable");

    println!("✅ add_download_url endpoint test passed!");
    println!("   - Correctly handles missing URL field");
    println!("   - Correctly handles invalid options");
    println!("   - Correctly handles network errors");
}

#[tokio::test]
async fn test_pause_download_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing POST /downloads/:id/pause endpoint...");

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a test download to the database
    use crate::db::NewDownload;

    let new_download = NewDownload {
        name: "Test Download".to_string(),
        nzb_path: "/tmp/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("test_hash".to_string()),
        job_name: Some("Test Download".to_string()),
        category: Some("movies".to_string()),
        destination: "/downloads".to_string(),
        post_process: 4,               // UnpackAndCleanup
        priority: 0,                   // Normal
        status: 1,                     // Downloading (so it can be paused)
        size_bytes: 1024 * 1024 * 100, // 100 MB
    };

    // Insert download and get its ID
    let download_id = downloader.db.insert_download(&new_download).await.unwrap();

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: Pause existing download
    println!("  📝 Test 1: Pause existing download");
    let request = Request::builder()
        .method("POST")
        .uri(format!("/downloads/{}/pause", download_id))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();

    // Check that response is successful
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "pause_download should return 204 NO_CONTENT for existing download"
    );

    // Verify download is now paused in database
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::types::Status::from_i32(download.status),
        crate::types::Status::Paused,
        "Download status should be Paused after pause"
    );

    println!("    ✓ Returns 204 NO_CONTENT");
    println!("    ✓ Download status is now Paused");

    // Test 2: Pause non-existent download (should return 404)
    println!("  📝 Test 2: Pause non-existent download");
    let request2 = Request::builder()
        .method("POST")
        .uri("/downloads/99999/pause")
        .body(Body::empty())
        .unwrap();

    let response2 = app.clone().oneshot(request2).await.unwrap();

    assert_eq!(
        response2.status(),
        StatusCode::NOT_FOUND,
        "pause_download should return 404 for non-existent download"
    );

    println!("    ✓ Returns 404 NOT_FOUND for non-existent download");

    // Test 3: Try to pause a completed download (should return 409 CONFLICT)
    println!("  📝 Test 3: Pause completed download");

    // Create a completed download
    let completed_download = NewDownload {
        name: "Completed Download".to_string(),
        nzb_path: "/tmp/completed.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("completed_hash".to_string()),
        job_name: Some("Completed Download".to_string()),
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 4, // Complete
        size_bytes: 1024 * 1024,
    };

    let completed_id = downloader
        .db
        .insert_download(&completed_download)
        .await
        .unwrap();

    let request3 = Request::builder()
        .method("POST")
        .uri(format!("/downloads/{}/pause", completed_id))
        .body(Body::empty())
        .unwrap();

    let response3 = app.oneshot(request3).await.unwrap();

    assert_eq!(
        response3.status(),
        StatusCode::CONFLICT,
        "pause_download should return 409 CONFLICT for completed download"
    );

    println!("    ✓ Returns 409 CONFLICT for completed download");

    println!("✅ pause_download endpoint test passed!");
    println!("   - Successfully pauses downloading");
    println!("   - Returns 404 for non-existent downloads");
    println!("   - Returns 409 for downloads in terminal states");
}

#[tokio::test]
async fn test_resume_download_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing POST /downloads/:id/resume endpoint...");

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a test download to the database in Paused state
    use crate::db::NewDownload;

    let new_download = NewDownload {
        name: "Paused Download".to_string(),
        nzb_path: "/tmp/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("test_hash".to_string()),
        job_name: Some("Paused Download".to_string()),
        category: Some("movies".to_string()),
        destination: "/downloads".to_string(),
        post_process: 4,               // UnpackAndCleanup
        priority: 0,                   // Normal
        status: 2,                     // Paused (so it can be resumed)
        size_bytes: 1024 * 1024 * 100, // 100 MB
    };

    // Insert download and get its ID
    let download_id = downloader.db.insert_download(&new_download).await.unwrap();

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: Resume paused download
    println!("  📝 Test 1: Resume paused download");
    let request = Request::builder()
        .method("POST")
        .uri(format!("/downloads/{}/resume", download_id))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();

    // Check that response is successful
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "resume_download should return 204 NO_CONTENT for paused download"
    );

    // Verify download is now queued in database
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::types::Status::from_i32(download.status),
        crate::types::Status::Queued,
        "Download status should be Queued after resume"
    );

    println!("    ✓ Returns 204 NO_CONTENT");
    println!("    ✓ Download status is now Queued");

    // Test 2: Resume non-existent download (should return 404)
    println!("  📝 Test 2: Resume non-existent download");
    let request2 = Request::builder()
        .method("POST")
        .uri("/downloads/99999/resume")
        .body(Body::empty())
        .unwrap();

    let response2 = app.clone().oneshot(request2).await.unwrap();

    assert_eq!(
        response2.status(),
        StatusCode::NOT_FOUND,
        "resume_download should return 404 for non-existent download"
    );

    println!("    ✓ Returns 404 NOT_FOUND for non-existent download");

    // Test 3: Try to resume a completed download (should return 409 CONFLICT)
    println!("  📝 Test 3: Resume completed download");

    // Create a completed download
    let completed_download = NewDownload {
        name: "Completed Download".to_string(),
        nzb_path: "/tmp/completed.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("completed_hash".to_string()),
        job_name: Some("Completed Download".to_string()),
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 4, // Complete
        size_bytes: 1024 * 1024,
    };

    let completed_id = downloader
        .db
        .insert_download(&completed_download)
        .await
        .unwrap();

    let request3 = Request::builder()
        .method("POST")
        .uri(format!("/downloads/{}/resume", completed_id))
        .body(Body::empty())
        .unwrap();

    let response3 = app.clone().oneshot(request3).await.unwrap();

    assert_eq!(
        response3.status(),
        StatusCode::CONFLICT,
        "resume_download should return 409 CONFLICT for completed download"
    );

    println!("    ✓ Returns 409 CONFLICT for completed download");

    // Test 4: Resume already active download (should be idempotent - return 204)
    println!("  📝 Test 4: Resume already queued download (idempotent)");

    // Create a queued download
    let queued_download = NewDownload {
        name: "Queued Download".to_string(),
        nzb_path: "/tmp/queued.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("queued_hash".to_string()),
        job_name: Some("Queued Download".to_string()),
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0, // Queued
        size_bytes: 1024 * 1024,
    };

    let queued_id = downloader
        .db
        .insert_download(&queued_download)
        .await
        .unwrap();

    let request4 = Request::builder()
        .method("POST")
        .uri(format!("/downloads/{}/resume", queued_id))
        .body(Body::empty())
        .unwrap();

    let response4 = app.oneshot(request4).await.unwrap();

    assert_eq!(
        response4.status(),
        StatusCode::NO_CONTENT,
        "resume_download should return 204 for already-queued download (idempotent)"
    );

    println!("    ✓ Returns 204 NO_CONTENT for already-queued download (idempotent)");

    println!("✅ resume_download endpoint test passed!");
    println!("   - Successfully resumes paused downloads");
    println!("   - Returns 404 for non-existent downloads");
    println!("   - Returns 409 for downloads in terminal states");
    println!("   - Idempotent for already-active downloads");
}

#[tokio::test]
async fn test_delete_download_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing DELETE /downloads/:id endpoint...");

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a test download to the database
    use crate::db::NewDownload;

    let new_download = NewDownload {
        name: "Download to Delete".to_string(),
        nzb_path: "/tmp/test_delete.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("delete_hash".to_string()),
        job_name: Some("Download to Delete".to_string()),
        category: Some("movies".to_string()),
        destination: "/downloads".to_string(),
        post_process: 4,               // UnpackAndCleanup
        priority: 0,                   // Normal
        status: 0,                     // Queued
        size_bytes: 1024 * 1024 * 100, // 100 MB
    };

    // Insert download and get its ID
    let download_id = downloader.db.insert_download(&new_download).await.unwrap();

    // Verify download was created
    assert!(
        downloader
            .db
            .get_download(download_id)
            .await
            .unwrap()
            .is_some(),
        "Download should exist before deletion"
    );

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: Delete existing download
    println!("  📝 Test 1: Delete existing download");
    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/downloads/{}", download_id))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();

    // Check that response is successful
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "delete_download should return 204 NO_CONTENT for existing download"
    );

    // Verify download was deleted from database
    assert!(
        downloader
            .db
            .get_download(download_id)
            .await
            .unwrap()
            .is_none(),
        "Download should not exist after deletion"
    );

    println!("    ✓ Returns 204 NO_CONTENT");
    println!("    ✓ Download removed from database");

    // Test 2: Delete non-existent download (should return 404)
    println!("  📝 Test 2: Delete non-existent download");
    let request2 = Request::builder()
        .method("DELETE")
        .uri("/downloads/99999")
        .body(Body::empty())
        .unwrap();

    let response2 = app.clone().oneshot(request2).await.unwrap();

    assert_eq!(
        response2.status(),
        StatusCode::NOT_FOUND,
        "delete_download should return 404 for non-existent download"
    );

    println!("    ✓ Returns 404 NOT_FOUND for non-existent download");

    // Test 3: Delete with delete_files query parameter
    println!("  📝 Test 3: Delete with delete_files query parameter");

    // Create another download
    let download2 = NewDownload {
        name: "Download to Delete 2".to_string(),
        nzb_path: "/tmp/test_delete2.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("delete_hash2".to_string()),
        job_name: Some("Download to Delete 2".to_string()),
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0,
        size_bytes: 1024 * 1024,
    };

    let download_id2 = downloader.db.insert_download(&download2).await.unwrap();

    let request3 = Request::builder()
        .method("DELETE")
        .uri(format!("/downloads/{}?delete_files=true", download_id2))
        .body(Body::empty())
        .unwrap();

    let response3 = app.oneshot(request3).await.unwrap();

    assert_eq!(
        response3.status(),
        StatusCode::NO_CONTENT,
        "delete_download should return 204 with delete_files parameter"
    );

    // Verify download was deleted
    assert!(
        downloader
            .db
            .get_download(download_id2)
            .await
            .unwrap()
            .is_none(),
        "Download should not exist after deletion with delete_files=true"
    );

    println!("    ✓ Returns 204 NO_CONTENT with delete_files=true");
    println!("    ✓ Download removed from database");

    println!("✅ delete_download endpoint test passed!");
    println!("   - Successfully deletes existing downloads");
    println!("   - Returns 404 for non-existent downloads");
    println!("   - Accepts delete_files query parameter");
}

#[tokio::test]
async fn test_set_download_priority_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing PATCH /downloads/:id/priority endpoint...");

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a test download to the database in Queued state
    use crate::db::NewDownload;

    let new_download = NewDownload {
        name: "Test Download".to_string(),
        nzb_path: "/tmp/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("test_hash".to_string()),
        job_name: Some("Test Download".to_string()),
        category: Some("movies".to_string()),
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0, // Normal
        status: 0,   // Queued
        size_bytes: 1024 * 1024 * 100,
    };

    // Insert download and get its ID
    let download_id = downloader.db.insert_download(&new_download).await.unwrap();

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: Set priority to High
    println!("  📝 Test 1: Set priority to High");
    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/downloads/{}/priority", download_id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"priority": "high"}"#))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "set_download_priority should return 204 NO_CONTENT for valid priority"
    );

    // Verify priority was updated in database
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::types::Priority::from_i32(download.priority),
        crate::types::Priority::High,
        "Download priority should be High after update"
    );

    println!("    ✓ Returns 204 NO_CONTENT");
    println!("    ✓ Priority updated to High in database");

    // Test 2: Set priority to Low
    println!("  📝 Test 2: Set priority to Low");
    let request2 = Request::builder()
        .method("PATCH")
        .uri(format!("/downloads/{}/priority", download_id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"priority": "low"}"#))
        .unwrap();

    let response2 = app.clone().oneshot(request2).await.unwrap();

    assert_eq!(
        response2.status(),
        StatusCode::NO_CONTENT,
        "set_download_priority should return 204 NO_CONTENT for Low priority"
    );

    // Verify priority was updated
    let download2 = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::types::Priority::from_i32(download2.priority),
        crate::types::Priority::Low,
        "Download priority should be Low after update"
    );

    println!("    ✓ Priority updated to Low");

    // Test 3: Set priority to Force
    println!("  📝 Test 3: Set priority to Force");
    let request3 = Request::builder()
        .method("PATCH")
        .uri(format!("/downloads/{}/priority", download_id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"priority": "force"}"#))
        .unwrap();

    let response3 = app.clone().oneshot(request3).await.unwrap();

    assert_eq!(
        response3.status(),
        StatusCode::NO_CONTENT,
        "set_download_priority should return 204 NO_CONTENT for Force priority"
    );

    // Verify priority was updated
    let download3 = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::types::Priority::from_i32(download3.priority),
        crate::types::Priority::Force,
        "Download priority should be Force after update"
    );

    println!("    ✓ Priority updated to Force");

    // Test 4: Missing priority field (should return 400)
    println!("  📝 Test 4: Missing priority field");
    let request4 = Request::builder()
        .method("PATCH")
        .uri(format!("/downloads/{}/priority", download_id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{}"#))
        .unwrap();

    let response4 = app.clone().oneshot(request4).await.unwrap();

    assert_eq!(
        response4.status(),
        StatusCode::BAD_REQUEST,
        "set_download_priority should return 400 BAD_REQUEST for missing priority field"
    );

    // Parse response body
    use axum::body::to_bytes;
    let body_bytes = to_bytes(response4.into_body(), usize::MAX).await.unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(
        body_json["error"]["code"].as_str().unwrap(),
        "missing_priority",
        "Error code should be 'missing_priority'"
    );

    println!("    ✓ Returns 400 BAD_REQUEST for missing priority");

    // Test 5: Invalid priority value (should return 400)
    println!("  📝 Test 5: Invalid priority value");
    let request5 = Request::builder()
        .method("PATCH")
        .uri(format!("/downloads/{}/priority", download_id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"priority": "invalid_priority"}"#))
        .unwrap();

    let response5 = app.clone().oneshot(request5).await.unwrap();

    assert_eq!(
        response5.status(),
        StatusCode::BAD_REQUEST,
        "set_download_priority should return 400 BAD_REQUEST for invalid priority value"
    );

    let body_bytes5 = to_bytes(response5.into_body(), usize::MAX).await.unwrap();
    let body_json5: serde_json::Value = serde_json::from_slice(&body_bytes5).unwrap();

    assert_eq!(
        body_json5["error"]["code"].as_str().unwrap(),
        "invalid_priority",
        "Error code should be 'invalid_priority'"
    );

    println!("    ✓ Returns 400 BAD_REQUEST for invalid priority");

    // Test 6: Non-existent download (should return 404)
    println!("  📝 Test 6: Non-existent download");
    let request6 = Request::builder()
        .method("PATCH")
        .uri("/downloads/99999/priority")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"priority": "high"}"#))
        .unwrap();

    let response6 = app.oneshot(request6).await.unwrap();

    assert_eq!(
        response6.status(),
        StatusCode::NOT_FOUND,
        "set_download_priority should return 404 NOT_FOUND for non-existent download"
    );

    println!("    ✓ Returns 404 NOT_FOUND for non-existent download");

    println!("✅ set_download_priority endpoint test passed!");
    println!("   - Successfully updates priority to High, Low, and Force");
    println!("   - Returns 400 for missing priority field");
    println!("   - Returns 400 for invalid priority value");
    println!("   - Returns 404 for non-existent downloads");
}

#[tokio::test]
async fn test_reprocess_download_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing POST /downloads/:id/reprocess endpoint...");

    // Setup
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config);

    // Create temp directory for test
    std::fs::create_dir_all(&downloader.config.download.temp_dir).unwrap();

    // Add a test download
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

    // Create download directory with a test file
    let download_path = downloader
        .config
        .temp_dir()
        .join(format!("download_{}", download_id));
    std::fs::create_dir_all(&download_path).unwrap();
    std::fs::write(download_path.join("test.txt"), "test content").unwrap();

    // Mark download as complete (so we can reprocess it)
    downloader
        .db
        .update_status(download_id, crate::types::Status::Complete.to_i32())
        .await
        .unwrap();

    println!("  📝 Created test download with ID: {}", download_id);

    // Test 1: Reprocess existing download
    println!("  🔍 Test 1: Reprocess existing download with files");
    let request = Request::builder()
        .method("POST")
        .uri(format!("/downloads/{}/reprocess", download_id))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "reprocess should return 204 NO_CONTENT"
    );

    println!("    ✓ Returns 204 NO_CONTENT for successful reprocess");

    // Test 2: Reprocess download with missing files
    println!("  🔍 Test 2: Reprocess download with missing files");

    // Remove the download directory (ignore error if already removed)
    let _ = std::fs::remove_dir_all(&download_path);

    let request = Request::builder()
        .method("POST")
        .uri(format!("/downloads/{}/reprocess", download_id))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "reprocess should return 404 NOT_FOUND when files are missing"
    );

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let response_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        response_json["error"]["code"], "files_not_found",
        "Error code should be 'files_not_found'"
    );

    println!("    ✓ Returns 404 NOT_FOUND when files are missing");
    println!("    ✓ Returns correct error code 'files_not_found'");

    // Test 3: Reprocess non-existent download
    println!("  🔍 Test 3: Reprocess non-existent download");
    let request = Request::builder()
        .method("POST")
        .uri("/downloads/999999/reprocess")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "reprocess should return 404 NOT_FOUND for non-existent download"
    );

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let response_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        response_json["error"]["code"], "not_found",
        "Error code should be 'not_found'"
    );

    println!("    ✓ Returns 404 NOT_FOUND for non-existent download");
    println!("    ✓ Returns correct error code 'not_found'");

    println!("✅ reprocess_download endpoint test passed!");
    println!("   - Returns 204 NO_CONTENT for successful reprocess");
    println!("   - Returns 404 with 'files_not_found' when download files are missing");
    println!("   - Returns 404 with 'not_found' for non-existent downloads");
}

#[tokio::test]
async fn test_reextract_download_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing POST /downloads/:id/reextract endpoint...");

    // Setup
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config);

    // Create temp directory for test
    std::fs::create_dir_all(&downloader.config.download.temp_dir).unwrap();

    // Add a test download
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

    // Create download directory with a test file
    let download_path = downloader
        .config
        .temp_dir()
        .join(format!("download_{}", download_id));
    std::fs::create_dir_all(&download_path).unwrap();
    std::fs::write(download_path.join("test.txt"), "test content").unwrap();

    // Mark download as complete (so we can re-extract it)
    downloader
        .db
        .update_status(download_id, crate::types::Status::Complete.to_i32())
        .await
        .unwrap();

    println!("  📝 Created test download with ID: {}", download_id);

    // Test 1: Re-extract existing download
    println!("  🔍 Test 1: Re-extract existing download with files");
    let request = Request::builder()
        .method("POST")
        .uri(format!("/downloads/{}/reextract", download_id))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "reextract should return 204 NO_CONTENT"
    );

    println!("    ✓ Returns 204 NO_CONTENT for successful re-extraction");

    // Test 2: Re-extract download with missing files (use separate download with no temp dir)
    println!("  🔍 Test 2: Re-extract download with missing files");

    let download_id_2 = downloader
        .add_nzb_content(
            nzb_content.as_bytes(),
            "test2.nzb",
            crate::types::DownloadOptions::default(),
        )
        .await
        .unwrap();

    // Mark as complete but DON'T create temp directory
    downloader
        .db
        .update_status(download_id_2, crate::types::Status::Complete.to_i32())
        .await
        .unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/downloads/{}/reextract", download_id_2))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "reextract should return 404 NOT_FOUND when files are missing"
    );

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let response_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        response_json["error"]["code"], "files_not_found",
        "Error code should be 'files_not_found'"
    );

    println!("    ✓ Returns 404 NOT_FOUND when files are missing");
    println!("    ✓ Returns correct error code 'files_not_found'");

    // Test 3: Re-extract non-existent download
    println!("  🔍 Test 3: Re-extract non-existent download");
    let request = Request::builder()
        .method("POST")
        .uri("/downloads/999999/reextract")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "reextract should return 404 NOT_FOUND for non-existent download"
    );

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let response_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        response_json["error"]["code"], "not_found",
        "Error code should be 'not_found'"
    );

    println!("    ✓ Returns 404 NOT_FOUND for non-existent download");
    println!("    ✓ Returns correct error code 'not_found'");

    println!("✅ reextract_download endpoint test passed!");
    println!("   - Returns 204 NO_CONTENT for successful re-extraction");
    println!("   - Returns 404 with 'files_not_found' when download files are missing");
    println!("   - Returns 404 with 'not_found' for non-existent downloads");
}
