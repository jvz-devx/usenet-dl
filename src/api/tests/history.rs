use super::*;

#[tokio::test]
async fn test_get_history_endpoint() {
    use crate::db::NewHistoryEntry;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing GET /history endpoint...");

    // Setup
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config);

    // Test 1: Empty history
    println!("  🔍 Test 1: Empty history returns empty array");
    let request = Request::builder()
        .method("GET")
        .uri("/history")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "GET /history should return 200 OK"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["items"].as_array().unwrap().len(),
        0,
        "Empty history should have 0 items"
    );
    assert_eq!(
        json["total"].as_i64().unwrap(),
        0,
        "Empty history should have total=0"
    );
    assert_eq!(
        json["limit"].as_i64().unwrap(),
        50,
        "Default limit should be 50"
    );
    assert_eq!(
        json["offset"].as_i64().unwrap(),
        0,
        "Default offset should be 0"
    );
    println!("    ✓ Empty history returns correct structure");

    // Test 2: Add history entries
    println!("  🔍 Test 2: History with entries");

    // Add some history entries directly to the database
    use chrono::Utc;
    use std::path::PathBuf;

    for i in 1..=5 {
        let entry = NewHistoryEntry {
            name: format!("Download {}", i),
            category: Some("test".to_string()),
            destination: Some(PathBuf::from(format!("/downloads/test{}", i))),
            status: 4, // Complete
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: Utc::now().timestamp(),
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    // Add 2 failed downloads
    for i in 6..=7 {
        let entry = NewHistoryEntry {
            name: format!("Download {}", i),
            category: Some("test".to_string()),
            destination: None,
            status: 5, // Failed
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: Utc::now().timestamp(),
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    println!("  📝 Created 7 history entries (5 complete, 2 failed)");

    // Query all history
    let request = Request::builder()
        .method("GET")
        .uri("/history")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["items"].as_array().unwrap().len(),
        7,
        "Should have 7 items"
    );
    assert_eq!(json["total"].as_i64().unwrap(), 7, "Total should be 7");
    println!("    ✓ All history entries returned");

    // Test 3: Pagination
    println!("  🔍 Test 3: Pagination with limit and offset");
    let request = Request::builder()
        .method("GET")
        .uri("/history?limit=3&offset=2")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["items"].as_array().unwrap().len(),
        3,
        "Should return 3 items"
    );
    assert_eq!(json["limit"].as_i64().unwrap(), 3, "Limit should be 3");
    assert_eq!(json["offset"].as_i64().unwrap(), 2, "Offset should be 2");
    assert_eq!(
        json["total"].as_i64().unwrap(),
        7,
        "Total should still be 7"
    );
    println!("    ✓ Pagination works correctly");

    // Test 4: Filter by status - complete
    println!("  🔍 Test 4: Filter by status=complete");
    let request = Request::builder()
        .method("GET")
        .uri("/history?status=complete")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["items"].as_array().unwrap().len(),
        5,
        "Should have 5 complete items"
    );
    assert_eq!(json["total"].as_i64().unwrap(), 5, "Total should be 5");
    println!("    ✓ status=complete filter works");

    // Test 5: Filter by status - failed
    println!("  🔍 Test 5: Filter by status=failed");
    let request = Request::builder()
        .method("GET")
        .uri("/history?status=failed")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["items"].as_array().unwrap().len(),
        2,
        "Should have 2 failed items"
    );
    assert_eq!(json["total"].as_i64().unwrap(), 2, "Total should be 2");
    println!("    ✓ status=failed filter works");

    // Test 6: Invalid status filter
    println!("  🔍 Test 6: Invalid status filter returns 400");
    let request = Request::builder()
        .method("GET")
        .uri("/history?status=invalid")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Invalid status should return 400"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["error"]["code"]
            .as_str()
            .unwrap()
            .contains("invalid_status")
    );
    println!("    ✓ Invalid status returns 400 with error code");

    // Test 7: Limit boundary values
    println!("  🔍 Test 7: Limit boundary values");

    // Very high limit should be capped at 1000
    let request = Request::builder()
        .method("GET")
        .uri("/history?limit=9999")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["limit"].as_i64().unwrap(),
        1000,
        "Limit should be capped at 1000"
    );

    // Zero or negative limit should be converted to 1
    let request = Request::builder()
        .method("GET")
        .uri("/history?limit=0")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["limit"].as_i64().unwrap(),
        1,
        "Limit should be at least 1"
    );
    println!("    ✓ Limit boundary values handled correctly");

    println!("✅ GET /history endpoint test passed!");
    println!("   - Returns 200 OK with valid JSON");
    println!("   - Empty history returns correct structure");
    println!("   - Pagination works correctly (limit, offset)");
    println!("   - Status filtering works (complete/failed)");
    println!("   - Invalid status returns 400");
    println!("   - Limit boundary values handled correctly");
}

