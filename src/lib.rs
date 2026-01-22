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
pub use types::{
    DownloadId, DownloadInfo, DownloadOptions, Event, HistoryEntry, Priority, Stage, Status,
};

/// Main entry point for the usenet-dl library
pub struct UsenetDownloader {
    /// Event broadcast channel sender (multiple subscribers supported)
    event_tx: tokio::sync::broadcast::Sender<crate::types::Event>,
    /// Configuration
    _config: Config,
}

impl UsenetDownloader {
    /// Create a new UsenetDownloader instance
    pub async fn new(config: Config) -> Result<Self> {
        // Create broadcast channel with buffer size of 1000 events
        // This allows multiple subscribers to receive all events independently
        let (event_tx, _rx) = tokio::sync::broadcast::channel(1000);

        Ok(Self {
            event_tx,
            _config: config,
        })
    }

    /// Subscribe to download events
    ///
    /// Multiple subscribers are supported. Each subscriber receives all events independently.
    /// Events are buffered, but if a subscriber falls behind by more than 1000 events,
    /// it will receive a `RecvError::Lagged` error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use usenet_dl::{UsenetDownloader, Config};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let downloader = UsenetDownloader::new(Config::default()).await?;
    ///
    ///     // UI subscriber
    ///     let mut ui_events = downloader.subscribe();
    ///     tokio::spawn(async move {
    ///         while let Ok(event) = ui_events.recv().await {
    ///             println!("UI: {:?}", event);
    ///         }
    ///     });
    ///
    ///     // Logging subscriber
    ///     let mut log_events = downloader.subscribe();
    ///     tokio::spawn(async move {
    ///         while let Ok(event) = log_events.recv().await {
    ///             tracing::info!(?event, "download event");
    ///         }
    ///     });
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<crate::types::Event> {
        self.event_tx.subscribe()
    }

    /// Emit an event to all subscribers
    ///
    /// This is an internal helper method used throughout the codebase to emit events.
    /// Events are sent to all active subscribers via the broadcast channel.
    ///
    /// If there are no active subscribers, the event is silently dropped (ok() converts Err to None).
    /// This allows the download process to continue even if no one is listening to events.
    pub(crate) fn emit_event(&self, event: crate::types::Event) {
        // send() returns Err if there are no receivers, which is fine - we just drop the event
        self.event_tx.send(event).ok();
    }
}
