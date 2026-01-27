use super::*;

#[tokio::test]
async fn test_post_servers_test_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing POST /servers/test endpoint...");

    // Setup
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = Arc::new((*downloader.config).clone());
    let app = create_router(downloader.clone(), config);

    println!("  🔍 Test 1: Test server with valid configuration");

    // Create a test server config (will fail to connect but that's OK - we're testing the endpoint)
    let server_config = crate::config::ServerConfig {
        host: "news.example.com".to_string(),
        port: 563,
        tls: true,
        username: Some("testuser".to_string()),
        password: Some("testpass".to_string()),
        connections: 10,
        priority: 0,
        pipeline_depth: 10,
    };

    let request = Request::builder()
        .method("POST")
        .uri("/servers/test")
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&server_config).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "POST /servers/test should return 200 OK"
    );
    println!("    ✓ Returns 200 OK");

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let result: crate::types::ServerTestResult = serde_json::from_slice(&body).unwrap();
    println!("    ✓ Response body is valid ServerTestResult JSON");

    // The server will fail to connect (it doesn't exist), but we should get a proper error result
    assert!(!result.success, "Test should fail for non-existent server");
    assert!(result.error.is_some(), "Should have an error message");
    assert!(
        result.latency.is_some(),
        "Should measure latency even on failure"
    );
    assert!(
        result.capabilities.is_none(),
        "Should not have capabilities on failure"
    );
    println!("    ✓ Failed connection returns proper error result");

    println!("✅ POST /servers/test endpoint test passed!");
    println!("   - Returns 200 OK");
    println!("   - Accepts ServerConfig JSON");
    println!("   - Returns ServerTestResult");
    println!("   - Handles connection failures gracefully");
}

#[tokio::test]
async fn test_get_servers_test_endpoint() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for oneshot()

    println!("🧪 Testing GET /servers/test endpoint...");

    // Setup with a config that has multiple servers
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add some test servers to the config
    let mut config = (*downloader.get_config()).clone();
    config.servers.push(crate::config::ServerConfig {
        host: "news1.example.com".to_string(),
        port: 563,
        tls: true,
        username: Some("user1".to_string()),
        password: Some("pass1".to_string()),
        connections: 10,
        priority: 0,
        pipeline_depth: 10,
    });
    config.servers.push(crate::config::ServerConfig {
        host: "news2.example.com".to_string(),
        port: 119,
        tls: false,
        username: None,
        password: None,
        connections: 5,
        priority: 1,
        pipeline_depth: 10,
    });

    // Create a new downloader with the modified config
    let downloader = Arc::new(crate::UsenetDownloader::new(config.clone()).await.unwrap());

    // Create router
    let config_arc = Arc::new(config);
    let app = create_router(downloader.clone(), config_arc);

    println!("  🔍 Testing GET /servers/test returns results for all servers");

    let request = Request::builder()
        .method("GET")
        .uri("/servers/test")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "GET /servers/test should return 200 OK"
    );
    println!("    ✓ Returns 200 OK");

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let results: Vec<(String, crate::types::ServerTestResult)> =
        serde_json::from_slice(&body).unwrap();
    println!("    ✓ Response body is valid Vec<(String, ServerTestResult)> JSON");

    // Should have results for both servers
    assert_eq!(results.len(), 2, "Should have results for 2 servers");
    println!("    ✓ Returns results for all 2 configured servers");

    // Verify server names
    assert_eq!(
        results[0].0, "news1.example.com",
        "First server should be news1.example.com"
    );
    assert_eq!(
        results[1].0, "news2.example.com",
        "Second server should be news2.example.com"
    );
    println!("    ✓ Server names are correct");

    // Both should fail (non-existent servers) but have proper error info
    for (host, result) in &results {
        assert!(!result.success, "Server {} should fail to connect", host);
        assert!(
            result.error.is_some(),
            "Server {} should have error message",
            host
        );
        assert!(
            result.latency.is_some(),
            "Server {} should measure latency",
            host
        );
    }
    println!("    ✓ All server tests return proper error results");

    println!("✅ GET /servers/test endpoint test passed!");
    println!("   - Returns 200 OK");
    println!("   - Returns array of (hostname, ServerTestResult) tuples");
    println!("   - Tests all configured servers");
    println!("   - Each result includes server name and test result");
}
