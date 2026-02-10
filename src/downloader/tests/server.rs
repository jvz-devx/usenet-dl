use super::*;

#[tokio::test]
async fn test_server_health_check_invalid_server() {
    // Test: test_server should return error for non-existent server
    println!("Testing server health check with invalid server...");

    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download.download_dir = temp_dir.path().to_path_buf();

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
    assert!(
        !result.success,
        "Expected test_server to fail for invalid server"
    );
    assert!(
        result.error.is_some(),
        "Expected error message for failed connection"
    );
    assert!(
        result.latency.is_some(),
        "Expected latency even for failed connection"
    );
    assert!(
        result.capabilities.is_none(),
        "Expected no capabilities for failed connection"
    );

    println!("test_server correctly reports failure for invalid server");
    println!("  Error: {:?}", result.error.unwrap());
}

#[tokio::test]
async fn test_server_health_check_result_structure() {
    // Test: ServerTestResult has correct structure
    println!("Testing ServerTestResult structure...");

    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download.download_dir = temp_dir.path().to_path_buf();

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

    println!("ServerTestResult correctly serializes/deserializes");
    println!("  JSON: {}", json);
}

#[tokio::test]
async fn test_all_servers_empty_config() {
    // Test: test_all_servers with no configured servers
    println!("Testing test_all_servers with empty configuration...");

    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download.download_dir = temp_dir.path().to_path_buf();
    config.servers = vec![]; // No servers configured

    let downloader = UsenetDownloader::new(config).await.unwrap();

    let results = downloader.test_all_servers().await;

    assert!(
        results.is_empty(),
        "Expected empty results for empty server list"
    );

    println!("test_all_servers correctly handles empty server list");
}

#[tokio::test]
async fn test_all_servers_multiple_servers() {
    // Test: test_all_servers returns results for all servers
    println!("Testing test_all_servers with multiple servers...");

    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download.download_dir = temp_dir.path().to_path_buf();

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

    println!("test_all_servers correctly tests all configured servers");
    println!("  Tested {} servers", results.len());
}

#[tokio::test]
async fn test_server_capabilities_structure() {
    // Test: ServerCapabilities structure is correct
    println!("Testing ServerCapabilities structure...");

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

    println!("ServerCapabilities correctly serializes/deserializes");
    println!("  JSON: {}", json);
}
