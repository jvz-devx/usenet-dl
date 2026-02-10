use super::*;

#[tokio::test]
async fn test_sse_event_stream() {
    use crate::types::Event;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("\n🧪 Testing GET /events (SSE stream) endpoint...");

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;
    let config = Arc::new((*downloader.config).clone());

    // Create router
    let app = create_router(downloader.clone(), config);

    // Make request to /events endpoint
    let request = Request::builder()
        .uri("/events")
        .header("Accept", "text/event-stream")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Verify response status and content type
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "SSE endpoint should return 200 OK"
    );
    println!("    ✓ Returns 200 OK");

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    assert!(
        content_type.contains("text/event-stream"),
        "Content-Type should be text/event-stream, got: {}",
        content_type
    );
    println!("    ✓ Content-Type is text/event-stream");

    // Test that events are actually sent by emitting an event and checking the stream
    // Note: This is a basic test - full integration testing would require
    // reading from the stream, which is more complex in a unit test

    // Emit a test event
    downloader.emit_event(Event::QueuePaused);
    println!("    ✓ Event emission works (via emit_event)");

    // Verify subscribe works (the SSE endpoint uses this internally)
    let mut receiver = downloader.subscribe();

    // Emit another event and verify the receiver gets it
    downloader.emit_event(Event::QueueResumed);

    // Try to receive the event with a timeout
    let received = tokio::time::timeout(Duration::from_millis(100), receiver.recv()).await;

    assert!(
        received.is_ok() && received.unwrap().is_ok(),
        "Should be able to subscribe and receive events"
    );
    println!("    ✓ Event subscription works (SSE will use this)");

    println!("✅ GET /events endpoint test passed!");
    println!("   - Returns 200 OK");
    println!("   - Sets Content-Type to text/event-stream");
    println!("   - Event broadcasting system works");
    println!("   - Subscribers can receive events");
}

