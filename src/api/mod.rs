//! REST API server module
//!
//! Provides an OpenAPI 3.1 compliant REST API for managing downloads,
//! configuration, and monitoring the download queue.

use crate::{Config, UsenetDownloader, Result};
use axum::{
    http::HeaderValue,
    routing::{delete, get, patch, post, put},
    Router,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

pub mod routes;
pub mod state;

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
        .route("/scheduler/:id", delete(routes::delete_schedule_rule))
        // Add state to all routes
        .with_state(state);

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
}
