//! Test configuration helpers for loading .env credentials and creating test downloaders

use std::sync::Arc;
use tempfile::TempDir;
use usenet_dl::config::{DownloadConfig, PersistenceConfig};
use usenet_dl::{Config, ServerConfig, UsenetDownloader};

/// Error type for test configuration
#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Config error: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Load NNTP server configuration from environment variables
///
/// Required environment variables:
/// - `NNTP_HOST` - Server hostname
/// - `NNTP_USERNAME` - Authentication username
/// - `NNTP_PASSWORD` - Authentication password
///
/// Optional environment variables:
/// - `NNTP_PORT_SSL` - TLS port (default: 563)
/// - `NNTP_CONNECTIONS` - Number of connections (default: 4)
pub fn load_server_config() -> Result<ServerConfig, ConfigError> {
    dotenvy::dotenv().ok();

    let host = std::env::var("NNTP_HOST")
        .map_err(|_| ConfigError("NNTP_HOST not set in environment".to_string()))?;

    let port: u16 = std::env::var("NNTP_PORT_SSL")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(563);

    let username = std::env::var("NNTP_USERNAME")
        .map_err(|_| ConfigError("NNTP_USERNAME not set in environment".to_string()))?;

    let password = std::env::var("NNTP_PASSWORD")
        .map_err(|_| ConfigError("NNTP_PASSWORD not set in environment".to_string()))?;

    let connections: usize = std::env::var("NNTP_CONNECTIONS")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(4);

    Ok(ServerConfig {
        host,
        port,
        tls: true,
        username: Some(username),
        password: Some(password),
        connections,
        priority: 0,
        pipeline_depth: 10,
    })
}

/// Load server config with invalid password for auth failure tests
pub fn load_server_config_bad_password() -> Result<ServerConfig, ConfigError> {
    let mut config = load_server_config()?;
    config.password = Some("invalid_password_12345".to_string());
    Ok(config)
}

/// Load server config with invalid username for auth failure tests
pub fn load_server_config_bad_username() -> Result<ServerConfig, ConfigError> {
    let mut config = load_server_config()?;
    config.username = Some("invalid_user_12345".to_string());
    Ok(config)
}

/// Create a UsenetDownloader configured for live provider testing
///
/// Returns the downloader and temp directory (keep temp_dir alive for test duration)
pub async fn create_live_downloader() -> Result<(Arc<UsenetDownloader>, TempDir), ConfigError> {
    let server = load_server_config()?;
    let temp_dir = tempfile::tempdir()
        .map_err(|e| ConfigError(format!("Failed to create temp dir: {}", e)))?;

    let config = Config {
        servers: vec![server],
        persistence: PersistenceConfig {
            database_path: temp_dir.path().join("test.db"),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        download: DownloadConfig {
            download_dir: temp_dir.path().join("downloads"),
            temp_dir: temp_dir.path().join("temp"),
            max_concurrent_downloads: 2,
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config)
        .await
        .map_err(|e| ConfigError(format!("Failed to create downloader: {}", e)))?;

    Ok((Arc::new(downloader), temp_dir))
}

/// Create a UsenetDownloader with bad credentials for auth failure tests
pub async fn create_downloader_bad_auth() -> Result<(Arc<UsenetDownloader>, TempDir), ConfigError> {
    let server = load_server_config_bad_password()?;
    let temp_dir = tempfile::tempdir()
        .map_err(|e| ConfigError(format!("Failed to create temp dir: {}", e)))?;

    let config = Config {
        servers: vec![server],
        persistence: PersistenceConfig {
            database_path: temp_dir.path().join("test.db"),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        download: DownloadConfig {
            download_dir: temp_dir.path().join("downloads"),
            temp_dir: temp_dir.path().join("temp"),
            max_concurrent_downloads: 1,
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config)
        .await
        .map_err(|e| ConfigError(format!("Failed to create downloader: {}", e)))?;

    Ok((Arc::new(downloader), temp_dir))
}

/// Create a downloader for local Docker NNTP server
#[cfg(feature = "docker-tests")]
pub async fn create_docker_downloader(
    host: &str,
    port: u16,
) -> Result<(Arc<UsenetDownloader>, TempDir), ConfigError> {
    let temp_dir = tempfile::tempdir()
        .map_err(|e| ConfigError(format!("Failed to create temp dir: {}", e)))?;

    let config = Config {
        servers: vec![ServerConfig {
            host: host.to_string(),
            port,
            tls: false, // Local Docker server typically doesn't use TLS
            username: None,
            password: None,
            connections: 2,
            priority: 0,
            pipeline_depth: 10,
        }],
        database_path: temp_dir.path().join("test.db"),
        download: DownloadConfig {
            download_dir: temp_dir.path().join("downloads"),
            temp_dir: temp_dir.path().join("temp"),
            max_concurrent_downloads: 2,
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config)
        .await
        .map_err(|e| ConfigError(format!("Failed to create downloader: {}", e)))?;

    Ok((Arc::new(downloader), temp_dir))
}

/// Check if live test credentials are available
pub fn has_live_credentials() -> bool {
    dotenvy::dotenv().ok();
    std::env::var("NNTP_HOST").is_ok()
        && std::env::var("NNTP_USERNAME").is_ok()
        && std::env::var("NNTP_PASSWORD").is_ok()
}

/// Skip test if credentials are not available
#[macro_export]
macro_rules! skip_if_no_credentials {
    () => {
        if !$crate::common::has_live_credentials() {
            eprintln!("Skipping test: NNTP credentials not found in .env");
            return;
        }
    };
}