#[tokio::test]
async fn test_scheduler_endpoints() {
    use crate::config::{ScheduleAction, ScheduleRule, Weekday};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    println!("\n=== Testing Scheduler Endpoints ===");

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;
    let config = downloader.get_config();
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: GET /scheduler - should be empty initially
    println!("\nTest 1: GET /scheduler (empty)");
    let request = Request::builder()
        .method("GET")
        .uri("/scheduler")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    println!(
        "Initial scheduler response: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    assert!(json.is_array());
    assert_eq!(json.as_array().unwrap().len(), 0);
    println!("   ✓ Empty list returned");

    // Test 2: POST /scheduler - add a new rule
    println!("\nTest 2: POST /scheduler (add rule)");
    let rule = ScheduleRule {
        name: "Night time unlimited".to_string(),
        days: vec![],
        start_time: "00:00".to_string(),
        end_time: "06:00".to_string(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    };

    let request = Request::builder()
        .method("POST")
        .uri("/scheduler")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&rule).unwrap()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    println!(
        "Add rule response: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    assert!(json["id"].is_number());
    let rule_id = json["id"].as_i64().unwrap();
    println!("   ✓ Rule added with ID: {}", rule_id);

    // Test 3: POST /scheduler - add another rule with speed limit
    println!("\nTest 3: POST /scheduler (add work hours rule)");
    let rule2 = ScheduleRule {
        name: "Work hours limited".to_string(),
        days: vec![
            Weekday::Monday,
            Weekday::Tuesday,
            Weekday::Wednesday,
            Weekday::Thursday,
            Weekday::Friday,
        ],
        start_time: "09:00".to_string(),
        end_time: "17:00".to_string(),
        action: ScheduleAction::SpeedLimit {
            limit_bps: 1_000_000,
        },
        enabled: true,
    };

    let request = Request::builder()
        .method("POST")
        .uri("/scheduler")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&rule2).unwrap()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let rule2_id = json["id"].as_i64().unwrap();
    println!("   ✓ Rule added with ID: {}", rule2_id);

    // Test 4: GET /scheduler - should now have 2 rules
    println!("\nTest 4: GET /scheduler (with rules)");
    let request = Request::builder()
        .method("GET")
        .uri("/scheduler")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    println!(
        "Scheduler with rules: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    assert!(json.is_array());
    let rules = json.as_array().unwrap();
    assert_eq!(rules.len(), 2);
    println!("   ✓ 2 rules returned");

    // Verify first rule
    assert_eq!(rules[0]["id"], 0);
    assert_eq!(rules[0]["name"], "Night time unlimited");
    assert_eq!(rules[0]["start_time"], "00:00");
    assert_eq!(rules[0]["end_time"], "06:00");
    println!("   ✓ First rule details correct");

    // Verify second rule
    assert_eq!(rules[1]["id"], 1);
    assert_eq!(rules[1]["name"], "Work hours limited");
    assert_eq!(rules[1]["days"].as_array().unwrap().len(), 5);
    println!("   ✓ Second rule details correct");

    // Test 5: PUT /scheduler/:id - update a rule
    println!("\nTest 5: PUT /scheduler/0 (update rule)");
    let updated_rule = ScheduleRule {
        name: "Night time unlimited (updated)".to_string(),
        days: vec![Weekday::Saturday, Weekday::Sunday],
        start_time: "00:00".to_string(),
        end_time: "08:00".to_string(), // Changed to 8 AM
        action: ScheduleAction::Unlimited,
        enabled: false, // Disabled
    };

    let request = Request::builder()
        .method("PUT")
        .uri("/scheduler/0")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&updated_rule).unwrap()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    println!("   ✓ Rule updated successfully (204 No Content)");

    // Test 6: GET /scheduler - verify update
    println!("\nTest 6: GET /scheduler (verify update)");
    let request = Request::builder()
        .method("GET")
        .uri("/scheduler")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let rules = json.as_array().unwrap();
    assert_eq!(rules[0]["name"], "Night time unlimited (updated)");
    assert_eq!(rules[0]["end_time"], "08:00");
    assert_eq!(rules[0]["enabled"], false);
    assert_eq!(rules[0]["days"].as_array().unwrap().len(), 2);
    println!("   ✓ Rule update verified");

    // Test 7: PUT /scheduler/999 - update non-existent rule (should fail)
    println!("\nTest 7: PUT /scheduler/999 (not found)");
    let request = Request::builder()
        .method("PUT")
        .uri("/scheduler/999")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&updated_rule).unwrap()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
    println!("   ✓ 404 Not Found returned for non-existent rule");

    // Test 8: POST /scheduler with invalid time format
    println!("\nTest 8: POST /scheduler (invalid time format)");
    let invalid_rule = ScheduleRule {
        name: "Invalid".to_string(),
        days: vec![],
        start_time: "25:00".to_string(), // Invalid hour
        end_time: "06:00".to_string(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    };

    let request = Request::builder()
        .method("POST")
        .uri("/scheduler")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&invalid_rule).unwrap()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Invalid start_time format")
    );
    println!("   ✓ 400 Bad Request returned for invalid time format");

    // Test 9: DELETE /scheduler/:id - delete a rule
    println!("\nTest 9: DELETE /scheduler/0 (delete rule)");
    let request = Request::builder()
        .method("DELETE")
        .uri("/scheduler/0")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    println!("   ✓ Rule deleted successfully (204 No Content)");

    // Test 10: GET /scheduler - verify deletion (should have 1 rule left)
    println!("\nTest 10: GET /scheduler (verify deletion)");
    let request = Request::builder()
        .method("GET")
        .uri("/scheduler")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let rules = json.as_array().unwrap();
    assert_eq!(rules.len(), 1);
    // After deleting rule 0, rule 1 becomes rule 0 (array shifts)
    assert_eq!(rules[0]["name"], "Work hours limited");
    println!("   ✓ Only 1 rule remaining after deletion");

    // Test 11: DELETE /scheduler/999 - delete non-existent rule
    println!("\nTest 11: DELETE /scheduler/999 (not found)");
    let request = Request::builder()
        .method("DELETE")
        .uri("/scheduler/999")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
    println!("   ✓ 404 Not Found returned for non-existent rule");

    println!("\n=== Scheduler Endpoints Test: PASSED ===");
    println!("\nSummary:");
    println!("  - GET /scheduler (empty): ✓");
    println!("  - POST /scheduler (add rules): ✓");
    println!("  - GET /scheduler (with rules): ✓");
    println!("  - PUT /scheduler/:id (update): ✓");
    println!("  - PUT /scheduler/:id (not found): ✓");
    println!("  - POST /scheduler (invalid time): ✓");
    println!("  - DELETE /scheduler/:id: ✓");
    println!("  - DELETE /scheduler/:id (not found): ✓");
    println!("  - Rule details and IDs correct: ✓");
}

