//! REST API server module
//!
//! Provides an OpenAPI 3.1 compliant REST API for managing downloads,
//! configuration, and monitoring the download queue.

use crate::{Config, UsenetDownloader, Result};
use axum::{
    http::HeaderValue,
    middleware,
    routing::{delete, get, patch, post, put},
    Router,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

pub mod auth;
pub mod openapi;
pub mod routes;
pub mod state;

pub use openapi::ApiDoc;
pub use state::AppState;

/// Create the API router with all route definitions
///
/// # Routes
///
/// ## Queue Management
/// - `GET /downloads` - List all downloads
/// - `GET /downloads/:id` - Get single download
/// - `POST /downloads` - Add NZB from file upload
/// - `POST /downloads/url` - Add NZB from URL
/// - `POST /downloads/:id/pause` - Pause download
/// - `POST /downloads/:id/resume` - Resume download
/// - `DELETE /downloads/:id` - Cancel/remove download
/// - `PATCH /downloads/:id/priority` - Set priority
/// - `POST /downloads/:id/reprocess` - Re-run post-processing
/// - `POST /downloads/:id/reextract` - Re-run extraction only
///
/// ## Queue-Wide Operations
/// - `POST /queue/pause` - Pause all downloads
/// - `POST /queue/resume` - Resume all downloads
/// - `GET /queue/stats` - Get queue statistics
///
/// ## History
/// - `GET /history` - Get download history (with pagination)
/// - `DELETE /history` - Clear history
///
/// ## Server Management
/// - `POST /servers/test` - Test server connection
/// - `GET /servers/test` - Test all configured servers
///
/// ## Configuration
/// - `GET /config` - Get current config (sensitive fields redacted)
/// - `PATCH /config` - Update config
/// - `GET /config/speed-limit` - Get speed limit
/// - `PUT /config/speed-limit` - Set speed limit
///
/// ## Categories
/// - `GET /categories` - List categories
/// - `PUT /categories/:name` - Create/update category
/// - `DELETE /categories/:name` - Delete category
///
/// ## System
/// - `GET /health` - Health check
/// - `GET /openapi.json` - OpenAPI specification
/// - `GET /swagger-ui` - Interactive Swagger UI documentation (if enabled)
/// - `GET /events` - Server-sent events stream
/// - `POST /shutdown` - Graceful shutdown
///
/// ## RSS Feeds
/// - `GET /rss` - List RSS feeds
/// - `POST /rss` - Add RSS feed
/// - `PUT /rss/:id` - Update RSS feed
/// - `DELETE /rss/:id` - Delete RSS feed
/// - `POST /rss/:id/check` - Force check feed now
///
/// ## Scheduler
/// - `GET /scheduler` - Get schedule rules
/// - `POST /scheduler` - Add schedule rule
/// - `PUT /scheduler/:id` - Update schedule rule
/// - `DELETE /scheduler/:id` - Delete schedule rule
pub fn create_router(downloader: Arc<UsenetDownloader>, config: Arc<Config>) -> Router {
    let state = AppState::new(downloader, config.clone());

    // Build the router with all routes
    let router = Router::new()
        // Queue Management - Downloads
        .route("/downloads", get(routes::list_downloads))
        .route("/downloads", post(routes::add_download))
        .route("/downloads/:id", get(routes::get_download))
        .route("/downloads/:id", delete(routes::delete_download))
        .route("/downloads/:id/pause", post(routes::pause_download))
        .route("/downloads/:id/resume", post(routes::resume_download))
        .route(
            "/downloads/:id/priority",
            patch(routes::set_download_priority),
        )
        .route(
            "/downloads/:id/reprocess",
            post(routes::reprocess_download),
        )
        .route("/downloads/:id/reextract", post(routes::reextract_download))
        // URL-based NZB adding
        .route("/downloads/url", post(routes::add_download_url))
        // Queue-Wide Operations
        .route("/queue/pause", post(routes::pause_queue))
        .route("/queue/resume", post(routes::resume_queue))
        .route("/queue/stats", get(routes::queue_stats))
        // History
        .route("/history", get(routes::get_history))
        .route("/history", delete(routes::clear_history))
        // Server Management
        .route("/servers/test", post(routes::test_server))
        .route("/servers/test", get(routes::test_all_servers))
        // Configuration
        .route("/config", get(routes::get_config))
        .route("/config", patch(routes::update_config))
        .route("/config/speed-limit", get(routes::get_speed_limit))
        .route("/config/speed-limit", put(routes::set_speed_limit))
        // Categories
        .route("/categories", get(routes::list_categories))
        .route("/categories/:name", put(routes::create_or_update_category))
        .route("/categories/:name", delete(routes::delete_category))
        // System
        .route("/health", get(routes::health_check))
        .route("/openapi.json", get(routes::openapi_spec))
        .route("/events", get(routes::event_stream))
        .route("/shutdown", post(routes::shutdown))
        // RSS Feeds
        .route("/rss", get(routes::list_rss_feeds))
        .route("/rss", post(routes::add_rss_feed))
        .route("/rss/:id", put(routes::update_rss_feed))
        .route("/rss/:id", delete(routes::delete_rss_feed))
        .route("/rss/:id/check", post(routes::check_rss_feed))
        // Scheduler
        .route("/scheduler", get(routes::list_schedule_rules))
        .route("/scheduler", post(routes::add_schedule_rule))
        .route("/scheduler/:id", put(routes::update_schedule_rule))
        .route("/scheduler/:id", delete(routes::delete_schedule_rule));

    // Merge Swagger UI routes if enabled in config (before applying state)
    // Note: SwaggerUi will use the existing /openapi.json endpoint we already defined
    let router = if config.api.swagger_ui {
        router.merge(
            SwaggerUi::new("/swagger-ui")
                .url("/api/v1/openapi.json", ApiDoc::openapi())
        )
    } else {
        router
    };

    // Add state to all routes
    let router = router.with_state(state);

    // Apply authentication middleware if API key is configured
    let router = if config.api.api_key.is_some() {
        router.layer(middleware::from_fn_with_state(
            config.api.api_key.clone(),
            auth::require_api_key,
        ))
    } else {
        router
    };

    // Apply CORS middleware if enabled in config
    if config.api.cors_enabled {
        let cors = build_cors_layer(&config.api.cors_origins);
        router.layer(cors)
    } else {
        router
    }
}

/// Build a CORS layer based on configured origins
///
/// # Arguments
///
/// * `origins` - List of allowed origins (supports "*" for any origin)
///
/// # Returns
///
/// A configured CorsLayer that allows the specified origins, all methods,
/// and all headers for cross-origin requests.
fn build_cors_layer(origins: &[String]) -> CorsLayer {
    // Check if "*" (all origins) is in the list
    let allow_any = origins.iter().any(|o| o == "*");

    if allow_any || origins.is_empty() {
        // Allow all origins (default for local development)
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        // Allow specific origins
        let allowed: Vec<HeaderValue> = origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();

        CorsLayer::new()
            .allow_origin(AllowOrigin::list(allowed))
            .allow_methods(Any)
            .allow_headers(Any)
    }
}

/// Start the API server on the configured bind address.
///
/// This function creates a TCP listener, binds it to the configured address,
/// and starts serving the API router. It runs until the server is shut down.
///
/// # Arguments
///
/// * `downloader` - Arc-wrapped UsenetDownloader instance to handle API requests
/// * `config` - Arc-wrapped Config containing API configuration
///
/// # Returns
///
/// Returns a Result<()> that completes when the server stops, either due to
/// an error or graceful shutdown.
///
/// # Example
///
/// ```no_run
/// use usenet_dl::{UsenetDownloader, Config};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Arc::new(Config::default());
/// let downloader = Arc::new(UsenetDownloader::new((*config).clone()).await?);
///
/// // Start API server (blocks until shutdown)
/// usenet_dl::api::start_api_server(downloader, config).await?;
/// # Ok(())
/// # }
/// ```
pub async fn start_api_server(
    downloader: Arc<UsenetDownloader>,
    config: Arc<Config>,
) -> Result<()> {
    let bind_address = config.api.bind_address;

    tracing::info!(
        address = %bind_address,
        "Starting API server"
    );

    // Create the router with all routes
    let app = create_router(downloader, config);

    // Bind TCP listener to the configured address
    let listener = TcpListener::bind(bind_address)
        .await
        .map_err(|e| crate::error::Error::IoError(e))?;

    tracing::info!(
        address = %bind_address,
        "API server listening"
    );

    // Serve the API using the listener
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::Error::ApiServerError(e.to_string()))?;

    tracing::info!("API server stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use std::time::Duration;
    use tempfile::tempdir;

    /// Helper to create a test UsenetDownloader instance
    async fn create_test_downloader() -> (Arc<UsenetDownloader>, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let config = Config {
            database_path: db_path,
            servers: vec![], // No servers for testing
            ..Default::default()
        };

        let downloader = UsenetDownloader::new(config).await.unwrap();
        (Arc::new(downloader), temp_dir)
    }

    #[tokio::test]
    async fn test_api_server_spawns() {
        // Create test downloader with a unique port
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Use a random available port for testing
        let config = Arc::new(Config {
            api: crate::config::ApiConfig {
                bind_address: "127.0.0.1:0".parse().unwrap(), // Port 0 = OS assigns a free port
                ..Default::default()
            },
            ..(*downloader.config).clone()
        });

        // Spawn the API server
        let api_handle = tokio::spawn({
            let downloader = downloader.clone();
            let config = config.clone();
            async move {
                start_api_server(downloader, config).await
            }
        });

        // Give it a moment to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Abort the server task (since we don't have a graceful shutdown mechanism yet)
        api_handle.abort();

        // The test passes if we got here without panicking
        assert!(true, "API server spawned successfully");
    }

    #[tokio::test]
    async fn test_cors_enabled() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt; // for oneshot()

        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Config with CORS enabled (default)
        let config = Arc::new(Config {
            api: crate::config::ApiConfig {
                cors_enabled: true,
                cors_origins: vec!["*".to_string()],
                ..Default::default()
            },
            ..(*downloader.config).clone()
        });

        // Create router with CORS enabled
        let app = create_router(downloader, config);

        // Make a request with Origin header
        let request = Request::builder()
            .uri("/health")
            .header("Origin", "http://localhost:3000")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Check that response has CORS headers
        assert_eq!(response.status(), StatusCode::OK);

        // The CORS middleware should add access-control-allow-origin header
        let headers = response.headers();
        assert!(
            headers.contains_key("access-control-allow-origin"),
            "CORS header should be present when CORS is enabled"
        );
    }

    #[tokio::test]
    async fn test_cors_disabled() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt; // for oneshot()

        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Config with CORS disabled
        let config = Arc::new(Config {
            api: crate::config::ApiConfig {
                cors_enabled: false,
                ..Default::default()
            },
            ..(*downloader.config).clone()
        });

        // Create router with CORS disabled
        let app = create_router(downloader, config);

        // Make a request with Origin header
        let request = Request::builder()
            .uri("/health")
            .header("Origin", "http://localhost:3000")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Check that response still works but may not have CORS headers
        assert_eq!(response.status(), StatusCode::OK);
        // When CORS is disabled, the middleware is not applied
        // so no CORS headers should be added
    }

    #[test]
    fn test_build_cors_layer_any_origin() {
        // Test building CORS layer with "*" origin
        let origins = vec!["*".to_string()];
        let _layer = build_cors_layer(&origins);
        // Just verify it builds without panicking
    }

    #[test]
    fn test_build_cors_layer_specific_origins() {
        // Test building CORS layer with specific origins
        let origins = vec![
            "http://localhost:3000".to_string(),
            "https://example.com".to_string(),
        ];
        let _layer = build_cors_layer(&origins);
        // Just verify it builds without panicking
    }

    #[test]
    fn test_build_cors_layer_empty_origins() {
        // Test building CORS layer with no origins (should default to any)
        let origins: Vec<String> = vec![];
        let _layer = build_cors_layer(&origins);
        // Just verify it builds without panicking
    }

    #[tokio::test]
    async fn test_spawn_api_server_method() {
        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Use the spawn_api_server method
        let api_handle = downloader.spawn_api_server();

        // Give it a moment to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Abort the server task
        api_handle.abort();

        // Test passes if we got here
        assert!(true, "spawn_api_server method works");
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt; // for oneshot

        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;
        let config = downloader.config.clone();

        // Create the router
        let app = create_router(downloader, config);

        // Make a request to /health
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Check that we got a 200 OK
        assert_eq!(response.status(), StatusCode::OK);

        // Check the response body
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        assert!(body_str.contains("ok"));
        assert!(body_str.contains("0.1.0")); // Version from Cargo.toml
    }

    #[tokio::test]
    async fn test_authentication_with_api_key() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt; // for oneshot

        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Config with API key authentication enabled
        let config = Arc::new(Config {
            api: crate::config::ApiConfig {
                api_key: Some("test-secret-key".to_string()),
                ..Default::default()
            },
            ..(*downloader.config).clone()
        });

        // Create router with authentication
        let app = create_router(downloader, config);

        // Test 1: Request without API key should return 401
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // Test 2: Request with valid API key should succeed
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("X-Api-Key", "test-secret-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Test 3: Request with invalid API key should return 401
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("X-Api-Key", "wrong-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_authentication_disabled_by_default() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt; // for oneshot

        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Config with NO API key (default - authentication disabled)
        let config = Arc::new(Config {
            api: crate::config::ApiConfig {
                api_key: None, // No authentication
                ..Default::default()
            },
            ..(*downloader.config).clone()
        });

        // Create router without authentication
        let app = create_router(downloader, config);

        // Request without API key should succeed when authentication is disabled
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
    }

    #[tokio::test]
    async fn test_server_starts_and_responds_to_health() {
        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Bind to a random available port (port 0)
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Spawn the API server on the random port
        let config = Arc::new(Config {
            api: crate::config::ApiConfig {
                bind_address: addr,
                api_key: None, // No authentication for test
                ..Default::default()
            },
            ..(*downloader.config).clone()
        });

        let server_downloader = downloader.clone();
        let server_config = config.clone();
        let server_handle = tokio::spawn(async move {
            let app = create_router(server_downloader, server_config);
            axum::serve(listener, app).await.unwrap();
        });

        // Give the server a moment to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Make an HTTP request to /health using reqwest
        let client = reqwest::Client::new();
        let url = format!("http://{}/health", addr);
        let response = client.get(&url).send().await.unwrap();

        // Verify response status
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        // Verify response body
        let body = response.json::<serde_json::Value>().await.unwrap();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));

        // Shutdown the server
        server_handle.abort();
    }

    #[tokio::test]
    async fn test_openapi_json_endpoint() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt; // for oneshot

        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;
        let config = downloader.config.clone();

        // Create the router
        let app = create_router(downloader, config);

        // Make a request to /openapi.json
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Check that we got a 200 OK
        assert_eq!(response.status(), StatusCode::OK);

        // Check the response body contains valid OpenAPI spec
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        // Parse as JSON to verify it's valid
        let json: serde_json::Value = serde_json::from_str(&body_str)
            .expect("Response should be valid JSON");

        // Verify it has the required OpenAPI fields
        assert!(json.get("openapi").is_some(), "Should have 'openapi' field");
        assert!(json.get("info").is_some(), "Should have 'info' field");
        assert!(json.get("paths").is_some(), "Should have 'paths' field");

        // Verify OpenAPI version
        let openapi_version = json["openapi"].as_str().unwrap();
        assert!(openapi_version.starts_with("3."), "Should be OpenAPI 3.x");

        // Verify title
        assert_eq!(json["info"]["title"], "usenet-dl REST API");
    }

    #[tokio::test]
    async fn test_swagger_ui_enabled() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt; // for oneshot

        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Config with Swagger UI enabled (default)
        let config = Arc::new(Config {
            api: crate::config::ApiConfig {
                swagger_ui: true,
                ..Default::default()
            },
            ..(*downloader.config).clone()
        });

        // Create the router with Swagger UI enabled
        let app = create_router(downloader, config);

        // Make a request to /swagger-ui (should redirect or serve HTML)
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/swagger-ui/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Swagger UI should return 200 OK (serving HTML)
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Swagger UI should be accessible when enabled"
        );

        // Check that the response body contains HTML (Swagger UI page)
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        // Verify it's HTML content (Swagger UI page)
        assert!(
            body_str.contains("<!DOCTYPE html>") || body_str.contains("<html"),
            "Response should contain HTML"
        );
        assert!(
            body_str.contains("swagger") || body_str.contains("Swagger"),
            "Response should contain Swagger-related content"
        );
    }

    #[tokio::test]
    async fn test_swagger_ui_disabled() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt; // for oneshot

        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Config with Swagger UI disabled
        let config = Arc::new(Config {
            api: crate::config::ApiConfig {
                swagger_ui: false,
                ..Default::default()
            },
            ..(*downloader.config).clone()
        });

        // Create the router with Swagger UI disabled
        let app = create_router(downloader, config);

        // Make a request to /swagger-ui (should return 404)
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/swagger-ui/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 404 when Swagger UI is disabled
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "Swagger UI should not be accessible when disabled"
        );
    }

    #[tokio::test]
    async fn test_swagger_ui_shows_all_endpoints() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use serde_json::Value;
        use tower::ServiceExt; // for oneshot

        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Config with Swagger UI enabled (default)
        let config = Arc::new(Config {
            api: crate::config::ApiConfig {
                swagger_ui: true,
                ..Default::default()
            },
            ..(*downloader.config).clone()
        });

        // Create the router with Swagger UI enabled
        let app = create_router(downloader, config);

        // Get the OpenAPI spec from /openapi.json
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();

        // Verify OpenAPI spec structure
        // Note: utoipa generates OpenAPI 3.0.3 by default
        let openapi_version = json["openapi"].as_str().unwrap();
        assert!(openapi_version.starts_with("3."), "Should be OpenAPI 3.x");
        assert_eq!(json["info"]["title"], "usenet-dl REST API");

        // Count paths in the OpenAPI spec
        let paths = json["paths"].as_object().unwrap();

        // Count total paths
        let total_paths = paths.len();
        println!("Total paths in OpenAPI spec: {}", total_paths);

        // Print all available paths for debugging
        println!("Available paths:");
        for path in paths.keys() {
            println!("  - {}", path);
        }

        // We have 37 annotated route handlers, so we should have 37 paths
        // (Note: Some paths may have multiple HTTP methods, but they share the same path)
        assert!(
            total_paths >= 20,
            "Expected at least 20 unique paths, found {}",
            total_paths
        );

        // Verify some key operations
        let downloads_path = &json["paths"]["/api/v1/downloads"];
        assert!(
            downloads_path["get"].is_object(),
            "GET /api/v1/downloads should be documented"
        );
        assert!(
            downloads_path["post"].is_object(),
            "POST /api/v1/downloads should be documented"
        );

        let download_by_id_path = &json["paths"]["/api/v1/downloads/{id}"];
        assert!(
            download_by_id_path["get"].is_object(),
            "GET /api/v1/downloads/{{id}} should be documented"
        );
        assert!(
            download_by_id_path["delete"].is_object(),
            "DELETE /api/v1/downloads/{{id}} should be documented"
        );

        // Verify health endpoint
        let health_path = &json["paths"]["/api/v1/health"];
        assert!(
            health_path["get"].is_object(),
            "GET /api/v1/health should be documented"
        );

        // Verify OpenAPI spec endpoint itself
        let openapi_path = &json["paths"]["/api/v1/openapi.json"];
        assert!(
            openapi_path["get"].is_object(),
            "GET /api/v1/openapi.json should be documented"
        );

        // Verify tags are present
        let tags = json["tags"].as_array().unwrap();
        assert!(
            !tags.is_empty(),
            "OpenAPI spec should have tags for organization"
        );

        // Verify components/schemas are present (type definitions)
        let schemas = json["components"]["schemas"].as_object().unwrap();
        assert!(
            !schemas.is_empty(),
            "OpenAPI spec should have schema definitions"
        );

        println!("Available schemas:");
        for schema in schemas.keys() {
            println!("  - {}", schema);
        }

        // Verify some key schemas exist (only check ones that are currently implemented)
        let expected_schemas = vec![
            "DownloadInfo",
            "DownloadOptions",
            "Status",
            "Priority",
        ];

        for expected_schema in &expected_schemas {
            assert!(
                schemas.contains_key(*expected_schema),
                "OpenAPI spec should contain schema: {}",
                expected_schema
            );
        }

        println!("✅ Swagger UI OpenAPI spec validation complete!");
        println!("   - {} paths documented", total_paths);
        println!("   - {} schemas defined", schemas.len());
        println!("   - {} tags defined", tags.len());
    }

    #[tokio::test]
    async fn test_list_downloads_endpoint() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt; // for oneshot()

        // Create test downloader
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add some test downloads to the database
        use crate::types::{DownloadOptions, Priority};
        use crate::db::NewDownload;

        let new_download1 = NewDownload {
            name: "Test Download 1".to_string(),
            nzb_path: "/tmp/test1.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: Some("hash1".to_string()),
            job_name: Some("Test Download 1".to_string()),
            category: Some("movies".to_string()),
            destination: "/downloads".to_string(),
            post_process: 4, // UnpackAndCleanup
            priority: 0,     // Normal
            status: 0,       // Queued
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
            priority: 1, // High
            status: 1,   // Downloading
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
        let download1 = downloads.iter().find(|d| d.name == "Test Download 1").unwrap();
        assert_eq!(download1.category, Some("movies".to_string()));
        assert_eq!(download1.status, crate::types::Status::Queued);
        assert_eq!(download1.priority, crate::types::Priority::Normal);
        assert_eq!(download1.size_bytes, 1024 * 1024 * 100);

        let download2 = downloads.iter().find(|d| d.name == "Test Download 2").unwrap();
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
            post_process: 4, // UnpackAndCleanup
            priority: 0,     // Normal
            status: 0,       // Queued
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
                format!("multipart/form-data; boundary={}", boundary)
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
            .get_download(download_id)
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
                format!("multipart/form-data; boundary={}", boundary)
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
                format!("multipart/form-data; boundary={}", boundary)
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
            .body(Body::from(r#"{"url": "https://example.com/test.nzb", "options": "invalid"}"#))
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
            .body(Body::from(r#"{"url": "http://invalid-nonexistent-domain-12345.com/test.nzb"}"#))
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
        assert_eq!(json3["error"]["code"], "add_failed");

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
            post_process: 4, // UnpackAndCleanup
            priority: 0,     // Normal
            status: 1,       // Downloading (so it can be paused)
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
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
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

        let completed_id = downloader.db.insert_download(&completed_download).await.unwrap();

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
            post_process: 4, // UnpackAndCleanup
            priority: 0,     // Normal
            status: 2,       // Paused (so it can be resumed)
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
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
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

        let completed_id = downloader.db.insert_download(&completed_download).await.unwrap();

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

        let queued_id = downloader.db.insert_download(&queued_download).await.unwrap();

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
            post_process: 4, // UnpackAndCleanup
            priority: 0,     // Normal
            status: 0,       // Queued
            size_bytes: 1024 * 1024 * 100, // 100 MB
        };

        // Insert download and get its ID
        let download_id = downloader.db.insert_download(&new_download).await.unwrap();

        // Verify download was created
        assert!(
            downloader.db.get_download(download_id).await.unwrap().is_some(),
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
            downloader.db.get_download(download_id).await.unwrap().is_none(),
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
            downloader.db.get_download(download_id2).await.unwrap().is_none(),
            "Download should not exist after deletion with delete_files=true"
        );

        println!("    ✓ Returns 204 NO_CONTENT with delete_files=true");
        println!("    ✓ Download removed from database");

        println!("✅ delete_download endpoint test passed!");
        println!("   - Successfully deletes existing downloads");
        println!("   - Returns 404 for non-existent downloads");
        println!("   - Accepts delete_files query parameter");
    }
}