#[tokio::test]
async fn test_clear_history_endpoint() {
    use crate::db::NewHistoryEntry;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use std::path::PathBuf;
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing DELETE /history endpoint...");

    // Setup
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config);

    // Test 1: Clear all history (no filters)
    println!("  🔍 Test 1: Clear all history (no filters)");

    // Add 5 complete downloads and 2 failed downloads
    for i in 1..=5 {
        let entry = NewHistoryEntry {
            name: format!("Complete Download {}", i),
            category: Some("test".to_string()),
            destination: Some(PathBuf::from(format!("/downloads/test{}", i))),
            status: 4, // Complete
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: Utc::now().timestamp(),
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    for i in 6..=7 {
        let entry = NewHistoryEntry {
            name: format!("Failed Download {}", i),
            category: Some("test".to_string()),
            destination: None,
            status: 5, // Failed
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: Utc::now().timestamp(),
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    println!("  📝 Created 7 history entries (5 complete, 2 failed)");

    // Delete all history
    let request = Request::builder()
        .method("DELETE")
        .uri("/history")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "DELETE /history should return 200 OK"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["deleted"].as_u64().unwrap(),
        7,
        "Should delete all 7 entries"
    );
    println!("    ✓ Deleted all 7 history entries");

    // Verify history is empty
    let count = downloader.db.count_history(None).await.unwrap();
    assert_eq!(count, 0, "History should be empty after clearing");
    println!("    ✓ History is empty after clearing");

    // Test 2: Clear by status filter (complete)
    println!("  🔍 Test 2: Clear by status=complete");

    // Re-add history entries
    for i in 1..=3 {
        let entry = NewHistoryEntry {
            name: format!("Complete Download {}", i),
            category: Some("test".to_string()),
            destination: Some(PathBuf::from(format!("/downloads/test{}", i))),
            status: 4, // Complete
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: Utc::now().timestamp(),
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    for i in 4..=5 {
        let entry = NewHistoryEntry {
            name: format!("Failed Download {}", i),
            category: Some("test".to_string()),
            destination: None,
            status: 5, // Failed
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: Utc::now().timestamp(),
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    println!("  📝 Created 5 history entries (3 complete, 2 failed)");

    // Delete only complete entries
    let request = Request::builder()
        .method("DELETE")
        .uri("/history?status=complete")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["deleted"].as_u64().unwrap(),
        3,
        "Should delete 3 complete entries"
    );
    println!("    ✓ Deleted 3 complete history entries");

    // Verify only failed entries remain
    let count = downloader.db.count_history(Some(5)).await.unwrap(); // Failed status
    assert_eq!(count, 2, "Should have 2 failed entries remaining");
    println!("    ✓ 2 failed entries remain");

    // Test 3: Clear by status filter (failed)
    println!("  🔍 Test 3: Clear by status=failed");

    let request = Request::builder()
        .method("DELETE")
        .uri("/history?status=failed")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["deleted"].as_u64().unwrap(),
        2,
        "Should delete 2 failed entries"
    );
    println!("    ✓ Deleted 2 failed history entries");

    // Verify history is empty
    let count = downloader.db.count_history(None).await.unwrap();
    assert_eq!(count, 0, "History should be empty");
    println!("    ✓ History is now empty");

    // Test 4: Clear by timestamp (before filter)
    println!("  🔍 Test 4: Clear by timestamp (before filter)");

    let now = Utc::now().timestamp();
    let old_timestamp = now - 3600; // 1 hour ago

    // Add old entries (before 1 hour ago)
    for i in 1..=2 {
        let entry = NewHistoryEntry {
            name: format!("Old Download {}", i),
            category: Some("test".to_string()),
            destination: Some(PathBuf::from(format!("/downloads/old{}", i))),
            status: 4, // Complete
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: old_timestamp - 100, // Older than old_timestamp
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    // Add recent entries
    for i in 3..=4 {
        let entry = NewHistoryEntry {
            name: format!("Recent Download {}", i),
            category: Some("test".to_string()),
            destination: Some(PathBuf::from(format!("/downloads/recent{}", i))),
            status: 4, // Complete
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: now, // Recent
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    println!("  📝 Created 4 history entries (2 old, 2 recent)");

    // Delete old entries
    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/history?before={}", old_timestamp))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["deleted"].as_u64().unwrap(),
        2,
        "Should delete 2 old entries"
    );
    println!("    ✓ Deleted 2 old history entries");

    // Verify only recent entries remain
    let count = downloader.db.count_history(None).await.unwrap();
    assert_eq!(count, 2, "Should have 2 recent entries remaining");
    println!("    ✓ 2 recent entries remain");

    // Test 5: Clear with both filters (before + status)
    println!("  🔍 Test 5: Clear with both filters (before + status)");

    // Clear remaining entries first
    downloader.db.clear_history().await.unwrap();

    // Add mixed entries with different timestamps and statuses
    for i in 1..=2 {
        let entry = NewHistoryEntry {
            name: format!("Old Complete {}", i),
            category: Some("test".to_string()),
            destination: Some(PathBuf::from(format!("/downloads/old_complete{}", i))),
            status: 4, // Complete
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: old_timestamp - 100,
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    for i in 3..=4 {
        let entry = NewHistoryEntry {
            name: format!("Old Failed {}", i),
            category: Some("test".to_string()),
            destination: None,
            status: 5, // Failed
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: old_timestamp - 100,
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    for i in 5..=6 {
        let entry = NewHistoryEntry {
            name: format!("Recent Complete {}", i),
            category: Some("test".to_string()),
            destination: Some(PathBuf::from(format!("/downloads/recent_complete{}", i))),
            status: 4, // Complete
            size_bytes: i * 1000,
            download_time_secs: (i * 60) as i64,
            completed_at: now,
        };
        downloader.db.insert_history(&entry).await.unwrap();
    }

    println!("  📝 Created 6 history entries (2 old complete, 2 old failed, 2 recent complete)");

    // Delete only old complete entries
    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/history?before={}&status=complete", old_timestamp))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["deleted"].as_u64().unwrap(),
        2,
        "Should delete 2 old complete entries"
    );
    println!("    ✓ Deleted 2 old complete history entries");

    // Verify 4 entries remain (2 old failed + 2 recent complete)
    let count = downloader.db.count_history(None).await.unwrap();
    assert_eq!(count, 4, "Should have 4 entries remaining");
    println!("    ✓ 4 entries remain (2 old failed + 2 recent complete)");

    // Test 6: Invalid status filter
    println!("  🔍 Test 6: Invalid status filter returns 400");

    let request = Request::builder()
        .method("DELETE")
        .uri("/history?status=invalid")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Invalid status should return 400"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["error"]["code"]
            .as_str()
            .unwrap()
            .contains("invalid_status")
    );
    println!("    ✓ Invalid status returns 400 with error code");

    println!("✅ DELETE /history endpoint test passed!");
    println!("   - Returns 200 OK with deletion count");
    println!("   - Clears all history when no filters provided");
    println!("   - Filters by status (complete/failed) correctly");
    println!("   - Filters by timestamp (before) correctly");
    println!("   - Combines both filters (before + status) correctly");
    println!("   - Returns 400 for invalid status filter");
}