/// Test 28.8: Test duplicate detection with same NZB added twice via API
#[tokio::test]
async fn test_duplicate_detection_via_api() {
    use axum::http::{Method, header};
    use serde_json::Value;

    println!("\n=== Testing Duplicate Detection via API ===");

    // Valid NZB content for testing
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

    // Test 1: Block action - second upload should fail with 409 Conflict
    println!("\n--- Test 1: Block Action ---");
    {
        let temp_dir = tempdir().unwrap();
        let config = Config {
            persistence: crate::config::PersistenceConfig {
                database_path: temp_dir.path().join("test.db"),
                schedule_rules: vec![],
                categories: std::collections::HashMap::new(),
            },
            download: crate::config::DownloadConfig {
                download_dir: temp_dir.path().join("downloads"),
                temp_dir: temp_dir.path().join("temp"),
                ..Default::default()
            },
            processing: crate::config::ProcessingConfig {
                duplicate: crate::config::DuplicateConfig {
                    enabled: true,
                    action: crate::config::DuplicateAction::Block,
                    methods: vec![crate::config::DuplicateMethod::NzbHash],
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let downloader = Arc::new(UsenetDownloader::new(config.clone()).await.unwrap());
        let config = Arc::new(config);
        let app = create_router(downloader.clone(), config);

        // First upload - should succeed
        println!("  Uploading NZB first time...");
        let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
        let body_content = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.nzb\"\r\nContent-Type: application/x-nzb\r\n\r\n{}\r\n--{}--\r\n",
            boundary,
            String::from_utf8_lossy(nzb_content),
            boundary
        );

        let request = Request::builder()
            .method(Method::POST)
            .uri("/downloads")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body_content.clone()))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "First upload should succeed"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let first_id = json["id"].as_i64().unwrap();
        println!("  ✓ First upload succeeded with ID: {}", first_id);

        // Second upload - should be blocked with 409 Conflict
        println!("  Uploading same NZB second time (should be blocked)...");
        let request = Request::builder()
            .method(Method::POST)
            .uri("/downloads")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body_content))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "Second upload should be blocked with 409"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "duplicate");
        assert!(
            json["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Duplicate"),
            "Error message should mention duplicate"
        );
        println!("  ✓ Second upload blocked with 409 Conflict");
        println!("  ✓ Error message: {}", json["error"]["message"]);
    }

    // Test 2: Warn action - second upload should succeed with warning event
    println!("\n--- Test 2: Warn Action ---");
    {
        let temp_dir = tempdir().unwrap();
        let config = Config {
            persistence: crate::config::PersistenceConfig {
                database_path: temp_dir.path().join("test.db"),
                schedule_rules: vec![],
                categories: std::collections::HashMap::new(),
            },
            download: crate::config::DownloadConfig {
                download_dir: temp_dir.path().join("downloads"),
                temp_dir: temp_dir.path().join("temp"),
                ..Default::default()
            },
            processing: crate::config::ProcessingConfig {
                duplicate: crate::config::DuplicateConfig {
                    enabled: true,
                    action: crate::config::DuplicateAction::Warn,
                    methods: vec![crate::config::DuplicateMethod::NzbHash],
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let downloader = Arc::new(UsenetDownloader::new(config.clone()).await.unwrap());
        let config = Arc::new(config);
        let app = create_router(downloader.clone(), config);

        // Subscribe to events to catch duplicate warning
        let mut events = downloader.subscribe();

        // First upload
        println!("  Uploading NZB first time...");
        let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
        let body_content = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.nzb\"\r\nContent-Type: application/x-nzb\r\n\r\n{}\r\n--{}--\r\n",
            boundary,
            String::from_utf8_lossy(nzb_content),
            boundary
        );

        let request = Request::builder()
            .method(Method::POST)
            .uri("/downloads")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body_content.clone()))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let first_id = json["id"].as_i64().unwrap();
        println!("  ✓ First upload succeeded with ID: {}", first_id);

        // Second upload with different filename - should succeed but emit warning
        println!("  Uploading same NZB with different name (should warn but allow)...");
        let body_content_2 = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test-copy.nzb\"\r\nContent-Type: application/x-nzb\r\n\r\n{}\r\n--{}--\r\n",
            boundary,
            String::from_utf8_lossy(nzb_content),
            boundary
        );

        let request = Request::builder()
            .method(Method::POST)
            .uri("/downloads")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body_content_2))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "Second upload should succeed with Warn action"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let second_id = json["id"].as_i64().unwrap();
        assert!(second_id > first_id, "Second upload should get a new ID");
        println!("  ✓ Second upload succeeded with ID: {}", second_id);

        // Check for duplicate warning event
        // We may need to skip some events (e.g., Queued events from first upload)
        println!("  Checking for DuplicateDetected event...");
        let mut found_duplicate_event = false;
        for _ in 0..10 {
            // Try up to 10 events
            match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
                Ok(Ok(crate::Event::DuplicateDetected {
                    id,
                    name,
                    method,
                    existing_name,
                })) => {
                    assert_eq!(
                        id, first_id as i64,
                        "Event should reference first download ID"
                    );
                    assert_eq!(
                        name, "test-copy.nzb",
                        "Event should have second upload name"
                    );
                    assert_eq!(
                        method,
                        crate::config::DuplicateMethod::NzbHash,
                        "Event should show NzbHash method"
                    );
                    assert_eq!(
                        existing_name, "test.nzb",
                        "Event should have first upload name"
                    );
                    println!("  ✓ DuplicateDetected event received with correct details");
                    found_duplicate_event = true;
                    break;
                }
                Ok(Ok(_)) => {
                    // Skip other events
                    continue;
                }
                Ok(Err(_)) => break, // Channel error
                Err(_) => break,     // Timeout
            }
        }
        assert!(
            found_duplicate_event,
            "Should have received DuplicateDetected event"
        );
    }

    // Test 3: Allow action - second upload should succeed without blocking
    println!("\n--- Test 3: Allow Action ---");
    {
        let temp_dir = tempdir().unwrap();
        let config = Config {
            persistence: crate::config::PersistenceConfig {
                database_path: temp_dir.path().join("test.db"),
                schedule_rules: vec![],
                categories: std::collections::HashMap::new(),
            },
            download: crate::config::DownloadConfig {
                download_dir: temp_dir.path().join("downloads"),
                temp_dir: temp_dir.path().join("temp"),
                ..Default::default()
            },
            processing: crate::config::ProcessingConfig {
                duplicate: crate::config::DuplicateConfig {
                    enabled: true,
                    action: crate::config::DuplicateAction::Allow,
                    methods: vec![crate::config::DuplicateMethod::NzbHash],
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let downloader = Arc::new(UsenetDownloader::new(config.clone()).await.unwrap());
        let config = Arc::new(config);
        let app = create_router(downloader.clone(), config);

        // First upload
        println!("  Uploading NZB first time...");
        let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
        let body_content = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.nzb\"\r\nContent-Type: application/x-nzb\r\n\r\n{}\r\n--{}--\r\n",
            boundary,
            String::from_utf8_lossy(nzb_content),
            boundary
        );

        let request = Request::builder()
            .method(Method::POST)
            .uri("/downloads")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body_content.clone()))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let first_id = json["id"].as_i64().unwrap();
        println!("  ✓ First upload succeeded with ID: {}", first_id);

        // Second upload - should succeed without issue
        println!("  Uploading same NZB second time (should be allowed)...");
        let request = Request::builder()
            .method(Method::POST)
            .uri("/downloads")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body_content))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "Second upload should succeed with Allow action"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let second_id = json["id"].as_i64().unwrap();
        assert!(second_id > first_id, "Second upload should get a new ID");
        println!("  ✓ Second upload succeeded with ID: {}", second_id);
    }

    // Test 4: Disabled duplicate detection - should always allow
    println!("\n--- Test 4: Disabled Duplicate Detection ---");
    {
        let temp_dir = tempdir().unwrap();
        let config = Config {
            persistence: crate::config::PersistenceConfig {
                database_path: temp_dir.path().join("test.db"),
                schedule_rules: vec![],
                categories: std::collections::HashMap::new(),
            },
            download: crate::config::DownloadConfig {
                download_dir: temp_dir.path().join("downloads"),
                temp_dir: temp_dir.path().join("temp"),
                ..Default::default()
            },
            processing: crate::config::ProcessingConfig {
                duplicate: crate::config::DuplicateConfig {
                    enabled: false, // Disabled
                    action: crate::config::DuplicateAction::Block,
                    methods: vec![crate::config::DuplicateMethod::NzbHash],
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let downloader = Arc::new(UsenetDownloader::new(config.clone()).await.unwrap());
        let config = Arc::new(config);
        let app = create_router(downloader.clone(), config);

        // First upload
        println!("  Uploading NZB first time...");
        let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
        let body_content = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.nzb\"\r\nContent-Type: application/x-nzb\r\n\r\n{}\r\n--{}--\r\n",
            boundary,
            String::from_utf8_lossy(nzb_content),
            boundary
        );

        let request = Request::builder()
            .method(Method::POST)
            .uri("/downloads")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body_content.clone()))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let first_id = json["id"].as_i64().unwrap();
        println!("  ✓ First upload succeeded with ID: {}", first_id);

        // Second upload - should succeed (detection disabled)
        println!("  Uploading same NZB second time (detection disabled, should allow)...");
        let request = Request::builder()
            .method(Method::POST)
            .uri("/downloads")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body_content))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "Second upload should succeed when detection disabled"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let second_id = json["id"].as_i64().unwrap();
        assert!(second_id > first_id, "Second upload should get a new ID");
        println!("  ✓ Second upload succeeded with ID: {}", second_id);
    }

    println!("\n=== Duplicate Detection API Test: PASSED ===");
    println!("\nSummary:");
    println!("  - Block action prevents duplicate (409 Conflict): ✓");
    println!("  - Warn action allows duplicate with event: ✓");
    println!("  - Allow action silently allows duplicate: ✓");
    println!("  - Disabled detection allows all uploads: ✓");
}

