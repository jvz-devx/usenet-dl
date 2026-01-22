//! # usenet-dl
//!
//! Backend library for SABnzbd/NZBGet-like applications.
//!
//! ## Design Philosophy
//!
//! usenet-dl is designed to be:
//! - **Highly configurable** - Almost every behavior can be customized
//! - **Sensible defaults** - Works out of the box with zero configuration
//! - **Library-first** - No CLI or UI, purely a Rust crate for embedding
//! - **Event-driven** - Consumers subscribe to events, no polling required
//!
//! ## Quick Start
//!
//! ```no_run
//! use usenet_dl::{UsenetDownloader, Config, ServerConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = Config {
//!         servers: vec![
//!             ServerConfig {
//!                 host: "news.example.com".to_string(),
//!                 port: 563,
//!                 tls: true,
//!                 username: Some("user".to_string()),
//!                 password: Some("pass".to_string()),
//!                 connections: 10,
//!                 priority: 0,
//!             }
//!         ],
//!         ..Default::default()
//!     };
//!
//!     let downloader = UsenetDownloader::new(config).await?;
//!
//!     // Subscribe to events
//!     let mut events = downloader.subscribe();
//!     tokio::spawn(async move {
//!         while let Ok(event) = events.recv().await {
//!             println!("Event: {:?}", event);
//!         }
//!     });
//!
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod config;
pub mod db;
pub mod error;
pub mod types;

// Re-export commonly used types
pub use config::{Config, ServerConfig};
pub use db::Database;
pub use error::{Error, Result};
pub use types::{DownloadId, DownloadInfo, DownloadOptions, HistoryEntry, Priority, Status};

/// Main entry point for the usenet-dl library
pub struct UsenetDownloader {
    _marker: std::marker::PhantomData<()>,
}

impl UsenetDownloader {
    /// Create a new UsenetDownloader instance
    pub async fn new(_config: Config) -> Result<Self> {
        Ok(Self {
            _marker: std::marker::PhantomData,
        })
    }

    /// Subscribe to download events
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<crate::types::Event> {
        // Placeholder - will be implemented in Phase 1
        let (tx, rx) = tokio::sync::broadcast::channel(100);
        drop(tx);
        rx
    }
}
