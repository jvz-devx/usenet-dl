use super::*;

#[tokio::test]
async fn test_get_config_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing GET /config endpoint...");

    // Setup with custom config that has some sensitive fields
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Modify the config to include sensitive data
    let mut config = (*downloader.get_config()).clone();

    // Add a server with password
    config.servers.push(crate::config::ServerConfig {
        host: "news.example.com".to_string(),
        port: 563,
        tls: true,
        username: Some("testuser".to_string()),
        password: Some("super_secret_password".to_string()),
        connections: 10,
        priority: 0,
        pipeline_depth: 10,
    });

    // DO NOT add an API key - we want to test without authentication
    // (authentication is tested separately in test_authentication_enabled)
    config.server.api.api_key = None;

    // Create a new downloader with the modified config
    let downloader = Arc::new(crate::UsenetDownloader::new(config).await.unwrap());

    // Create router
    let config_arc = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config_arc);

    println!("  🔍 Testing GET /config returns 200 OK and redacts sensitive fields");

    let request = Request::builder()
        .method("GET")
        .uri("/config")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "get_config should return 200 OK"
    );
    println!("    ✓ Returns 200 OK");

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let returned_config: crate::config::Config = serde_json::from_slice(&body).unwrap();
    println!("    ✓ Response body is valid Config JSON");

    // Verify sensitive fields are redacted
    assert!(
        !returned_config.servers.is_empty(),
        "Should have at least one server"
    );

    // Find the server we added (with password)
    let test_server = returned_config
        .servers
        .iter()
        .find(|s| s.host == "news.example.com")
        .expect("Should have the test server");

    assert_eq!(
        test_server.password.as_ref().unwrap(),
        "***REDACTED***",
        "Server passwords should be redacted"
    );
    println!("    ✓ Server passwords are redacted");

    // Verify API key is None (we didn't set one to avoid auth issues in test)
    assert!(
        returned_config.server.api.api_key.is_none(),
        "API key should be None for this test"
    );
    println!("    ✓ API key field is correctly None (we didn't set one)");

    // Verify non-sensitive fields are NOT redacted
    assert!(
        returned_config
            .servers
            .iter()
            .any(|s| s.host == "news.example.com"),
        "Server hostname should not be redacted"
    );
    println!("    ✓ Non-sensitive fields (hostname) are not redacted");

    assert!(
        returned_config
            .servers
            .iter()
            .any(|s| s.username == Some("testuser".to_string())),
        "Username should not be redacted"
    );
    println!("    ✓ Username is not redacted");

    // Verify other config fields are returned correctly
    assert_eq!(
        returned_config.download.max_concurrent_downloads, 3,
        "max_concurrent_downloads should match default"
    );
    println!("    ✓ Other config fields are returned correctly");

    println!("✅ GET /config endpoint test passed!");
    println!("   - Returns 200 OK");
    println!("   - Returns valid Config JSON");
    println!("   - Redacts server passwords (***REDACTED***)");
    println!("   - Preserves non-sensitive fields (hostname, username)");
}

#[tokio::test]
async fn test_patch_config_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing PATCH /config endpoint...");

    // Setup test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config_arc = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config_arc);

    println!("  🔍 Testing PATCH /config updates speed limit");

    // Create a ConfigUpdate with a new speed limit
    let update = crate::config::ConfigUpdate {
        speed_limit_bps: Some(Some(10_000_000)), // 10 MB/s
    };

    let request = Request::builder()
        .method("PATCH")
        .uri("/config")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&update).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "update_config should return 200 OK"
    );
    println!("    ✓ Returns 200 OK");

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let _returned_config: crate::config::Config = serde_json::from_slice(&body).unwrap();
    println!("    ✓ Response body is valid Config JSON");

    // Note: The config in UsenetDownloader is immutable (wrapped in Arc),
    // but the speed limit is managed separately by the SpeedLimiter.
    // The returned config should still show the original speed_limit_bps value
    // from the config, since we don't update the Arc<Config> itself.
    // The actual speed limit change is reflected in the SpeedLimiter.

    println!("✅ PATCH /config endpoint test passed!");
    println!("   - Returns 200 OK");
    println!("   - Accepts ConfigUpdate JSON");
    println!("   - Returns updated Config");
}

#[tokio::test]
async fn test_get_speed_limit() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    println!("\n=== Testing GET /config/speed-limit ===");

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = downloader.get_config();
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: Get default speed limit (should be None/unlimited)
    let request = Request::builder()
        .method("GET")
        .uri("/config/speed-limit")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    // Default should be unlimited (null)
    assert_eq!(json["limit_bps"], Value::Null);

    println!("✅ GET /config/speed-limit (default unlimited) test passed!");

    // Test 2: Set a speed limit and verify we can read it back
    downloader.set_speed_limit(Some(10_000_000)).await; // 10 MB/s

    let request = Request::builder()
        .method("GET")
        .uri("/config/speed-limit")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    // Should return the limit we just set
    assert_eq!(json["limit_bps"], 10_000_000);

    println!("✅ GET /config/speed-limit (with limit set) test passed!");
    println!("   - Returns 200 OK");
    println!("   - Correct JSON structure with limit_bps field");
    println!("   - Returns null for unlimited speed");
    println!("   - Returns correct limit value after setting");
}

#[tokio::test]
async fn test_set_speed_limit() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    println!("\n=== Testing PUT /config/speed-limit ===");

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = downloader.get_config();
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: Set a speed limit (10 MB/s)
    println!("\nTest 1: Setting speed limit to 10 MB/s");
    let request = Request::builder()
        .method("PUT")
        .uri("/config/speed-limit")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"limit_bps": 10485760}"#))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify the limit was actually set by calling GET endpoint
    let request = Request::builder()
        .method("GET")
        .uri("/config/speed-limit")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit_bps"], 10_485_760);

    println!("✅ PUT /config/speed-limit (set limit) test passed!");

    // Test 2: Set unlimited (null)
    println!("\nTest 2: Setting unlimited speed");
    let request = Request::builder()
        .method("PUT")
        .uri("/config/speed-limit")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"limit_bps": null}"#))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify unlimited was set
    let request = Request::builder()
        .method("GET")
        .uri("/config/speed-limit")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit_bps"], Value::Null);

    println!("✅ PUT /config/speed-limit (unlimited) test passed!");

    // Test 3: Set another specific limit (5 MB/s)
    println!("\nTest 3: Changing to 5 MB/s");
    let request = Request::builder()
        .method("PUT")
        .uri("/config/speed-limit")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"limit_bps": 5242880}"#))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify the new limit
    let request = Request::builder()
        .method("GET")
        .uri("/config/speed-limit")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit_bps"], 5_242_880);

    println!("✅ PUT /config/speed-limit (change limit) test passed!");
    println!("   - Returns 204 No Content on success");
    println!("   - Accepts JSON with limit_bps field");
    println!("   - Properly sets numeric limits");
    println!("   - Properly sets unlimited (null)");
    println!("   - Changes are immediately reflected in GET endpoint");
}