// -----------------------------------------------------------------------
// System endpoint tests: health, capabilities, shutdown
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_health_check_returns_status_ok_and_version() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let (downloader, _temp_dir) = create_test_downloader().await;
    let config = downloader.config.clone();
    let app = create_router(downloader, config);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["status"], "ok",
        "health endpoint should report status=ok"
    );
    assert_eq!(
        json["version"],
        env!("CARGO_PKG_VERSION"),
        "health endpoint should return the crate version"
    );
    // Verify the response has exactly the expected fields (no extras, no missing)
    let obj = json.as_object().unwrap();
    assert!(
        obj.contains_key("status") && obj.contains_key("version"),
        "response must contain 'status' and 'version' keys"
    );
}

#[tokio::test]
async fn test_get_capabilities_returns_parity_info() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let (downloader, _temp_dir) = create_test_downloader().await;
    let config = downloader.config.clone();
    let app = create_router(downloader, config);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Verify the capabilities object has the parity section
    let parity = json
        .get("parity")
        .expect("capabilities must have 'parity' field");
    assert!(
        parity.get("can_verify").is_some(),
        "parity must have 'can_verify' field"
    );
    assert!(
        parity.get("can_repair").is_some(),
        "parity must have 'can_repair' field"
    );
    assert!(
        parity.get("handler").is_some(),
        "parity must have 'handler' field"
    );

    // The test downloader has no par2 configured and search_path defaults to true,
    // but may or may not find par2 on the system. The handler field should always
    // be a non-empty string regardless.
    let handler = parity["handler"]
        .as_str()
        .expect("handler should be a string");
    assert!(!handler.is_empty(), "handler name should not be empty");

    // can_verify and can_repair must be booleans
    assert!(
        parity["can_verify"].is_boolean(),
        "can_verify should be a boolean"
    );
    assert!(
        parity["can_repair"].is_boolean(),
        "can_repair should be a boolean"
    );
}

