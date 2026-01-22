//! Configuration types for usenet-dl

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main configuration for UsenetDownloader
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// NNTP server configurations (at least one required)
    pub servers: Vec<ServerConfig>,

    /// Download directory (default: "./downloads")
    #[serde(default = "default_download_dir")]
    pub download_dir: PathBuf,

    /// Temporary directory (default: "./temp")
    #[serde(default = "default_temp_dir")]
    pub temp_dir: PathBuf,

    /// Maximum concurrent downloads (default: 3)
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_downloads: usize,

    /// Speed limit in bytes per second (None = unlimited)
    #[serde(default)]
    pub speed_limit_bps: Option<u64>,

    /// Database path (default: "./usenet-dl.db")
    #[serde(default = "default_database_path")]
    pub database_path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            servers: vec![],
            download_dir: default_download_dir(),
            temp_dir: default_temp_dir(),
            max_concurrent_downloads: default_max_concurrent(),
            speed_limit_bps: None,
            database_path: default_database_path(),
        }
    }
}

/// NNTP server configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Server hostname
    pub host: String,

    /// Server port (typically 119 for unencrypted, 563 for TLS)
    pub port: u16,

    /// Use TLS (implicit TLS, not STARTTLS)
    pub tls: bool,

    /// Username for authentication
    pub username: Option<String>,

    /// Password for authentication
    pub password: Option<String>,

    /// Number of connections to maintain
    #[serde(default = "default_connections")]
    pub connections: usize,

    /// Server priority (lower = tried first, for backup servers)
    #[serde(default)]
    pub priority: i32,
}

// Default value functions
fn default_download_dir() -> PathBuf {
    PathBuf::from("downloads")
}

fn default_temp_dir() -> PathBuf {
    PathBuf::from("temp")
}

fn default_max_concurrent() -> usize {
    3
}

fn default_database_path() -> PathBuf {
    PathBuf::from("usenet-dl.db")
}

fn default_connections() -> usize {
    10
}