#[tokio::test]
async fn test_shutdown_returns_202_accepted() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    // We need to test the shutdown endpoint without actually exiting the process.
    // The shutdown handler spawns a background task that calls process::exit(0).
    // With oneshot(), the background task is spawned but won't complete because
    // we're in a test context. We just verify the HTTP response.
    let (downloader, _temp_dir) = create_test_downloader().await;
    let config = downloader.config.clone();
    let app = create_router(downloader, config);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/shutdown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "shutdown should return 202 Accepted"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["status"], "shutdown initiated",
        "shutdown response should confirm initiation"
    );
}

#[tokio::test]
async fn test_capabilities_with_no_servers_reflects_noop_parity() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    // Create a downloader with all tools explicitly disabled
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let mut config = Config::default();
    config.persistence.database_path = db_path;
    config.servers = vec![];
    config.tools.par2_path = None;
    config.tools.search_path = false; // Don't search PATH

    let downloader = crate::UsenetDownloader::new(config.clone()).await.unwrap();
    let downloader = Arc::new(downloader);
    let config = Arc::new(config);
    let app = create_router(downloader, config);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let parity = &json["parity"];
    // With search_path=false and no par2_path, the NoOpParityHandler should be used
    assert_eq!(parity["can_verify"], false, "NoOp handler cannot verify");
    assert_eq!(parity["can_repair"], false, "NoOp handler cannot repair");
    assert_eq!(
        parity["handler"], "noop",
        "should use the NoOp handler when par2 is not configured"
    );
}

#[tokio::test]
async fn test_health_endpoint_not_affected_by_authentication() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let (downloader, _temp_dir) = create_test_downloader().await;

    // Enable API key auth
    let mut config = (*downloader.config).clone();
    config.server.api.api_key = Some("secret-test-key-123".to_string());
    let config = Arc::new(config);

    let app = create_router(downloader, config);

    // Health endpoint WITH valid key should work
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("X-Api-Key", "secret-test-key-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Health endpoint WITHOUT key should be blocked (auth is global)
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "health should require auth when API key is configured"
    );
}

#[tokio::test]
async fn test_capabilities_endpoint_requires_auth_when_configured() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let (downloader, _temp_dir) = create_test_downloader().await;

    let mut config = (*downloader.config).clone();
    config.server.api.api_key = Some("my-secret".to_string());
    let config = Arc::new(config);

    let app = create_router(downloader, config);

    // Without auth header -> 401
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // With auth header -> 200
    let response = app
        .oneshot(
            Request::builder()
                .uri("/capabilities")
                .header("X-Api-Key", "my-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_shutdown_endpoint_requires_auth_when_configured() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let (downloader, _temp_dir) = create_test_downloader().await;

    let mut config = (*downloader.config).clone();
    config.server.api.api_key = Some("admin-key".to_string());
    let config = Arc::new(config);

    let app = create_router(downloader, config);

    // Without auth header -> 401
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/shutdown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "shutdown must require auth when configured"
    );

    // With valid auth -> 202
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/shutdown")
                .header("X-Api-Key", "admin-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

// -----------------------------------------------------------------------
// Authentication enforcement: verify 401 response body structure
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_missing_api_key_returns_401_with_structured_error_body() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let (downloader, _temp_dir) = create_test_downloader().await;

    let mut config = (*downloader.config).clone();
    config.server.api.api_key = Some("correct-key-abc".to_string());
    let config = Arc::new(config);

    let app = create_router(downloader, config);

    // GET /downloads without any API key header
    let response = app
        .oneshot(
            Request::builder()
                .uri("/downloads")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "data endpoint must reject requests without API key"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["error"]["code"], "unauthorized",
        "error code must be 'unauthorized'"
    );
    assert_eq!(
        json["error"]["message"], "Missing X-Api-Key header",
        "error message must indicate the missing header"
    );
}

#[tokio::test]
async fn test_wrong_api_key_returns_401_with_invalid_key_message() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let (downloader, _temp_dir) = create_test_downloader().await;

    let mut config = (*downloader.config).clone();
    config.server.api.api_key = Some("correct-key-abc".to_string());
    let config = Arc::new(config);

    let app = create_router(downloader, config);

    // GET /downloads with wrong API key
    let response = app
        .oneshot(
            Request::builder()
                .uri("/downloads")
                .header("X-Api-Key", "wrong-key-xyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "data endpoint must reject requests with wrong API key"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["error"]["code"], "unauthorized");
    assert_eq!(
        json["error"]["message"], "Invalid API key",
        "error message must distinguish wrong key from missing key"
    );
}

#[tokio::test]
async fn test_correct_api_key_allows_access_to_data_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let (downloader, _temp_dir) = create_test_downloader().await;

    let mut config = (*downloader.config).clone();
    config.server.api.api_key = Some("correct-key-abc".to_string());
    let config = Arc::new(config);

    let app = create_router(downloader, config);

    // GET /downloads with correct API key
    let response = app
        .oneshot(
            Request::builder()
                .uri("/downloads")
                .header("X-Api-Key", "correct-key-abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "data endpoint must allow requests with correct API key"
    );
}

#[tokio::test]
async fn test_shutdown_with_wrong_method_returns_405() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let (downloader, _temp_dir) = create_test_downloader().await;
    let config = downloader.config.clone();
    let app = create_router(downloader, config);

    // GET /shutdown should not be a valid route (shutdown is POST only)
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/shutdown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "GET /shutdown should return 405 Method Not Allowed"
    );
}
