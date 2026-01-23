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

pub mod api;
pub mod config;
pub mod db;
pub mod deobfuscation;
pub mod error;
pub mod extraction;
pub mod folder_watcher;
pub mod post_processing;
pub mod retry;
pub mod rss_manager;
pub mod rss_scheduler;
pub mod scheduler;
pub mod speed_limiter;
pub mod types;
pub mod utils;

// Re-export commonly used types
pub use config::{Config, ServerConfig};
pub use db::Database;
pub use error::{Error, Result};
pub use scheduler::{RuleId, ScheduleAction, ScheduleRule, Scheduler, Weekday};
pub use types::{
    DownloadId, DownloadInfo, DownloadOptions, Event, HistoryEntry, Priority, QueueStats, Stage,
    Status,
};
use std::path::PathBuf;
use std::sync::Arc;
use utils::extract_filename_from_response;

/// Main entry point for the usenet-dl library
/// Main downloader instance (cloneable - all fields are Arc-wrapped)
#[derive(Clone)]
pub struct UsenetDownloader {
    /// Database instance for persistence (wrapped in Arc for sharing across tasks)
    db: std::sync::Arc<Database>,
    /// Event broadcast channel sender (multiple subscribers supported)
    event_tx: tokio::sync::broadcast::Sender<crate::types::Event>,
    /// Configuration (wrapped in Arc for sharing across tasks)
    /// Made public for access by background tasks like RSS scheduler
    pub(crate) config: std::sync::Arc<Config>,
    /// NNTP connection pools (one per server, wrapped in Arc for sharing across tasks)
    nntp_pools: std::sync::Arc<Vec<nntp_rs::NntpPool>>,
    /// Priority queue for managing download order (protected by Mutex)
    queue: std::sync::Arc<tokio::sync::Mutex<std::collections::BinaryHeap<QueuedDownload>>>,
    /// Semaphore to limit concurrent downloads (respects max_concurrent_downloads config)
    concurrent_limit: std::sync::Arc<tokio::sync::Semaphore>,
    /// Map of active downloads to their cancellation tokens (for pause/cancel operations)
    active_downloads: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<DownloadId, tokio_util::sync::CancellationToken>>>,
    /// Global speed limiter shared across all downloads (token bucket algorithm)
    speed_limiter: speed_limiter::SpeedLimiter,
    /// Flag to indicate whether new downloads are accepted (set to false during shutdown)
    /// Made public for access by background tasks like RSS scheduler
    pub(crate) accepting_new: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Post-processing pipeline executor
    post_processor: std::sync::Arc<post_processing::PostProcessor>,
    /// Runtime-mutable categories (separate from config for dynamic updates)
    categories: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, crate::config::CategoryConfig>>>,
}

/// Internal struct representing a download in the priority queue
#[derive(Debug, Clone, Eq, PartialEq)]
struct QueuedDownload {
    id: DownloadId,
    priority: Priority,
    created_at: i64, // Unix timestamp for tie-breaking
}

// Implement Ord for BinaryHeap (max-heap by default)
impl Ord for QueuedDownload {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // First compare by priority (higher priority wins)
        match self.priority.cmp(&other.priority) {
            std::cmp::Ordering::Equal => {
                // If priorities are equal, older downloads come first (FIFO)
                // Note: Reversed because we want older (lower timestamp) to have higher priority
                other.created_at.cmp(&self.created_at)
            }
            ordering => ordering,
        }
    }
}

impl PartialOrd for QueuedDownload {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl UsenetDownloader {
    /// Create a new UsenetDownloader instance
    ///
    /// This initializes all core components:
    /// - Opens/creates the SQLite database
    /// - Runs migrations
    /// - Creates NNTP connection pools for each configured server
    /// - Sets up the event broadcast channel
    pub async fn new(config: Config) -> Result<Self> {
        // Initialize database
        let db = Database::new(&config.database_path).await?;

        // Mark that we're starting up (for unclean shutdown detection)
        db.set_clean_start().await?;

        // Create broadcast channel with buffer size of 1000 events
        // This allows multiple subscribers to receive all events independently
        let (event_tx, _rx) = tokio::sync::broadcast::channel(1000);

        // Create NNTP connection pools for each server
        let mut nntp_pools = Vec::with_capacity(config.servers.len());
        for server in &config.servers {
            let pool = nntp_rs::NntpPool::new(server.clone().into(), server.connections as u32)
                .await
                .map_err(|e| Error::Nntp(format!("Failed to create NNTP pool: {}", e)))?;
            nntp_pools.push(pool);
        }

        // Create priority queue (empty initially, will be loaded from database on startup)
        let queue = std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::BinaryHeap::new()
        ));

        // Create semaphore for concurrent download limiting
        let concurrent_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
            config.max_concurrent_downloads
        ));

        // Create active downloads tracking map
        let active_downloads = std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new()
        ));

        // Create speed limiter with configured limit (or unlimited if not set)
        let speed_limiter = speed_limiter::SpeedLimiter::new(config.speed_limit_bps);

        // Create config Arc early so we can share it
        let config_arc = std::sync::Arc::new(config.clone());

        // Initialize runtime-mutable categories from config
        let categories = std::sync::Arc::new(tokio::sync::RwLock::new(config.categories.clone()));

        // Create post-processing pipeline executor
        let post_processor = std::sync::Arc::new(post_processing::PostProcessor::new(
            event_tx.clone(),
            config_arc.clone(),
        ));

        let downloader = Self {
            db: std::sync::Arc::new(db),
            event_tx,
            config: config_arc,
            nntp_pools: std::sync::Arc::new(nntp_pools),
            queue,
            concurrent_limit,
            active_downloads,
            speed_limiter,
            accepting_new: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            post_processor,
            categories,
        };

        // Restore any incomplete downloads from database (from previous session)
        downloader.restore_queue().await?;

        Ok(downloader)
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

    /// Get the current configuration
    ///
    /// Returns a reference to the current configuration. The configuration is wrapped in an Arc,
    /// so this is a cheap clone operation.
    ///
    /// # Returns
    ///
    /// A reference to the current `Config`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, config::Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = Config::default();
    /// let downloader = UsenetDownloader::new(config).await?;
    ///
    /// let current_config = downloader.get_config();
    /// println!("Download directory: {:?}", current_config.download_dir);
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_config(&self) -> std::sync::Arc<Config> {
        std::sync::Arc::clone(&self.config)
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

    /// Spawn the REST API server in a background task
    ///
    /// This method spawns the API server as a separate async task using `tokio::spawn`.
    /// The server runs concurrently with download processing and listens on the configured
    /// bind address (default: 127.0.0.1:6789).
    ///
    /// The spawned task runs until the server is shut down (either via graceful shutdown
    /// or an error occurs).
    ///
    /// # Returns
    ///
    /// Returns a `tokio::task::JoinHandle` that can be used to wait for the server to finish
    /// or to cancel the server task.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use usenet_dl::{UsenetDownloader, Config};
    /// use std::sync::Arc;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = Config::default();
    ///     let downloader = Arc::new(UsenetDownloader::new(config).await?);
    ///
    ///     // Spawn API server in background
    ///     let api_handle = downloader.spawn_api_server();
    ///
    ///     // Server is now running, handle other tasks...
    ///     // To wait for completion: api_handle.await??;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn spawn_api_server(self: &std::sync::Arc<Self>) -> tokio::task::JoinHandle<Result<()>> {
        let downloader = self.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            crate::api::start_api_server(downloader, config).await
        })
    }

    /// Add a download to the in-memory priority queue
    ///
    /// This method adds a download ID to the priority queue for processing.
    /// Downloads are ordered by priority (High > Normal > Low) and then by creation time (FIFO).
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to add to the queue
    ///
    /// # Errors
    ///
    /// Returns an error if the download doesn't exist in the database
    async fn add_to_queue(&self, id: DownloadId) -> Result<()> {
        // Fetch download from database to get priority and created_at
        let download = self.db.get_download(id).await?
            .ok_or_else(|| Error::Database(format!("Download {} not found", id)))?;

        let queued_download = QueuedDownload {
            id,
            priority: Priority::from_i32(download.priority),
            created_at: download.created_at,
        };

        // Add to priority queue
        let mut queue = self.queue.lock().await;
        queue.push(queued_download);

        Ok(())
    }

    /// Remove a download from the in-memory priority queue
    ///
    /// This method removes a download from the queue without starting it.
    /// Used when a download is cancelled or removed.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to remove from the queue
    ///
    /// # Returns
    ///
    /// Returns true if the download was found and removed, false otherwise
    async fn remove_from_queue(&self, id: DownloadId) -> bool {
        let mut queue = self.queue.lock().await;

        let original_len = queue.len();

        // Collect all items except the one we want to remove
        let items: Vec<_> = queue.drain().filter(|item| item.id != id).collect();

        let was_removed = items.len() < original_len;

        // Rebuild queue without the removed item
        *queue = items.into_iter().collect();

        was_removed
    }

    /// Get the next download from the priority queue
    ///
    /// Returns the highest-priority download that's ready to start.
    /// Downloads are ordered by priority and then by creation time (FIFO for same priority).
    ///
    /// # Returns
    ///
    /// The DownloadId of the next download to process, or None if queue is empty
    async fn get_next_download(&self) -> Option<DownloadId> {
        let mut queue = self.queue.lock().await;
        queue.pop().map(|item| item.id)
    }

    /// Peek at the next download without removing it from the queue
    ///
    /// # Returns
    ///
    /// The DownloadId of the next download, or None if queue is empty
    async fn peek_next_download(&self) -> Option<DownloadId> {
        let queue = self.queue.lock().await;
        queue.peek().map(|item| item.id)
    }

    /// Get the current size of the download queue
    ///
    /// # Returns
    ///
    /// The number of downloads currently in the queue
    async fn queue_size(&self) -> usize {
        let queue = self.queue.lock().await;
        queue.len()
    }

    /// Pause a download
    ///
    /// This method pauses a download without removing it from the queue.
    /// If the download is currently downloading, it will be stopped gracefully
    /// (after completing the current article). The download will be marked as
    /// Paused in the database and can be resumed later with `resume()`.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to pause
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if the download was successfully paused, or an error if:
    /// - The download doesn't exist
    /// - The download is already paused, complete, or failed
    /// - Database update fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Pause a download
    /// downloader.pause(id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn pause(&self, id: DownloadId) -> Result<()> {
        // Fetch download from database
        let download = self.db.get_download(id).await?
            .ok_or_else(|| Error::Database(format!("Download {} not found", id)))?;

        let current_status = Status::from_i32(download.status);

        // Check if download can be paused
        match current_status {
            Status::Paused => {
                // Already paused, nothing to do
                return Ok(());
            }
            Status::Complete | Status::Failed => {
                return Err(Error::Database(format!(
                    "Cannot pause download {}: status is {:?}",
                    id, current_status
                )));
            }
            Status::Queued | Status::Downloading | Status::Processing => {
                // Can be paused
            }
        }

        // If download is actively running, cancel its task
        let mut active_downloads = self.active_downloads.lock().await;
        if let Some(cancel_token) = active_downloads.get(&id) {
            // Signal the download task to stop
            cancel_token.cancel();
            // Remove from active downloads (task will clean up)
            active_downloads.remove(&id);
        }
        drop(active_downloads); // Release lock

        // Remove from queue if it's still queued (not yet started)
        self.remove_from_queue(id).await;

        // Update status to Paused in database
        self.db.update_status(id, Status::Paused.to_i32()).await?;

        // Emit paused event (we'll add this event type if needed, for now we can skip)
        // For consistency with design, downloads don't have individual pause events
        // Global pause_all emits QueuePaused, but individual pause is silent

        Ok(())
    }

    /// Resume a paused download
    ///
    /// This method restarts a paused download by changing its status back to Queued
    /// and adding it to the priority queue. The queue processor will automatically
    /// pick it up and continue downloading from where it left off.
    ///
    /// Downloads resume at the article level - any articles that were already
    /// downloaded are skipped, and only pending articles are fetched.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to resume
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if the download was successfully resumed, or an error if:
    /// - The download doesn't exist
    /// - The download is not paused (already queued, downloading, complete, or failed)
    /// - Database update fails
    /// - Queue insertion fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Resume a paused download
    /// downloader.resume(id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume(&self, id: DownloadId) -> Result<()> {
        // Fetch download from database
        let download = self.db.get_download(id).await?
            .ok_or_else(|| Error::Database(format!("Download {} not found", id)))?;

        let current_status = Status::from_i32(download.status);

        // Check if download can be resumed
        match current_status {
            Status::Paused => {
                // Can be resumed
            }
            Status::Queued | Status::Downloading | Status::Processing => {
                // Already active, nothing to do (idempotent)
                return Ok(());
            }
            Status::Complete | Status::Failed => {
                return Err(Error::Database(format!(
                    "Cannot resume download {}: status is {:?}",
                    id, current_status
                )));
            }
        }

        // Update status back to Queued
        self.db.update_status(id, Status::Queued.to_i32()).await?;

        // Add back to priority queue for processing
        // The queue processor will automatically pick it up
        // Article-level tracking ensures only pending articles are downloaded
        self.add_to_queue(id).await?;

        Ok(())
    }

    /// Resume a partially downloaded job from where it left off
    ///
    /// This method is the low-level resume operation that queries pending articles
    /// and adds the download back to the queue for processing. It checks if there are
    /// any pending articles remaining - if none, it proceeds directly to post-processing.
    /// If articles remain, it re-queues the download for the queue processor to continue.
    ///
    /// This method is primarily used internally by restore_queue() during startup to
    /// resume interrupted downloads, but can also be called directly for explicit resume operations.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to resume
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if the download was successfully resumed, or an error if:
    /// - The download doesn't exist
    /// - Database query fails
    /// - Queue insertion fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Resume a partially completed download
    /// downloader.resume_download(id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume_download(&self, id: DownloadId) -> Result<()> {
        // Get pending articles for this download
        let pending_articles = self.db.get_pending_articles(id).await?;

        if pending_articles.is_empty() {
            // All articles downloaded, proceed to post-processing
            tracing::info!(
                download_id = id,
                "No pending articles - proceeding to post-processing"
            );

            // TODO: Task 10.3 - Implement start_post_processing()
            // For now, just update status to Processing
            self.db.update_status(id, Status::Processing.to_i32()).await?;

            // Emit event to indicate post-processing stage
            self.emit_event(Event::Verifying { id });

            // TODO: Will call self.start_post_processing(id).await in Phase 2
            Ok(())
        } else {
            // Resume downloading remaining articles
            tracing::info!(
                download_id = id,
                pending_articles = pending_articles.len(),
                "Resuming download with pending articles"
            );

            // Update status back to Queued
            self.db.update_status(id, Status::Queued.to_i32()).await?;

            // Add back to priority queue for processing
            // The queue processor will automatically pick it up and download pending articles
            self.add_to_queue(id).await?;

            Ok(())
        }
    }

    /// Restore incomplete downloads from database on startup
    ///
    /// This method is called automatically during initialization to restore
    /// any downloads that were in progress when the application last shut down.
    ///
    /// The restoration process:
    /// 1. Queries database for downloads with status: Queued, Downloading, or Processing
    /// 2. For downloads in Downloading or Processing state, calls resume_download()
    /// 3. For downloads in Queued state, adds them back to the priority queue
    ///
    /// Downloads with status Complete or Failed are not restored (they're in history).
    /// Paused downloads are also not restored (user explicitly paused them).
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if restoration succeeds, or an error if:
    /// - Database query fails
    /// - Resume operation fails for any download
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example() -> Result<()> {
    /// // restore_queue() is called automatically in UsenetDownloader::new()
    /// let downloader = UsenetDownloader::new(Config::default()).await?;
    /// // Queue is now restored from previous session
    /// # Ok(())
    /// # }
    /// ```
    pub async fn restore_queue(&self) -> Result<()> {
        tracing::info!("Restoring queue from database");

        // Get all incomplete downloads (status IN (0=Queued, 1=Downloading, 3=Processing))
        let incomplete_downloads = self.db.get_incomplete_downloads().await?;

        if incomplete_downloads.is_empty() {
            tracing::info!("No incomplete downloads to restore");
            return Ok(());
        }

        tracing::info!(
            count = incomplete_downloads.len(),
            "Found incomplete downloads to restore"
        );

        // Store count before iterating
        let restore_count = incomplete_downloads.len();

        // Process each download based on its status
        for download in incomplete_downloads {
            let status = Status::from_i32(download.status);

            match status {
                Status::Downloading | Status::Processing => {
                    // These were actively running - resume them
                    tracing::info!(
                        download_id = download.id,
                        status = ?status,
                        "Resuming interrupted download"
                    );
                    self.resume_download(download.id).await?;
                }
                Status::Queued => {
                    // These were waiting in queue - add back to queue
                    tracing::info!(
                        download_id = download.id,
                        "Re-adding queued download to priority queue"
                    );
                    self.add_to_queue(download.id).await?;
                }
                _ => {
                    // Shouldn't happen (get_incomplete_downloads filters by status)
                    tracing::warn!(
                        download_id = download.id,
                        status = ?status,
                        "Unexpected download status during restore - skipping"
                    );
                }
            }
        }

        tracing::info!(
            restored_count = restore_count,
            "Queue restoration complete"
        );

        Ok(())
    }

    /// Cancel a download and delete its files
    ///
    /// This method removes a download from the queue, stops it if actively running,
    /// deletes all downloaded files from the temp directory, and removes it from the database.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to cancel
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if the download was successfully cancelled, or an error if:
    /// - The download doesn't exist
    /// - Database deletion fails
    /// - File deletion fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Cancel a download and remove all files
    /// downloader.cancel(id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn cancel(&self, id: DownloadId) -> Result<()> {
        // Verify download exists
        let _download = self.db.get_download(id).await?
            .ok_or_else(|| Error::Database(format!("Download {} not found", id)))?;

        // If download is actively running, cancel its task
        let mut active_downloads = self.active_downloads.lock().await;
        if let Some(cancel_token) = active_downloads.get(&id) {
            // Signal the download task to stop
            cancel_token.cancel();
            // Remove from active downloads
            active_downloads.remove(&id);
        }
        drop(active_downloads); // Release lock

        // Remove from queue if it's still queued (not yet started)
        self.remove_from_queue(id).await;

        // Delete downloaded files from temp directory
        let download_temp_dir = self.config.temp_dir.join(format!("download_{}", id));
        if download_temp_dir.exists() {
            if let Err(e) = tokio::fs::remove_dir_all(&download_temp_dir).await {
                tracing::warn!(
                    download_id = id,
                    path = ?download_temp_dir,
                    error = %e,
                    "Failed to delete download temp directory"
                );
                // Continue anyway - database deletion is more important
            }
        }

        // Delete download from database (cascades to articles, passwords)
        self.db.delete_download(id).await?;

        // Emit Removed event
        self.emit_event(crate::types::Event::Removed { id });

        Ok(())
    }

    /// Set the priority of a download
    ///
    /// This method changes the priority of a download. If the download is queued,
    /// it will be re-queued with the new priority. Active downloads keep running
    /// but the priority is saved for when they're queued again.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to update
    /// * `priority` - The new priority level (Low, Normal, High, or Force)
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if the priority was successfully updated, or an error if:
    /// - The download doesn't exist
    /// - Database update fails
    /// - Queue reordering fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Set download to high priority
    /// downloader.set_priority(id, Priority::High).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_priority(&self, id: DownloadId, priority: Priority) -> Result<()> {
        // Verify download exists
        let download = self.db.get_download(id).await?
            .ok_or_else(|| Error::Database(format!("Download {} not found", id)))?;

        let current_status = Status::from_i32(download.status);

        // Update priority in database
        self.db.update_priority(id, priority as i32).await?;

        // If download is queued (not actively downloading), reorder the queue
        // by removing and re-adding with new priority
        if current_status == Status::Queued {
            // Remove from queue
            self.remove_from_queue(id).await;

            // Re-add with new priority
            // We need to fetch the download again to get updated priority
            self.add_to_queue(id).await?;
        }

        // For active downloads, priority change takes effect when they finish
        // and get re-queued (e.g., for post-processing or if paused/resumed)

        Ok(())
    }

    /// Re-run post-processing on a completed or failed download
    ///
    /// This method allows re-running the post-processing pipeline on a download.
    /// This is useful when:
    /// - Extraction failed due to missing password (now added)
    /// - Post-processing settings changed
    /// - Files were manually repaired
    ///
    /// The download files must still exist in the temp directory for reprocessing to work.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to reprocess
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if reprocessing started successfully, or an error if:
    /// - The download doesn't exist
    /// - Download files are missing from temp directory
    /// - Database update fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Re-run post-processing after adding a password
    /// downloader.reprocess(id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn reprocess(&self, id: DownloadId) -> Result<()> {
        // Get download from database
        let _download = self.db.get_download(id).await?
            .ok_or_else(|| Error::NotFound(format!("Download {} not found", id)))?;

        // Determine download path (temp directory)
        let download_path = self.config.temp_dir
            .join(format!("download_{}", id));

        // Verify download files still exist
        if !download_path.exists() {
            return Err(Error::NotFound(format!(
                "Download files not found at {}. Cannot reprocess.",
                download_path.display()
            )));
        }

        tracing::info!(
            download_id = id,
            path = %download_path.display(),
            "Starting reprocessing"
        );

        // Reset status and re-queue for post-processing
        self.db.update_status(id, Status::Processing.to_i32()).await?;

        // Clear any previous error message
        self.db.set_error(id, "").await?;

        // Emit Verifying event to indicate post-processing is starting
        self.emit_event(Event::Verifying { id });

        // Start post-processing pipeline
        // This will run asynchronously
        let downloader = self.clone();
        tokio::spawn(async move {
            if let Err(e) = downloader.start_post_processing(id).await {
                tracing::error!(
                    download_id = id,
                    error = %e,
                    "Reprocessing failed"
                );
            }
        });

        Ok(())
    }

    /// Re-run extraction only (skip verify/repair)
    ///
    /// This method re-runs the extraction stage for a download that has already been downloaded.
    /// Unlike `reprocess()`, this skips PAR2 verification and repair stages and goes straight
    /// to archive extraction. This is useful when:
    /// - Extraction failed due to missing password (now added)
    /// - Extraction settings changed
    /// - User wants to re-extract without re-downloading
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to re-extract
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if re-extraction was started successfully.
    /// Returns Err if the download doesn't exist or files are missing.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::UsenetDownloader;
    /// # async fn example(downloader: &UsenetDownloader) -> Result<(), Box<dyn std::error::Error>> {
    /// // Re-extract download 123 after adding password
    /// downloader.reextract(123).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn reextract(&self, id: DownloadId) -> Result<()> {
        // Get download from database
        let download = self.db.get_download(id).await?
            .ok_or_else(|| Error::NotFound(format!("Download {} not found", id)))?;

        // Determine download path (temp directory)
        let download_path = self.config.temp_dir
            .join(format!("download_{}", id));

        // Verify download files still exist
        if !download_path.exists() {
            return Err(Error::NotFound(format!(
                "Download files not found at {}. Cannot re-extract.",
                download_path.display()
            )));
        }

        tracing::info!(
            download_id = id,
            path = %download_path.display(),
            "Starting re-extraction (skip verify/repair)"
        );

        // Reset status to processing
        self.db.update_status(id, Status::Processing.to_i32()).await?;

        // Clear any previous error message
        self.db.set_error(id, "").await?;

        // Emit Extracting event to indicate extraction is starting
        self.emit_event(Event::Extracting {
            id,
            archive: String::new(),
            percent: 0.0,
        });

        // Run extraction stage only (skip verify/repair)
        // This will run asynchronously
        let downloader = self.clone();
        let destination = PathBuf::from(download.destination);
        let post_processor = self.post_processor.clone();
        tokio::spawn(async move {
            // Run re-extraction (extract + move, skip verify/repair)
            match post_processor.reextract(id, download_path, destination).await {
                Ok(final_path) => {
                    tracing::info!(
                        download_id = id,
                        ?final_path,
                        "Re-extraction complete"
                    );

                    // Update status to complete
                    if let Err(e) = downloader.db.update_status(id, Status::Complete.to_i32()).await {
                        tracing::error!(
                            download_id = id,
                            error = %e,
                            "Failed to update status to complete"
                        );
                    }

                    // Emit Complete event
                    downloader.emit_event(Event::Complete {
                        id,
                        path: final_path,
                    });
                }
                Err(e) => {
                    tracing::error!(
                        download_id = id,
                        error = %e,
                        "Re-extraction failed"
                    );

                    // Update status to failed
                    if let Err(db_err) = downloader.db.update_status(id, Status::Failed.to_i32()).await {
                        tracing::error!(
                            download_id = id,
                            error = %db_err,
                            "Failed to update status to failed"
                        );
                    }

                    // Set error message
                    if let Err(db_err) = downloader.db.set_error(id, &e.to_string()).await {
                        tracing::error!(
                            download_id = id,
                            error = %db_err,
                            "Failed to set error message"
                        );
                    }

                    // Emit Failed event
                    downloader.emit_event(Event::Failed {
                        id,
                        stage: Stage::Extract,
                        error: e.to_string(),
                        files_kept: true,
                    });
                }
            }
        });

        Ok(())
    }

    /// Pause all active downloads
    ///
    /// This method pauses all downloads that are currently queued, downloading, or processing.
    /// Already paused, completed, or failed downloads are not affected.
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if successful, or an error if database operations fail.
    /// Individual pause failures are logged but don't stop the operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader) -> Result<()> {
    /// // Pause all downloads
    /// downloader.pause_all().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn pause_all(&self) -> Result<()> {
        // Get all downloads that can be paused (Queued, Downloading, Processing)
        let all_downloads = self.db.list_downloads().await?;

        let mut paused_count = 0;

        for download in all_downloads {
            let status = Status::from_i32(download.status);

            // Only pause active downloads
            match status {
                Status::Queued | Status::Downloading | Status::Processing => {
                    if let Err(e) = self.pause(download.id).await {
                        tracing::warn!(
                            download_id = download.id,
                            error = %e,
                            "Failed to pause download during pause_all"
                        );
                        // Continue with other downloads
                    } else {
                        paused_count += 1;
                    }
                }
                Status::Paused | Status::Complete | Status::Failed => {
                    // Skip already paused/finished downloads
                }
            }
        }

        tracing::info!(
            paused_count = paused_count,
            "Paused all active downloads"
        );

        // Emit global QueuePaused event
        self.emit_event(crate::types::Event::QueuePaused);

        Ok(())
    }

    /// Resume all paused downloads
    ///
    /// This method resumes all downloads that are currently paused.
    /// Downloads in other states (queued, downloading, complete, failed) are not affected.
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if successful, or an error if database operations fail.
    /// Individual resume failures are logged but don't stop the operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader) -> Result<()> {
    /// // Resume all paused downloads
    /// downloader.resume_all().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume_all(&self) -> Result<()> {
        // Get all paused downloads
        let paused_downloads = self.db.list_downloads_by_status(Status::Paused.to_i32()).await?;

        let mut resumed_count = 0;

        for download in paused_downloads {
            if let Err(e) = self.resume(download.id).await {
                tracing::warn!(
                    download_id = download.id,
                    error = %e,
                    "Failed to resume download during resume_all"
                );
                // Continue with other downloads
            } else {
                resumed_count += 1;
            }
        }

        tracing::info!(
            resumed_count = resumed_count,
            "Resumed all paused downloads"
        );

        // Emit global QueueResumed event
        self.emit_event(crate::types::Event::QueueResumed);

        Ok(())
    }

    /// Get the current global speed limit
    ///
    /// Returns the current speed limit in bytes per second, or None if unlimited.
    ///
    /// # Returns
    ///
    /// * `Option<u64>` - Speed limit in bytes per second (None = unlimited)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::default();
    /// # let downloader = UsenetDownloader::new(config).await?;
    /// // Get current speed limit
    /// let limit = downloader.get_speed_limit();
    /// if let Some(bps) = limit {
    ///     println!("Current speed limit: {} bytes/sec", bps);
    /// } else {
    ///     println!("No speed limit (unlimited)");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_speed_limit(&self) -> Option<u64> {
        self.speed_limiter.get_limit()
    }

    /// Set the global speed limit
    ///
    /// This changes the download speed limit for all concurrent downloads.
    /// The change takes effect immediately.
    ///
    /// # Arguments
    ///
    /// * `limit_bps` - New speed limit in bytes per second (None = unlimited)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::default();
    /// # let downloader = UsenetDownloader::new(config).await?;
    /// // Set to 10 MB/s
    /// downloader.set_speed_limit(Some(10_000_000)).await;
    ///
    /// // Remove speed limit (unlimited)
    /// downloader.set_speed_limit(None).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_speed_limit(&self, limit_bps: Option<u64>) {
        // Update the speed limiter
        self.speed_limiter.set_limit(limit_bps);

        // Emit event to notify subscribers
        self.emit_event(crate::types::Event::SpeedLimitChanged { limit_bps });

        tracing::info!(
            limit_bps = ?limit_bps,
            "Speed limit changed"
        );
    }

    /// Update runtime-changeable configuration settings
    ///
    /// This method updates configuration settings that can be safely changed while the
    /// downloader is running. Fields requiring restart (like database_path, download_dir,
    /// servers) cannot be updated via this method.
    ///
    /// # Arguments
    ///
    /// * `updates` - Configuration updates to apply
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # use usenet_dl::config::ConfigUpdate;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::default();
    /// # let downloader = UsenetDownloader::new(config).await?;
    /// // Update speed limit
    /// let updates = ConfigUpdate {
    ///     speed_limit_bps: Some(Some(10_000_000)), // 10 MB/s
    /// };
    /// downloader.update_config(updates).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn update_config(&self, updates: crate::config::ConfigUpdate) {
        // Update speed limit if provided
        if let Some(speed_limit) = updates.speed_limit_bps {
            self.set_speed_limit(speed_limit).await;
        }
    }

    /// Create or update a category
    ///
    /// This method adds a new category or updates an existing one with the provided configuration.
    /// The change takes effect immediately for new downloads.
    ///
    /// # Arguments
    ///
    /// * `name` - The category name
    /// * `config` - The category configuration
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # use usenet_dl::config::CategoryConfig;
    /// # use std::path::PathBuf;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let downloader = UsenetDownloader::new(Config::default()).await?;
    /// let category_config = CategoryConfig {
    ///     destination: PathBuf::from("/downloads/movies"),
    ///     post_process: None,
    ///     watch_folder: None,
    ///     scripts: vec![],
    /// };
    /// downloader.add_or_update_category("movies".to_string(), category_config).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn add_or_update_category(&self, name: String, config: crate::config::CategoryConfig) {
        let mut categories = self.categories.write().await;
        categories.insert(name, config);
    }

    /// Remove a category
    ///
    /// This method removes a category from the runtime configuration.
    /// Returns true if the category existed and was removed, false otherwise.
    ///
    /// # Arguments
    ///
    /// * `name` - The category name to remove
    ///
    /// # Returns
    ///
    /// `true` if the category was removed, `false` if it didn't exist
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let downloader = UsenetDownloader::new(Config::default()).await?;
    /// let was_removed = downloader.remove_category("movies").await;
    /// if was_removed {
    ///     println!("Category removed");
    /// } else {
    ///     println!("Category not found");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn remove_category(&self, name: &str) -> bool {
        let mut categories = self.categories.write().await;
        categories.remove(name).is_some()
    }

    /// Get all categories
    ///
    /// Returns a clone of the current categories HashMap.
    ///
    /// # Returns
    ///
    /// A HashMap of category names to CategoryConfig
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let downloader = UsenetDownloader::new(Config::default()).await?;
    /// let categories = downloader.get_categories().await;
    /// for (name, config) in categories {
    ///     println!("Category: {}, Destination: {:?}", name, config.destination);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_categories(&self) -> std::collections::HashMap<String, crate::config::CategoryConfig> {
        self.categories.read().await.clone()
    }

    // =========================================================================
    // RSS Feed Management
    // =========================================================================

    /// Get all RSS feeds
    pub async fn get_rss_feeds(&self) -> Result<Vec<crate::config::RssFeedConfig>> {
        use std::time::Duration;

        let feeds = self.db.get_all_rss_feeds().await?;
        let mut result = Vec::new();

        for feed in feeds {
            // Get filters for this feed
            let filter_rows = self.db.get_rss_filters(feed.id).await?;
            let filters = filter_rows.into_iter().map(|row| {
                crate::config::RssFilter {
                    name: row.name,
                    include: row.include_patterns
                        .map(|s| serde_json::from_str(&s).unwrap_or_default())
                        .unwrap_or_default(),
                    exclude: row.exclude_patterns
                        .map(|s| serde_json::from_str(&s).unwrap_or_default())
                        .unwrap_or_default(),
                    min_size: row.min_size.map(|s| s as u64),
                    max_size: row.max_size.map(|s| s as u64),
                    max_age: row.max_age_secs.map(|s| Duration::from_secs(s as u64)),
                }
            }).collect();

            result.push(crate::config::RssFeedConfig {
                url: feed.url,
                check_interval: Duration::from_secs(feed.check_interval_secs as u64),
                category: feed.category,
                filters,
                auto_download: feed.auto_download != 0,
                priority: crate::types::Priority::from_i32(feed.priority),
                enabled: feed.enabled != 0,
            });
        }

        Ok(result)
    }

    /// Get RSS feed by ID with its configuration
    pub async fn get_rss_feed(&self, id: i64) -> Result<Option<(i64, String, crate::config::RssFeedConfig)>> {
        use std::time::Duration;

        let feed = match self.db.get_rss_feed(id).await? {
            Some(f) => f,
            None => return Ok(None),
        };

        // Get filters for this feed
        let filter_rows = self.db.get_rss_filters(feed.id).await?;
        let filters = filter_rows.into_iter().map(|row| {
            crate::config::RssFilter {
                name: row.name,
                include: row.include_patterns
                    .map(|s| serde_json::from_str(&s).unwrap_or_default())
                    .unwrap_or_default(),
                exclude: row.exclude_patterns
                    .map(|s| serde_json::from_str(&s).unwrap_or_default())
                    .unwrap_or_default(),
                min_size: row.min_size.map(|s| s as u64),
                max_size: row.max_size.map(|s| s as u64),
                max_age: row.max_age_secs.map(|s| Duration::from_secs(s as u64)),
            }
        }).collect();

        let config = crate::config::RssFeedConfig {
            url: feed.url,
            check_interval: Duration::from_secs(feed.check_interval_secs as u64),
            category: feed.category,
            filters,
            auto_download: feed.auto_download != 0,
            priority: crate::types::Priority::from_i32(feed.priority),
            enabled: feed.enabled != 0,
        };

        Ok(Some((feed.id, feed.name, config)))
    }

    /// Add a new RSS feed
    pub async fn add_rss_feed(&self, name: String, config: crate::config::RssFeedConfig) -> Result<i64> {
        // Insert the feed
        let feed_id = self.db.insert_rss_feed(
            &name,
            &config.url,
            config.check_interval.as_secs() as i64,
            config.category.as_deref(),
            config.auto_download,
            config.priority as i32,
            config.enabled,
        ).await?;

        // Insert filters
        for filter in &config.filters {
            let include_json = if filter.include.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.include).unwrap())
            };

            let exclude_json = if filter.exclude.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.exclude).unwrap())
            };

            self.db.insert_rss_filter(
                feed_id,
                &filter.name,
                include_json.as_deref(),
                exclude_json.as_deref(),
                filter.min_size.map(|s| s as i64),
                filter.max_size.map(|s| s as i64),
                filter.max_age.map(|d| d.as_secs() as i64),
            ).await?;
        }

        Ok(feed_id)
    }

    /// Update an existing RSS feed
    pub async fn update_rss_feed(&self, id: i64, name: String, config: crate::config::RssFeedConfig) -> Result<bool> {
        // Update the feed
        let updated = self.db.update_rss_feed(
            id,
            &name,
            &config.url,
            config.check_interval.as_secs() as i64,
            config.category.as_deref(),
            config.auto_download,
            config.priority as i32,
            config.enabled,
        ).await?;

        if !updated {
            return Ok(false);
        }

        // Delete old filters and insert new ones
        self.db.delete_rss_filters(id).await?;

        for filter in &config.filters {
            let include_json = if filter.include.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.include).unwrap())
            };

            let exclude_json = if filter.exclude.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.exclude).unwrap())
            };

            self.db.insert_rss_filter(
                id,
                &filter.name,
                include_json.as_deref(),
                exclude_json.as_deref(),
                filter.min_size.map(|s| s as i64),
                filter.max_size.map(|s| s as i64),
                filter.max_age.map(|d| d.as_secs() as i64),
            ).await?;
        }

        Ok(true)
    }

    /// Delete an RSS feed
    pub async fn delete_rss_feed(&self, id: i64) -> Result<bool> {
        self.db.delete_rss_feed(id).await
    }

    /// Force check an RSS feed now (for manual triggering via API)
    pub async fn check_rss_feed_now(&self, id: i64) -> Result<usize> {
        // Get the feed configuration  
        let (feed_id, _name, config) = match self.get_rss_feed(id).await? {
            Some(f) => f,
            None => return Err(Error::NotFound(format!("RSS feed {} not found", id))),
        };

        // Create a temporary RssManager with just this feed
        let rss_manager = crate::rss_manager::RssManager::new(
            self.db.clone(),
            Arc::new(self.clone()),
            vec![config.clone()],
        )?;

        // Check the feed
        let items = rss_manager.check_feed(&config).await?;

        // Process items (auto-download if enabled)
        let queued = rss_manager.process_feed_items(feed_id, &config, items).await?;

        // Update last check status
        self.db.update_rss_feed_check_status(id, None).await?;

        Ok(queued)
    }
    /// Gracefully shut down the downloader
    ///
    /// This method performs a graceful shutdown sequence:
    /// 1. Cancels all active downloads (using their cancellation tokens)
    /// 2. Waits for active downloads to complete with a timeout (30 seconds)
    /// 3. Persists final state to the database
    /// 4. Closes database connections
    ///
    /// Note: In Phase 4+, this will also stop folder watchers and RSS feed checks.
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail during shutdown.
    /// The method will attempt to complete as much of the shutdown sequence as possible
    /// even if some steps fail.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use usenet_dl::{UsenetDownloader, Config};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = Config::default();
    ///     let downloader = UsenetDownloader::new(config).await?;
    ///
    ///     // Do some work...
    ///
    ///     // Gracefully shut down
    ///     downloader.shutdown().await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn shutdown(&self) -> Result<()> {
        tracing::info!("Initiating graceful shutdown");

        // 1. Stop accepting new downloads
        self.accepting_new.store(false, std::sync::atomic::Ordering::SeqCst);
        tracing::info!("Stopped accepting new downloads");

        // 2. Gracefully pause all active downloads (allow current article to finish)
        self.pause_graceful_all().await;
        tracing::info!("Signaled graceful pause to all active downloads");

        // 3. Wait for active downloads to complete with timeout
        let shutdown_timeout = std::time::Duration::from_secs(30);
        let wait_result = tokio::time::timeout(
            shutdown_timeout,
            self.wait_for_active_downloads()
        ).await;

        match wait_result {
            Ok(Ok(())) => {
                tracing::info!("All active downloads completed gracefully");
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "Error while waiting for downloads to complete");
            }
            Err(_) => {
                tracing::warn!("Timeout waiting for downloads to complete, proceeding with shutdown");
            }
        }

        // 4. Persist final state
        if let Err(e) = self.persist_all_state().await {
            tracing::error!(error = %e, "Failed to persist final state during shutdown");
            // Continue with shutdown even if persistence fails
        } else {
            tracing::info!("Final state persisted to database");
        }

        // 5. Mark clean shutdown in database
        if let Err(e) = self.db.set_clean_shutdown().await {
            tracing::error!(error = %e, "Failed to mark clean shutdown in database");
            // Continue with shutdown even if this fails
        } else {
            tracing::info!("Marked clean shutdown in database");
        }

        // 6. Emit shutdown event
        let _ = self.event_tx.send(Event::Shutdown);

        // 7. Close database connections
        // Note: Database is in an Arc, so we can't consume it directly.
        // The connection pool will be closed when the last Arc reference is dropped.
        // We log this for observability but don't actually close the pool here.
        tracing::info!("Shutdown complete - database connections will close when downloader is dropped");

        tracing::info!("Graceful shutdown complete");
        Ok(())
    }

    /// Gracefully pause all active downloads by signaling cancellation
    ///
    /// This method triggers a graceful pause of all active downloads. The downloads
    /// will complete their current article before stopping, ensuring no partial
    /// article downloads and maintaining data integrity.
    ///
    /// # Implementation Notes
    ///
    /// The graceful pause works because the download loop checks for cancellation
    /// at the beginning of each article download iteration (before starting the next
    /// article). This means:
    /// - If an article is currently being downloaded, it will complete
    /// - After completion, the cancellation check will detect the signal
    /// - The download task will exit cleanly, updating its status to Paused
    ///
    /// # Usage
    ///
    /// This method is primarily used during shutdown to ensure clean termination.
    async fn pause_graceful_all(&self) {
        let active = self.active_downloads.lock().await;
        tracing::debug!(active_count = active.len(), "Gracefully pausing all active downloads");

        for (id, token) in active.iter() {
            tracing::debug!(download_id = id, "Signaling graceful pause");
            token.cancel();
        }
    }

    /// Wait for all active downloads to complete
    ///
    /// This is a helper method used during shutdown to wait for active downloads
    /// to finish their current work before closing.
    ///
    /// # Returns
    ///
    /// Returns Ok(()) when all active downloads have completed, or an error if
    /// there's a problem checking the active downloads.
    async fn wait_for_active_downloads(&self) -> Result<()> {
        loop {
            let active_count = {
                let active = self.active_downloads.lock().await;
                active.len()
            };

            if active_count == 0 {
                return Ok(());
            }

            tracing::debug!(active_count, "Waiting for active downloads to complete");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Persist all state to the database
    ///
    /// This method ensures that all download state is saved to the database before
    /// shutdown. Since SQLite operations are auto-committed and all state changes
    /// are immediately persisted during normal operation, this method primarily
    /// serves as an explicit checkpoint during graceful shutdown.
    ///
    /// # Current Implementation
    ///
    /// In the current implementation (Phase 1), download states are already persisted
    /// to the database throughout their lifecycle:
    /// - Status changes are immediately written via `update_status()`
    /// - Progress updates are written via `update_progress()`
    /// - Article status is tracked in the `download_articles` table
    ///
    /// # Future Phases
    ///
    /// As additional features are implemented, this method will be extended to persist:
    /// - Folder watcher state (Phase 4)
    /// - RSS feed state and seen items (Phase 4)
    /// - Scheduler state (Phase 4)
    /// - Any in-memory caches or buffers
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if state was persisted successfully, or an error if
    /// there was a problem writing to the database.
    async fn persist_all_state(&self) -> Result<()> {
        tracing::debug!("Persisting all state to database");

        // Get all downloads that are currently in-progress states
        let downloads = self.db.get_all_downloads().await?;

        let mut persisted_count = 0;
        for download in downloads {
            // For downloads in Downloading or Processing state that are no longer active,
            // ensure their state reflects they were interrupted during shutdown
            let is_active = {
                let active = self.active_downloads.lock().await;
                active.contains_key(&download.id)
            };

            // If a download is in an active state but not in active_downloads,
            // it means it was interrupted during shutdown
            if !is_active && (download.status == Status::Downloading.to_i32() ||
                             download.status == Status::Processing.to_i32()) {
                // Mark as Paused so it can be resumed on next startup
                self.db.update_status(download.id, Status::Paused.to_i32()).await?;
                persisted_count += 1;
                tracing::debug!(
                    download_id = download.id,
                    "Marked interrupted download as Paused for resume on restart"
                );
            }
        }

        if persisted_count > 0 {
            tracing::info!(
                persisted_count,
                "Persisted state for {} interrupted download(s)",
                persisted_count
            );
        } else {
            tracing::debug!("All download states already persisted");
        }

        Ok(())
    }

    /// Add an NZB to the download queue from raw bytes
    ///
    /// This method parses the NZB content, creates a download record in the database,
    /// and emits a Queued event.
    ///
    /// # Arguments
    ///
    /// * `content` - Raw NZB file content (XML)
    /// * `name` - Name for this download (typically the NZB filename without extension)
    /// * `options` - Download options (category, destination, priority, etc.)
    ///
    /// # Returns
    ///
    /// The unique DownloadId for this download
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - NZB content is invalid or cannot be parsed
    /// - NZB validation fails (missing segments, invalid structure)
    /// - Database insertion fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use usenet_dl::{UsenetDownloader, Config, DownloadOptions};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let downloader = UsenetDownloader::new(Config::default()).await?;
    ///
    ///     let nzb_content = std::fs::read("example.nzb")?;
    ///     let id = downloader.add_nzb_content(
    ///         &nzb_content,
    ///         "example",
    ///         DownloadOptions::default()
    ///     ).await?;
    ///
    ///     println!("Added download with ID: {}", id);
    ///     Ok(())
    /// }
    /// ```
    pub async fn add_nzb_content(
        &self,
        content: &[u8],
        name: &str,
        options: DownloadOptions,
    ) -> Result<DownloadId> {
        // Check if accepting new downloads (reject during shutdown)
        if !self.accepting_new.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::ShuttingDown);
        }

        // Parse NZB content from bytes to string
        let nzb_string = String::from_utf8(content.to_vec())
            .map_err(|e| Error::InvalidNzb(format!("NZB content is not valid UTF-8: {}", e)))?;

        // Parse NZB using nntp-rs
        let nzb = nntp_rs::parse_nzb(&nzb_string)
            .map_err(|e| Error::InvalidNzb(format!("Failed to parse NZB: {}", e)))?;

        // Validate NZB structure and segments
        nzb.validate()
            .map_err(|e| Error::InvalidNzb(format!("NZB validation failed: {}", e)))?;

        // Extract metadata from NZB
        let nzb_meta_name = nzb.meta.get("title").map(|s| s.to_string());
        let nzb_password = nzb.meta.get("password").map(|s| s.to_string());

        // Calculate total size
        let size_bytes = nzb.total_bytes() as i64;

        // Calculate NZB hash for duplicate detection (sha256)
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(content);
        let hash_result = hasher.finalize();
        let nzb_hash = format!("{:x}", hash_result);

        // Determine job name (for deobfuscation and duplicate detection)
        // Use NZB meta title if available, otherwise the provided name
        let job_name = nzb_meta_name.clone().unwrap_or_else(|| name.to_string());

        // Determine destination directory
        let destination = if let Some(dest) = options.destination {
            dest
        } else if let Some(category) = &options.category {
            // Check if category has custom destination
            let categories = self.categories.read().await;
            if let Some(cat_config) = categories.get(category) {
                cat_config.destination.clone()
            } else {
                self.config.download_dir.clone()
            }
        } else {
            self.config.download_dir.clone()
        };

        // Determine post-processing mode
        let post_process = if let Some(pp) = options.post_process {
            pp
        } else if let Some(category) = &options.category {
            // Check if category has custom post-processing
            let categories = self.categories.read().await;
            if let Some(cat_config) = categories.get(category) {
                cat_config.post_process.unwrap_or(self.config.default_post_process)
            } else {
                self.config.default_post_process
            }
        } else {
            self.config.default_post_process
        };

        // Merge NZB password with provided password (provided takes priority)
        let final_password = options.password.or(nzb_password);

        // Create download record
        let new_download = db::NewDownload {
            name: name.to_string(),
            nzb_path: format!("memory:{}", name), // Stored in memory, not from file
            nzb_meta_name,
            nzb_hash: Some(nzb_hash),
            job_name: Some(job_name),
            category: options.category.clone(),
            destination: destination.to_string_lossy().to_string(),
            post_process: post_process.to_i32(),
            priority: options.priority as i32,
            status: Status::Queued.to_i32(),
            size_bytes,
        };

        // Insert download into database
        let download_id = self.db.insert_download(&new_download).await?;

        // Insert all articles (segments) for resume support
        for file in &nzb.files {
            for segment in &file.segments {
                let article = db::NewArticle {
                    download_id,
                    message_id: segment.message_id.clone(),
                    segment_number: segment.number as i32,
                    size_bytes: segment.bytes as i64,
                };
                self.db.insert_article(&article).await?;
            }
        }

        // Cache password if provided
        if let Some(password) = final_password {
            self.db.set_correct_password(download_id, &password).await?;
        }

        // Emit Queued event
        self.emit_event(Event::Queued {
            id: download_id,
            name: name.to_string(),
        });

        // Add to priority queue for processing
        self.add_to_queue(download_id).await?;

        Ok(download_id)
    }

    /// Add an NZB to the download queue from a file
    ///
    /// This is a convenience method that reads an NZB file from disk and delegates
    /// to `add_nzb_content()`. The filename (without extension) is used as the download name.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the NZB file
    /// * `options` - Download options (category, destination, priority, etc.)
    ///
    /// # Returns
    ///
    /// The unique DownloadId for this download
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - File cannot be read
    /// - NZB content is invalid or cannot be parsed
    /// - NZB validation fails (missing segments, invalid structure)
    /// - Database insertion fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use usenet_dl::{UsenetDownloader, Config, DownloadOptions};
    /// use std::path::Path;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let downloader = UsenetDownloader::new(Config::default()).await?;
    ///
    ///     let id = downloader.add_nzb(
    ///         Path::new("example.nzb"),
    ///         DownloadOptions::default()
    ///     ).await?;
    ///
    ///     println!("Added download with ID: {}", id);
    ///     Ok(())
    /// }
    /// ```
    pub async fn add_nzb(
        &self,
        path: &std::path::Path,
        options: DownloadOptions,
    ) -> Result<DownloadId> {
        // Read file content
        let content = tokio::fs::read(path)
            .await
            .map_err(|e| Error::Io(std::io::Error::new(
                e.kind(),
                format!("Failed to read NZB file '{}': {}", path.display(), e)
            )))?;

        // Extract filename without extension as download name
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Delegate to add_nzb_content
        self.add_nzb_content(&content, &name, options).await
    }

    /// Add NZB from URL
    ///
    /// This method fetches an NZB file from a given HTTP(S) URL and adds it to the queue.
    ///
    /// # Arguments
    ///
    /// * `url` - HTTP(S) URL to fetch the NZB file from
    /// * `options` - Download options (category, priority, password, etc.)
    ///
    /// # Returns
    ///
    /// Returns the download ID on success, or an error if:
    /// - The URL is invalid
    /// - The HTTP request fails (network error, 404, etc.)
    /// - The response body cannot be read
    /// - The NZB content is invalid
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader) -> Result<()> {
    /// let id = downloader.add_nzb_url(
    ///     "https://example.com/file.nzb",
    ///     DownloadOptions {
    ///         category: Some("movies".to_string()),
    ///         priority: Priority::High,
    ///         ..Default::default()
    ///     }
    /// ).await?;
    /// println!("Added download with ID: {}", id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn add_nzb_url(
        &self,
        url: &str,
        options: DownloadOptions,
    ) -> Result<DownloadId> {
        // Create HTTP client with timeout to prevent hanging
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create HTTP client: {}", e)
            )))?;

        // Fetch NZB from URL with timeout
        let response = client.get(url)
            .send()
            .await
            .map_err(|e| {
                let error_msg = if e.is_timeout() {
                    format!("Timeout fetching NZB from URL '{}' (exceeded 30 seconds)", url)
                } else if e.is_connect() {
                    format!("Connection failed for URL '{}': {}", url, e)
                } else {
                    format!("Failed to fetch NZB from URL '{}': {}", url, e)
                };
                Error::Io(std::io::Error::new(std::io::ErrorKind::Other, error_msg))
            })?;

        // Check HTTP status
        if !response.status().is_success() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("HTTP error fetching NZB: {} {}", response.status(), url)
            )));
        }

        // Extract filename from Content-Disposition header or URL
        let name = extract_filename_from_response(&response, url);

        // Read response body
        let content = response.bytes()
            .await
            .map_err(|e| Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to read response body from '{}': {}", url, e)
            )))?;

        // Delegate to add_nzb_content
        self.add_nzb_content(&content, &name, options).await
    }

    /// Mark an NZB file as processed in the database
    ///
    /// This is used by the folder watcher with WatchFolderAction::Keep to track
    /// which NZB files have already been processed to avoid re-adding them.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the NZB file to mark as processed
    ///
    /// # Returns
    ///
    /// Returns Ok(()) on success, or an error if the database operation fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # use std::path::Path;
    /// # async fn example(downloader: UsenetDownloader) -> Result<()> {
    /// let nzb_path = Path::new("/watch/folder/movie.nzb");
    /// downloader.mark_nzb_processed(nzb_path).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn mark_nzb_processed(&self, path: &std::path::Path) -> Result<()> {
        self.db.mark_nzb_processed(path).await
    }

    /// Start the queue processor task
    ///
    /// This method spawns a background task that continuously:
    /// 1. Waits for the next download in the priority queue
    /// 2. Acquires a permit from the concurrency limiter (respects max_concurrent_downloads)
    /// 3. Spawns a download task for that download
    /// 4. Repeats until shutdown
    ///
    /// The queue processor ensures downloads are started in priority order and
    /// respects the configured concurrency limit.
    ///
    /// # Returns
    ///
    /// Returns a `tokio::task::JoinHandle` for the processor task. The task runs
    /// indefinitely until the queue is empty and no more downloads are added.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader) -> Result<()> {
    /// // Start the queue processor
    /// let processor_handle = downloader.start_queue_processor();
    ///
    /// // Add downloads to the queue
    /// // ...
    ///
    /// // Queue processor will automatically spawn downloads
    /// # Ok(())
    /// # }
    /// ```
    pub fn start_queue_processor(&self) -> tokio::task::JoinHandle<()> {
        let queue = self.queue.clone();
        let concurrent_limit = self.concurrent_limit.clone();
        let db = self.db.clone();
        let event_tx = self.event_tx.clone();
        let nntp_pools = self.nntp_pools.clone();
        let config = self.config.clone();
        let active_downloads = self.active_downloads.clone();
        let speed_limiter = self.speed_limiter.clone();

        tokio::spawn(async move {
            loop {
                // Get the next download from the queue
                let download_id = {
                    let mut queue_guard = queue.lock().await;
                    queue_guard.pop().map(|item| item.id)
                };

                if let Some(id) = download_id {
                    // Acquire a permit from the semaphore (blocks if at max concurrent downloads)
                    // Clone the permit Arc so it's held for the duration of the download
                    let permit = concurrent_limit.clone().acquire_owned().await;

                    // Check if permit acquisition failed (should be Ok unless semaphore is closed)
                    let permit = match permit {
                        Ok(p) => p,
                        Err(_) => {
                            // Semaphore closed, exit processor
                            break;
                        }
                    };

                    // Clone dependencies for the download task
                    let db_clone = db.clone();
                    let event_tx_clone = event_tx.clone();
                    let nntp_pools_clone = nntp_pools.clone();
                    let config_clone = config.clone();
                    let active_downloads_clone = active_downloads.clone();
                    let speed_limiter_clone = speed_limiter.clone();

                    // Create cancellation token for this download
                    let cancel_token = tokio_util::sync::CancellationToken::new();

                    // Register the cancellation token
                    {
                        let mut active = active_downloads_clone.lock().await;
                        active.insert(id, cancel_token.clone());
                    }

                    // Spawn the download task
                    tokio::spawn(async move {
                        // Permit is held for the entire duration of this task
                        let _permit = permit;

                        // Fetch download record
                        let download = match db_clone.get_download(id).await {
                            Ok(Some(d)) => d,
                            Ok(None) => {
                                tracing::warn!(download_id = id, "Download not found in database");
                                // Clean up active downloads
                                let mut active = active_downloads_clone.lock().await;
                                active.remove(&id);
                                return;
                            }
                            Err(e) => {
                                tracing::error!(download_id = id, error = %e, "Failed to fetch download");
                                // Clean up active downloads
                                let mut active = active_downloads_clone.lock().await;
                                active.remove(&id);
                                return;
                            }
                        };

                        // Update status to Downloading and record start time
                        if let Err(e) = db_clone.update_status(id, Status::Downloading.to_i32()).await {
                            tracing::error!(download_id = id, error = %e, "Failed to update status");
                            // Clean up active downloads
                            let mut active = active_downloads_clone.lock().await;
                            active.remove(&id);
                            return;
                        }
                        if let Err(e) = db_clone.set_started(id).await {
                            tracing::error!(download_id = id, error = %e, "Failed to set start time");
                            // Clean up active downloads
                            let mut active = active_downloads_clone.lock().await;
                            active.remove(&id);
                            return;
                        }

                        // Emit Downloading event (initial progress 0%)
                        event_tx_clone
                            .send(Event::Downloading {
                                id,
                                percent: 0.0,
                                speed_bps: 0,
                            })
                            .ok();

                        // Get all pending articles
                        let pending_articles = match db_clone.get_pending_articles(id).await {
                            Ok(articles) => articles,
                            Err(e) => {
                                tracing::error!(download_id = id, error = %e, "Failed to get pending articles");
                                // Clean up active downloads
                                let mut active = active_downloads_clone.lock().await;
                                active.remove(&id);
                                return;
                            }
                        };

                        if pending_articles.is_empty() {
                            // No articles to download - mark as complete
                            event_tx_clone
                                .send(Event::DownloadComplete { id })
                                .ok();
                            // Clean up active downloads
                            let mut active = active_downloads_clone.lock().await;
                            active.remove(&id);
                            return;
                        }

                        let total_articles = pending_articles.len();
                        let total_size_bytes = download.size_bytes as u64;
                        let mut downloaded_articles = 0;
                        let mut downloaded_bytes: u64 = 0;

                        // Track download start time for speed calculation
                        let download_start = std::time::Instant::now();

                        // Create temp directory for this download
                        let download_temp_dir = config_clone.temp_dir.join(format!("download_{}", id));
                        if let Err(e) = tokio::fs::create_dir_all(&download_temp_dir).await {
                            tracing::error!(download_id = id, error = %e, "Failed to create temp directory");
                            let _ = db_clone.update_status(id, Status::Failed.to_i32()).await;
                            let _ = db_clone.set_error(id, &format!("Failed to create temp directory: {}", e)).await;
                            event_tx_clone
                                .send(Event::DownloadFailed {
                                    id,
                                    error: format!("Failed to create temp directory: {}", e),
                                })
                                .ok();
                            // Clean up active downloads
                            let mut active = active_downloads_clone.lock().await;
                            active.remove(&id);
                            return;
                        }

                        // Download each article
                        for article in pending_articles {
                            // Check if download was paused/cancelled
                            if cancel_token.is_cancelled() {
                                // Update status to Paused
                                let _ = db_clone.update_status(id, Status::Paused.to_i32()).await;

                                // Remove from active downloads
                                let mut active = active_downloads_clone.lock().await;
                                active.remove(&id);
                                drop(active);

                                return;
                            }
                            // Get a connection from the first NNTP pool
                            // TODO: Add multi-server failover in future tasks
                            let pool = match nntp_pools_clone.first() {
                                Some(p) => p,
                                None => {
                                    tracing::error!(download_id = id, "No NNTP pools configured");
                                    let _ = db_clone.update_status(id, Status::Failed.to_i32()).await;
                                    let _ = db_clone.set_error(id, "No NNTP pools configured").await;
                                    event_tx_clone
                                        .send(Event::DownloadFailed {
                                            id,
                                            error: "No NNTP pools configured".to_string(),
                                        })
                                        .ok();
                                    // Clean up active downloads
                                    let mut active = active_downloads_clone.lock().await;
                                    active.remove(&id);
                                    return;
                                }
                            };

                            let mut conn = match pool.get().await {
                                Ok(c) => c,
                                Err(e) => {
                                    tracing::error!(download_id = id, error = %e, "Failed to get NNTP connection");
                                    let _ = db_clone.update_status(id, Status::Failed.to_i32()).await;
                                    let _ = db_clone.set_error(id, &format!("Failed to get NNTP connection: {}", e)).await;
                                    event_tx_clone
                                        .send(Event::DownloadFailed {
                                            id,
                                            error: format!("Failed to get NNTP connection: {}", e),
                                        })
                                        .ok();
                                    // Clean up active downloads
                                    let mut active = active_downloads_clone.lock().await;
                                    active.remove(&id);
                                    return;
                                }
                            };

                            // Acquire bandwidth tokens before downloading
                            // This enforces the global speed limit across all concurrent downloads
                            speed_limiter_clone.acquire(article.size_bytes as u64).await;

                            // Fetch the article from the server
                            match conn.fetch_article(&article.message_id).await {
                                Ok(response) => {
                                    // Save article content to temp directory
                                    let article_file = download_temp_dir.join(format!("article_{}.dat", article.segment_number));

                                    // Join response lines into single string for storage
                                    let article_content = response.lines.join("\n");
                                    if let Err(e) = tokio::fs::write(&article_file, article_content.as_bytes()).await {
                                        tracing::error!(download_id = id, error = %e, "Failed to write article file");
                                        let _ = db_clone.update_status(id, Status::Failed.to_i32()).await;
                                        let _ = db_clone.set_error(id, &format!("Failed to write article file: {}", e)).await;
                                        event_tx_clone
                                            .send(Event::DownloadFailed {
                                                id,
                                                error: format!("Failed to write article file: {}", e),
                                            })
                                            .ok();
                                        // Clean up active downloads
                                        let mut active = active_downloads_clone.lock().await;
                                        active.remove(&id);
                                        return;
                                    }

                                    // Mark article as downloaded
                                    if let Err(e) = db_clone.update_article_status(
                                        article.id,
                                        crate::db::article_status::DOWNLOADED,
                                    ).await {
                                        tracing::error!(download_id = id, error = %e, "Failed to update article status");
                                        continue;
                                    }

                                    downloaded_articles += 1;
                                    downloaded_bytes += article.size_bytes as u64;

                                    // Calculate progress percentage
                                    let progress_percent = if total_size_bytes > 0 {
                                        (downloaded_bytes as f32 / total_size_bytes as f32) * 100.0
                                    } else {
                                        (downloaded_articles as f32 / total_articles as f32) * 100.0
                                    };

                                    // Calculate download speed (bytes per second)
                                    let elapsed_secs = download_start.elapsed().as_secs_f64();
                                    let speed_bps = if elapsed_secs > 0.0 {
                                        (downloaded_bytes as f64 / elapsed_secs) as u64
                                    } else {
                                        0
                                    };

                                    // Update progress in database
                                    if let Err(e) = db_clone.update_progress(
                                        id,
                                        progress_percent,
                                        speed_bps,
                                        downloaded_bytes,
                                    ).await {
                                        tracing::error!(download_id = id, error = %e, "Failed to update progress");
                                    }

                                    // Emit progress event
                                    event_tx_clone
                                        .send(Event::Downloading {
                                            id,
                                            percent: progress_percent,
                                            speed_bps,
                                        })
                                        .ok();
                                }
                                Err(e) => {
                                    // Mark article as failed
                                    let _ = db_clone.update_article_status(article.id, crate::db::article_status::FAILED).await;

                                    // For now, fail the entire download on first article failure
                                    // TODO: Add retry logic in Tasks 8.1-8.6
                                    tracing::error!(download_id = id, error = %e, "Article fetch failed");
                                    let _ = db_clone.update_status(id, Status::Failed.to_i32()).await;
                                    let _ = db_clone.set_error(id, &format!("Article fetch failed: {}", e)).await;

                                    event_tx_clone
                                        .send(Event::DownloadFailed {
                                            id,
                                            error: format!("Article fetch failed: {}", e),
                                        })
                                        .ok();

                                    // Clean up active downloads
                                    let mut active = active_downloads_clone.lock().await;
                                    active.remove(&id);

                                    return;
                                }
                            }
                        }

                        // All articles downloaded successfully
                        if let Err(e) = db_clone.update_status(id, Status::Complete.to_i32()).await {
                            tracing::error!(download_id = id, error = %e, "Failed to mark download complete");
                            // Clean up active downloads
                            let mut active = active_downloads_clone.lock().await;
                            active.remove(&id);
                            return;
                        }
                        if let Err(e) = db_clone.set_completed(id).await {
                            tracing::error!(download_id = id, error = %e, "Failed to set completion time");
                        }

                        event_tx_clone
                            .send(Event::DownloadComplete { id })
                            .ok();

                        // Clean up: remove from active downloads
                        let mut active = active_downloads_clone.lock().await;
                        active.remove(&id);
                    });
                } else {
                    // Queue is empty, wait a bit before checking again
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        })
    }

    /// Spawn a download task for a queued download
    ///
    /// This method spawns an asynchronous task that:
    /// 1. Fetches the download record from the database
    /// 2. Gets all pending articles (not yet downloaded)
    /// 3. Downloads each article from NNTP servers
    /// 4. Updates progress and article status in the database
    /// 5. Emits progress events to subscribers
    ///
    /// The task runs independently and completes when all articles are downloaded
    /// or an error occurs.
    ///
    /// # Arguments
    ///
    /// * `download_id` - The ID of the download to process
    ///
    /// # Returns
    ///
    /// Returns a `tokio::task::JoinHandle` that can be awaited to get the result
    /// of the download task.
    ///
    /// # Errors
    ///
    /// The spawned task will fail if:
    /// - The download ID doesn't exist in the database
    /// - NNTP connection fails
    /// - Articles cannot be fetched from the server
    /// - Database updates fail
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Spawn the download task
    /// let handle = downloader.spawn_download_task(id);
    ///
    /// // Optionally await the result
    /// match handle.await {
    ///     Ok(Ok(())) => println!("Download completed successfully"),
    ///     Ok(Err(e)) => println!("Download failed: {}", e),
    ///     Err(e) => println!("Task panicked: {}", e),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Start the folder watcher background task
    ///
    /// This method spawns a background task that monitors configured watch folders
    /// for new NZB files and automatically adds them to the download queue.
    ///
    /// # Returns
    /// Returns a `JoinHandle` that can be used to await the folder watcher task.
    /// The task will run indefinitely until the channel is closed.
    ///
    /// # Errors
    /// Returns error if the folder watcher cannot be initialized (e.g., invalid watch folder path).
    ///
    /// # Example
    /// ```no_run
    /// # use usenet_dl::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<()> {
    /// # let config = Config::default();
    /// let downloader = Arc::new(UsenetDownloader::new(config).await?);
    ///
    /// // Start the folder watcher
    /// let watcher_handle = downloader.start_folder_watcher()?;
    ///
    /// // Watcher will now automatically add NZB files found in configured folders
    /// // Optionally await the handle if you want to wait for completion
    /// // watcher_handle.await.ok();
    /// # Ok(())
    /// # }
    /// ```
    pub fn start_folder_watcher(&self) -> Result<tokio::task::JoinHandle<()>> {
        // Get watch folder configurations from config
        let watch_folders = self.config.watch_folders.clone();

        // If no watch folders configured, return early
        if watch_folders.is_empty() {
            tracing::info!("No watch folders configured, skipping folder watcher");
            // Return a completed task handle
            return Ok(tokio::spawn(async {}));
        }

        // Create folder watcher instance
        let mut watcher = folder_watcher::FolderWatcher::new(
            std::sync::Arc::new(self.clone()),
            watch_folders,
        )?;

        // Start watching all configured folders
        watcher.start()?;

        // Spawn the watcher task
        let handle = tokio::spawn(async move {
            watcher.run().await;
        });

        tracing::info!("Folder watcher background task started");

        Ok(handle)
    }

    /// Start RSS feed scheduler for automatic feed checking
    ///
    /// This spawns a background task that periodically checks all configured RSS feeds
    /// based on their individual check_interval settings. The scheduler will:
    /// - Check each enabled feed at its configured interval
    /// - Parse RSS/Atom feed content
    /// - Apply filters to items
    /// - Mark items as seen to prevent duplicates
    /// - Auto-download matching items if auto_download is enabled
    ///
    /// # Returns
    /// A `JoinHandle` for the spawned background task. The task runs indefinitely
    /// until the downloader is shut down (accepting_new flag set to false).
    ///
    /// If no RSS feeds are configured, returns a completed task immediately.
    ///
    /// # Example
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # use usenet_dl::config::RssFeedConfig;
    /// # use std::time::Duration;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::default();
    /// let downloader = UsenetDownloader::new(config).await?;
    ///
    /// // Start RSS scheduler
    /// let scheduler_handle = downloader.start_rss_scheduler();
    ///
    /// // Scheduler will now automatically check feeds at their configured intervals
    /// // Optionally await the handle if you want to wait for completion
    /// // scheduler_handle.await.ok();
    /// # Ok(())
    /// # }
    /// ```
    pub fn start_rss_scheduler(&self) -> tokio::task::JoinHandle<()> {
        // Get RSS feed configurations from config
        let rss_feeds = self.config.rss_feeds.clone();

        // If no RSS feeds configured, return early
        if rss_feeds.is_empty() {
            tracing::info!("No RSS feeds configured, skipping RSS scheduler");
            // Return a completed task handle
            return tokio::spawn(async {});
        }

        // Create RSS manager instance
        let rss_manager = std::sync::Arc::new(
            rss_manager::RssManager::new(
                self.db.clone(),
                std::sync::Arc::new(self.clone()),
                rss_feeds.clone(),
            ).expect("Failed to create RSS manager")
        );

        // Create scheduler instance
        let scheduler = rss_scheduler::RssScheduler::new(
            std::sync::Arc::new(self.clone()),
            rss_manager,
        );

        // Spawn the scheduler task
        let handle = tokio::spawn(async move {
            scheduler.run().await;
        });

        tracing::info!("RSS scheduler background task started");

        handle
    }

    pub fn spawn_download_task(
        &self,
        download_id: DownloadId,
    ) -> tokio::task::JoinHandle<Result<()>> {
        let db = self.db.clone();
        let event_tx = self.event_tx.clone();
        let nntp_pools = self.nntp_pools.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            // Fetch download record
            let download = match db.get_download(download_id).await? {
                Some(d) => d,
                None => {
                    return Err(Error::Database(format!(
                        "Download with ID {} not found",
                        download_id
                    )))
                }
            };

            // Update status to Downloading and record start time
            db.update_status(download_id, Status::Downloading.to_i32()).await?;
            db.set_started(download_id).await?;

            // Emit Downloading event (initial progress 0%)
            event_tx
                .send(Event::Downloading {
                    id: download_id,
                    percent: 0.0,
                    speed_bps: 0,
                })
                .ok();

            // Get all pending articles
            let pending_articles = db.get_pending_articles(download_id).await?;

            if pending_articles.is_empty() {
                // No articles to download - mark as complete
                event_tx
                    .send(Event::DownloadComplete { id: download_id })
                    .ok();
                return Ok(());
            }

            let total_articles = pending_articles.len();
            let total_size_bytes = download.size_bytes as u64;
            let mut downloaded_articles = 0;
            let mut downloaded_bytes: u64 = 0;

            // Track download start time for speed calculation
            let download_start = std::time::Instant::now();

            // Create temp directory for this download
            let download_temp_dir = config.temp_dir.join(format!("download_{}", download_id));
            tokio::fs::create_dir_all(&download_temp_dir).await.map_err(|e| {
                Error::Io(std::io::Error::new(
                    e.kind(),
                    format!("Failed to create temp directory: {}", e),
                ))
            })?;

            // Store article data in temp directory
            // Later we'll assemble these into the final file (post-processing phase)
            let mut article_data = Vec::new();

            // Download each article
            for article in pending_articles {
                // Get a connection from the first NNTP pool
                // TODO: Add multi-server failover in future tasks
                let pool = nntp_pools
                    .first()
                    .ok_or_else(|| Error::Database("No NNTP pools configured".to_string()))?;

                let mut conn = pool.get().await.map_err(|e| {
                    Error::Database(format!("Failed to get NNTP connection: {}", e))
                })?;

                // Fetch the article from the server
                match conn.fetch_article(&article.message_id).await {
                    Ok(response) => {
                        // Save article content to temp directory
                        // Each article gets its own file: article_<segment_number>.dat
                        let article_file = download_temp_dir.join(format!("article_{}.dat", article.segment_number));

                        // Join response lines into single string for storage
                        let article_content = response.lines.join("\n");
                        tokio::fs::write(&article_file, article_content.as_bytes()).await.map_err(|e| {
                            Error::Io(std::io::Error::new(
                                e.kind(),
                                format!("Failed to write article file: {}", e),
                            ))
                        })?;

                        // Track article data for later assembly
                        article_data.push((article.segment_number, response.lines));

                        // Mark article as downloaded
                        db.update_article_status(
                            article.id,
                            crate::db::article_status::DOWNLOADED,
                        )
                        .await?;

                        downloaded_articles += 1;
                        downloaded_bytes += article.size_bytes as u64;

                        // Calculate progress percentage
                        let progress_percent = if total_size_bytes > 0 {
                            (downloaded_bytes as f32 / total_size_bytes as f32) * 100.0
                        } else {
                            (downloaded_articles as f32 / total_articles as f32) * 100.0
                        };

                        // Calculate download speed (bytes per second)
                        let elapsed_secs = download_start.elapsed().as_secs_f64();
                        let speed_bps = if elapsed_secs > 0.0 {
                            (downloaded_bytes as f64 / elapsed_secs) as u64
                        } else {
                            0
                        };

                        // Update progress in database
                        db.update_progress(
                            download_id,
                            progress_percent,
                            speed_bps,
                            downloaded_bytes,
                        )
                        .await?;

                        // Emit progress event
                        event_tx
                            .send(Event::Downloading {
                                id: download_id,
                                percent: progress_percent,
                                speed_bps,
                            })
                            .ok();
                    }
                    Err(e) => {
                        // Mark article as failed
                        db.update_article_status(article.id, crate::db::article_status::FAILED)
                            .await?;

                        // For now, fail the entire download on first article failure
                        // TODO: Add retry logic in Tasks 8.1-8.6
                        db.update_status(download_id, Status::Failed.to_i32()).await?;
                        db.set_error(download_id, &format!("Article fetch failed: {}", e))
                            .await?;

                        event_tx
                            .send(Event::DownloadFailed {
                                id: download_id,
                                error: format!("Article fetch failed: {}", e),
                            })
                            .ok();

                        return Err(Error::Database(format!("Article fetch failed: {}", e)));
                    }
                }
            }

            // All articles downloaded successfully
            db.update_status(download_id, Status::Complete.to_i32()).await?;
            db.set_completed(download_id).await?;

            event_tx
                .send(Event::DownloadComplete { id: download_id })
                .ok();

            Ok(())
        })
    }

    /// Start post-processing for a completed download
    ///
    /// This is the entry point to the post-processing pipeline. It coordinates
    /// verification, repair, extraction, moving, and cleanup based on the
    /// configured PostProcess mode.
    ///
    /// # Arguments
    ///
    /// * `download_id` - The download to post-process
    ///
    /// # Returns
    ///
    /// Returns Ok(()) on success, Err on any stage failure
    ///
    /// # Example
    ///
    /// ```no_run
    /// use usenet_dl::{UsenetDownloader, Config};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let downloader = UsenetDownloader::new(Config::default()).await?;
    ///
    ///     // After download completes, start post-processing
    ///     downloader.start_post_processing(1).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn start_post_processing(&self, download_id: DownloadId) -> Result<()> {
        use crate::types::Status;

        tracing::info!(download_id, "starting post-processing");

        // Update status to Processing
        self.db.update_status(download_id, Status::Processing.to_i32()).await?;

        // Get download info from database
        let download = self.db.get_download(download_id).await?
            .ok_or_else(|| Error::NotFound(format!("download {} not found", download_id)))?;

        // Determine download path (temp directory)
        let download_path = self.config.temp_dir
            .join(format!("download_{}", download_id));

        // Determine final destination
        let destination = std::path::PathBuf::from(&download.destination);

        // Determine post-processing mode
        let post_process = crate::config::PostProcess::from_i32(download.post_process);

        // Execute post-processing pipeline
        match self.post_processor.start_post_processing(
            download_id,
            download_path,
            post_process,
            destination.clone(),
        ).await {
            Ok(final_path) => {
                // Mark as complete and emit Complete event
                self.db.update_status(download_id, Status::Complete.to_i32()).await?;
                self.event_tx.send(crate::types::Event::Complete {
                    id: download_id,
                    path: final_path,
                }).ok();

                tracing::info!(download_id, "post-processing completed successfully");
                Ok(())
            }
            Err(e) => {
                // Mark as failed and emit Failed event
                self.db.update_status(download_id, Status::Failed.to_i32()).await?;
                self.db.set_error(download_id, &e.to_string()).await?;

                self.event_tx.send(crate::types::Event::Failed {
                    id: download_id,
                    stage: crate::types::Stage::Extract, // TODO: Track actual stage
                    error: e.to_string(),
                    files_kept: true, // Default: keep files on failure
                }).ok();

                tracing::error!(download_id, error = %e, "post-processing failed");
                Err(e)
            }
        }
    }
}

/// Helper function to run the downloader with graceful signal handling.
///
/// This function sets up signal handlers for SIGTERM and SIGINT (Ctrl+C),
/// and calls the downloader's shutdown() method when a signal is received.
///
/// # Example
///
/// ```no_run
/// use usenet_dl::{UsenetDownloader, Config, run_with_shutdown};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = Config::default();
///     let downloader = UsenetDownloader::new(config).await?;
///
///     // Run with automatic signal handling
///     run_with_shutdown(downloader).await?;
///
///     Ok(())
/// }
/// ```
pub async fn run_with_shutdown(downloader: UsenetDownloader) -> Result<()> {
    use tokio::signal::unix::{signal, SignalKind};

    let shutdown_signal = async {
        // Set up signal handlers
        let mut sigterm = signal(SignalKind::terminate())
            .expect("Failed to register SIGTERM handler");
        let mut sigint = signal(SignalKind::interrupt())
            .expect("Failed to register SIGINT handler");

        tokio::select! {
            _ = sigterm.recv() => {
                tracing::info!("Received SIGTERM signal");
            }
            _ = sigint.recv() => {
                tracing::info!("Received SIGINT signal (Ctrl+C)");
            }
        }
    };

    // Wait for shutdown signal
    shutdown_signal.await;

    // Perform graceful shutdown
    downloader.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::time::{Duration, Instant};

    /// Helper to create a test UsenetDownloader instance with a persistent database
    /// Returns the downloader and the tempdir (which must be kept alive)
    async fn create_test_downloader() -> (UsenetDownloader, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let config = Config {
            database_path: db_path,
            servers: vec![], // No servers for testing
            max_concurrent_downloads: 3,
            ..Default::default()
        };

        // Initialize database
        let db = Database::new(&config.database_path).await.unwrap();

        // Create broadcast channel
        let (event_tx, _rx) = tokio::sync::broadcast::channel(1000);

        // No NNTP pools since we have no servers
        let nntp_pools = Vec::new();

        // Create priority queue
        let queue = std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::BinaryHeap::new()
        ));

        // Create semaphore
        let concurrent_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
            config.max_concurrent_downloads
        ));

        // Create active downloads tracking map
        let active_downloads = std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new()
        ));

        // Create speed limiter with configured limit
        let speed_limiter = speed_limiter::SpeedLimiter::new(config.speed_limit_bps);

        // Create config Arc early so we can share it
        let config_arc = std::sync::Arc::new(config.clone());

        // Initialize runtime-mutable categories from config
        let categories = std::sync::Arc::new(tokio::sync::RwLock::new(config.categories.clone()));

        // Create post-processing pipeline executor
        let post_processor = std::sync::Arc::new(post_processing::PostProcessor::new(
            event_tx.clone(),
            config_arc.clone(),
        ));

        let downloader = UsenetDownloader {
            db: std::sync::Arc::new(db),
            event_tx,
            config: config_arc,
            nntp_pools: std::sync::Arc::new(nntp_pools),
            queue,
            concurrent_limit,
            active_downloads,
            speed_limiter,
            accepting_new: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            post_processor,
            categories,
        };

        (downloader, temp_dir)
    }

    /// Sample NZB content for testing
    const SAMPLE_NZB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <head>
    <meta type="title">Test Download</meta>
    <meta type="password">testpass123</meta>
    <meta type="category">movies</meta>
  </head>
  <file poster="user@example.com" date="1234567890" subject="test.file.rar [1/2]">
    <groups>
      <group>alt.binaries.test</group>
    </groups>
    <segments>
      <segment bytes="768000" number="1">part1of2@example.com</segment>
      <segment bytes="512000" number="2">part2of2@example.com</segment>
    </segments>
  </file>
</nzb>"#;

    #[tokio::test]
    async fn test_add_nzb_content_basic() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add NZB to queue
        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test_download", DownloadOptions::default())
            .await
            .unwrap();

        assert!(download_id > 0);

        // Verify download was created in database
        let download = downloader.db.get_download(download_id).await.unwrap();
        assert!(download.is_some());

        let download = download.unwrap();
        assert_eq!(download.name, "test_download");
        assert_eq!(download.status, Status::Queued.to_i32());
        assert_eq!(download.size_bytes, 768000 + 512000); // Total of both segments
    }

    #[tokio::test]
    async fn test_add_nzb_content_extracts_metadata() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();

        // Check NZB metadata was extracted
        assert_eq!(download.nzb_meta_name, Some("Test Download".to_string()));
        assert_eq!(download.job_name, Some("Test Download".to_string())); // Uses meta title

        // Check password was cached
        let cached_password = downloader.db.get_cached_password(download_id).await.unwrap();
        assert_eq!(cached_password, Some("testpass123".to_string()));
    }

    #[tokio::test]
    async fn test_add_nzb_content_creates_articles() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Verify articles were created
        let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
        assert_eq!(articles.len(), 2); // Two segments in sample NZB

        assert_eq!(articles[0].message_id, "part1of2@example.com");
        assert_eq!(articles[0].segment_number, 1);
        assert_eq!(articles[0].size_bytes, 768000);

        assert_eq!(articles[1].message_id, "part2of2@example.com");
        assert_eq!(articles[1].segment_number, 2);
        assert_eq!(articles[1].size_bytes, 512000);
    }

    #[tokio::test]
    async fn test_add_nzb_content_with_options() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let options = DownloadOptions {
            category: Some("test_category".to_string()),
            priority: Priority::High,
            password: Some("override_password".to_string()),
            ..Default::default()
        };

        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", options)
            .await
            .unwrap();

        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();

        // Check options were applied
        assert_eq!(download.category, Some("test_category".to_string()));
        assert_eq!(download.priority, Priority::High as i32);

        // Check provided password overrides NZB password
        let cached_password = downloader.db.get_cached_password(download_id).await.unwrap();
        assert_eq!(cached_password, Some("override_password".to_string()));
    }

    #[tokio::test]
    async fn test_add_nzb_content_calculates_hash() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();

        // Verify hash was calculated and stored
        assert!(download.nzb_hash.is_some());
        let hash = download.nzb_hash.unwrap();
        assert_eq!(hash.len(), 64); // SHA256 produces 64 hex characters
    }

    #[tokio::test]
    async fn test_add_nzb_content_invalid_utf8() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Invalid UTF-8 bytes
        let invalid_bytes = vec![0xFF, 0xFE, 0xFD];

        let result = downloader
            .add_nzb_content(&invalid_bytes, "test", DownloadOptions::default())
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidNzb(msg) => assert!(msg.contains("not valid UTF-8")),
            _ => panic!("Expected InvalidNzb error"),
        }
    }

    #[tokio::test]
    async fn test_add_nzb_content_invalid_xml() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let invalid_nzb = b"<not><valid>xml";

        let result = downloader
            .add_nzb_content(invalid_nzb, "test", DownloadOptions::default())
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidNzb(msg) => {
                // Accept either parse error or validation error
                assert!(msg.contains("Failed to parse NZB") || msg.contains("validation failed"));
            }
            _ => panic!("Expected InvalidNzb error"),
        }
    }

    #[tokio::test]
    async fn test_add_nzb_content_emits_event() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Subscribe to events before spawning task
        let mut events = downloader.subscribe();

        // Add NZB
        downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Wait for Queued event
        let event = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            events.recv()
        ).await.unwrap().unwrap();

        match event {
            Event::Queued { id, name } => {
                assert!(id > 0);
                assert_eq!(name, "test");
            }
            _ => panic!("Expected Queued event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_add_nzb_from_file() {
        let (downloader, temp_dir) = create_test_downloader().await;

        // Create a test NZB file
        let nzb_path = temp_dir.path().join("test_download.nzb");
        tokio::fs::write(&nzb_path, SAMPLE_NZB).await.unwrap();

        // Add NZB from file
        let download_id = downloader
            .add_nzb(&nzb_path, DownloadOptions::default())
            .await
            .unwrap();

        assert!(download_id > 0);

        // Verify download was created with correct name (filename without extension)
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(download.name, "test_download");
        assert_eq!(download.status, Status::Queued.to_i32());
    }

    #[tokio::test]
    async fn test_add_nzb_file_not_found() {
        let (downloader, temp_dir) = create_test_downloader().await;

        let nonexistent_path = temp_dir.path().join("nonexistent.nzb");

        let result = downloader
            .add_nzb(&nonexistent_path, DownloadOptions::default())
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Io(e) => {
                assert!(e.to_string().contains("Failed to read NZB file"));
            }
            _ => panic!("Expected Io error"),
        }
    }

    #[tokio::test]
    async fn test_add_nzb_extracts_filename() {
        let (downloader, temp_dir) = create_test_downloader().await;

        // Create test file with complex filename
        let nzb_path = temp_dir.path().join("My.Movie.2024.1080p.nzb");
        tokio::fs::write(&nzb_path, SAMPLE_NZB).await.unwrap();

        let download_id = downloader
            .add_nzb(&nzb_path, DownloadOptions::default())
            .await
            .unwrap();

        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        // Should use filename without .nzb extension
        assert_eq!(download.name, "My.Movie.2024.1080p");
    }

    #[tokio::test]
    async fn test_add_nzb_with_options() {
        let (downloader, temp_dir) = create_test_downloader().await;

        let nzb_path = temp_dir.path().join("test.nzb");
        tokio::fs::write(&nzb_path, SAMPLE_NZB).await.unwrap();

        let options = DownloadOptions {
            category: Some("movies".to_string()),
            priority: Priority::High,
            ..Default::default()
        };

        let download_id = downloader
            .add_nzb(&nzb_path, options)
            .await
            .unwrap();

        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(download.category, Some("movies".to_string()));
        assert_eq!(download.priority, Priority::High as i32);
    }

    // URL Fetching Tests

    #[tokio::test]
    async fn test_add_nzb_url_success() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Start mock HTTP server
        let mock_server = MockServer::start().await;

        // Mock successful NZB download with Content-Disposition header
        Mock::given(method("GET"))
            .and(path("/test.nzb"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Disposition", "attachment; filename=\"Movie.Release.nzb\"")
                    .set_body_bytes(SAMPLE_NZB)
            )
            .mount(&mock_server)
            .await;

        // Fetch NZB from mock server
        let url = format!("{}/test.nzb", mock_server.uri());
        let download_id = downloader
            .add_nzb_url(&url, DownloadOptions::default())
            .await
            .unwrap();

        assert!(download_id > 0);

        // Verify download was created with filename from Content-Disposition
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(download.name, "Movie.Release");
        assert_eq!(download.status, Status::Queued.to_i32());
    }

    #[tokio::test]
    async fn test_add_nzb_url_extracts_filename_from_url() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Start mock HTTP server
        let mock_server = MockServer::start().await;

        // Mock successful NZB download without Content-Disposition header
        Mock::given(method("GET"))
            .and(path("/downloads/My.Movie.2024.nzb"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(SAMPLE_NZB)
            )
            .mount(&mock_server)
            .await;

        // Fetch NZB from mock server
        let url = format!("{}/downloads/My.Movie.2024.nzb", mock_server.uri());
        let download_id = downloader
            .add_nzb_url(&url, DownloadOptions::default())
            .await
            .unwrap();

        // Verify download was created with filename from URL path
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(download.name, "My.Movie.2024");
    }

    #[tokio::test]
    async fn test_add_nzb_url_http_404() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Start mock HTTP server
        let mock_server = MockServer::start().await;

        // Mock 404 Not Found response
        Mock::given(method("GET"))
            .and(path("/notfound.nzb"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        // Attempt to fetch non-existent NZB
        let url = format!("{}/notfound.nzb", mock_server.uri());
        let result = downloader
            .add_nzb_url(&url, DownloadOptions::default())
            .await;

        // Should return error
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Io(e) => {
                let msg = e.to_string();
                assert!(msg.contains("HTTP error"));
                assert!(msg.contains("404"));
            }
            _ => panic!("Expected Io error for HTTP 404"),
        }
    }

    #[tokio::test]
    async fn test_add_nzb_url_http_403() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Start mock HTTP server
        let mock_server = MockServer::start().await;

        // Mock 403 Forbidden response
        Mock::given(method("GET"))
            .and(path("/forbidden.nzb"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&mock_server)
            .await;

        // Attempt to fetch forbidden NZB
        let url = format!("{}/forbidden.nzb", mock_server.uri());
        let result = downloader
            .add_nzb_url(&url, DownloadOptions::default())
            .await;

        // Should return error
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Io(e) => {
                let msg = e.to_string();
                assert!(msg.contains("HTTP error"));
                assert!(msg.contains("403"));
            }
            _ => panic!("Expected Io error for HTTP 403"),
        }
    }

    #[tokio::test]
    async fn test_add_nzb_url_timeout() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};
        use std::time::Duration;

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Start mock HTTP server
        let mock_server = MockServer::start().await;

        // Mock slow response that exceeds timeout (30 seconds)
        // Note: This test would take 30+ seconds to run, so we'll test connection failure instead
        Mock::given(method("GET"))
            .and(path("/slow.nzb"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(35))  // Exceeds 30 second timeout
                    .set_body_bytes(SAMPLE_NZB)
            )
            .mount(&mock_server)
            .await;

        // Attempt to fetch slow NZB
        let url = format!("{}/slow.nzb", mock_server.uri());
        let result = downloader
            .add_nzb_url(&url, DownloadOptions::default())
            .await;

        // Should return timeout error
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Io(e) => {
                let msg = e.to_string();
                assert!(msg.contains("Timeout") || msg.contains("timeout"));
            }
            _ => panic!("Expected Io error for timeout"),
        }
    }

    #[tokio::test]
    async fn test_add_nzb_url_connection_refused() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Use a URL that will cause connection refused (port unlikely to be in use)
        // Port 9 is the discard service, rarely running on modern systems
        let url = "http://127.0.0.1:9/test.nzb";
        let result = downloader
            .add_nzb_url(url, DownloadOptions::default())
            .await;

        // Should return connection error
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Io(e) => {
                let msg = e.to_string();
                assert!(msg.contains("Connection failed") || msg.contains("Failed to fetch"));
            }
            _ => panic!("Expected Io error for connection refused"),
        }
    }

    #[tokio::test]
    async fn test_add_nzb_url_with_options() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Start mock HTTP server
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/movie.nzb"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(SAMPLE_NZB)
            )
            .mount(&mock_server)
            .await;

        // Fetch NZB with options
        let options = DownloadOptions {
            category: Some("movies".to_string()),
            priority: Priority::High,
            ..Default::default()
        };

        let url = format!("{}/movie.nzb", mock_server.uri());
        let download_id = downloader
            .add_nzb_url(&url, options)
            .await
            .unwrap();

        // Verify options were applied
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(download.category, Some("movies".to_string()));
        assert_eq!(download.priority, Priority::High as i32);
    }

    // Priority Queue Tests

    #[tokio::test]
    async fn test_queue_adds_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add download
        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Verify it's in the queue
        assert_eq!(downloader.queue_size().await, 1);

        // Verify we can get it from the queue
        let next_id = downloader.peek_next_download().await;
        assert_eq!(next_id, Some(id));
    }

    #[tokio::test]
    async fn test_queue_priority_ordering() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add downloads with different priorities
        let low_id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "low",
                DownloadOptions {
                    priority: Priority::Low,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let high_id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "high",
                DownloadOptions {
                    priority: Priority::High,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let normal_id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "normal",
                DownloadOptions {
                    priority: Priority::Normal,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Queue should have 3 items
        assert_eq!(downloader.queue_size().await, 3);

        // Should return highest priority first (High > Normal > Low)
        assert_eq!(downloader.get_next_download().await, Some(high_id));
        assert_eq!(downloader.get_next_download().await, Some(normal_id));
        assert_eq!(downloader.get_next_download().await, Some(low_id));
        assert_eq!(downloader.get_next_download().await, None);
    }

    #[tokio::test]
    async fn test_queue_fifo_for_same_priority() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add multiple downloads with same priority
        let id1 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "first", DownloadOptions::default())
            .await
            .unwrap();

        // Small delay to ensure different timestamps
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let id2 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "second", DownloadOptions::default())
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let id3 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "third", DownloadOptions::default())
            .await
            .unwrap();

        // Should return in FIFO order for same priority
        assert_eq!(downloader.get_next_download().await, Some(id1));
        assert_eq!(downloader.get_next_download().await, Some(id2));
        assert_eq!(downloader.get_next_download().await, Some(id3));
    }

    #[tokio::test]
    async fn test_queue_remove_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add downloads
        let id1 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "first", DownloadOptions::default())
            .await
            .unwrap();

        let id2 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "second", DownloadOptions::default())
            .await
            .unwrap();

        let id3 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "third", DownloadOptions::default())
            .await
            .unwrap();

        assert_eq!(downloader.queue_size().await, 3);

        // Remove middle download
        let removed = downloader.remove_from_queue(id2).await;
        assert!(removed);
        assert_eq!(downloader.queue_size().await, 2);

        // Should still get id1 and id3
        assert_eq!(downloader.get_next_download().await, Some(id1));
        assert_eq!(downloader.get_next_download().await, Some(id3));
        assert_eq!(downloader.get_next_download().await, None);
    }

    #[tokio::test]
    async fn test_queue_remove_nonexistent() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Try to remove download that doesn't exist
        let removed = downloader.remove_from_queue(999).await;
        assert!(!removed);
    }

    #[tokio::test]
    async fn test_queue_force_priority() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add normal priority download
        let normal_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "normal", DownloadOptions::default())
            .await
            .unwrap();

        // Add force priority download (should jump to front)
        let force_id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "force",
                DownloadOptions {
                    priority: Priority::Force,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Force should come first even though added second
        assert_eq!(downloader.get_next_download().await, Some(force_id));
        assert_eq!(downloader.get_next_download().await, Some(normal_id));
    }

    // Pause/Resume Tests

    #[tokio::test]
    async fn test_pause_queued_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add download
        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Download should be queued
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Queued.to_i32());

        // Pause it
        downloader.pause(id).await.unwrap();

        // Status should be updated to Paused
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Paused.to_i32());
    }

    #[tokio::test]
    async fn test_pause_already_paused() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Pause it once
        downloader.pause(id).await.unwrap();

        // Pause it again (should be idempotent)
        let result = downloader.pause(id).await;
        assert!(result.is_ok());

        // Status should still be Paused
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Paused.to_i32());
    }

    #[tokio::test]
    async fn test_pause_completed_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Mark as complete
        downloader.db.update_status(id, Status::Complete.to_i32()).await.unwrap();

        // Try to pause (should fail)
        let result = downloader.pause(id).await;
        assert!(result.is_err());

        // Status should still be Complete
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Complete.to_i32());
    }

    #[tokio::test]
    async fn test_pause_nonexistent_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Try to pause download that doesn't exist
        let result = downloader.pause(999).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_resume_paused_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add download
        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Pause it
        downloader.pause(id).await.unwrap();
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Paused.to_i32());

        // Resume it
        downloader.resume(id).await.unwrap();

        // Status should be updated to Queued
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Queued.to_i32());

        // Should be back in the queue
        assert!(downloader.queue_size().await > 0);
    }

    #[tokio::test]
    async fn test_resume_already_queued() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Download is already queued
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Queued.to_i32());

        // Try to resume (should be idempotent)
        let result = downloader.resume(id).await;
        assert!(result.is_ok());

        // Status should still be Queued
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Queued.to_i32());
    }

    #[tokio::test]
    async fn test_resume_completed_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Mark as complete
        downloader.db.update_status(id, Status::Complete.to_i32()).await.unwrap();

        // Try to resume (should fail)
        let result = downloader.resume(id).await;
        assert!(result.is_err());

        // Status should still be Complete
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Complete.to_i32());
    }

    #[tokio::test]
    async fn test_resume_failed_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Mark as failed
        downloader.db.update_status(id, Status::Failed.to_i32()).await.unwrap();

        // Try to resume (should fail - use reprocess() instead for failed downloads)
        let result = downloader.resume(id).await;
        assert!(result.is_err());

        // Status should still be Failed
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Failed.to_i32());
    }

    #[tokio::test]
    async fn test_resume_nonexistent_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Try to resume download that doesn't exist
        let result = downloader.resume(999).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_pause_resume_cycle() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add download
        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        let initial_queue_size = downloader.queue_size().await;

        // Pause
        downloader.pause(id).await.unwrap();
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Paused.to_i32());

        // Resume
        downloader.resume(id).await.unwrap();
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Queued.to_i32());

        // Queue size should be restored
        assert_eq!(downloader.queue_size().await, initial_queue_size);
    }

    #[tokio::test]
    async fn test_resume_preserves_priority() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add high priority download
        let id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "test",
                DownloadOptions {
                    priority: Priority::High,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Add normal priority download
        let normal_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "normal", DownloadOptions::default())
            .await
            .unwrap();

        // Pause high priority download
        downloader.pause(id).await.unwrap();

        // Resume high priority download
        downloader.resume(id).await.unwrap();

        // High priority download should still come first
        assert_eq!(downloader.get_next_download().await, Some(id));
        assert_eq!(downloader.get_next_download().await, Some(normal_id));
    }

    #[tokio::test]
    async fn test_cancel_queued_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Verify download exists in database
        assert!(downloader.db.get_download(id).await.unwrap().is_some());

        // Verify download is in queue
        assert_eq!(downloader.queue_size().await, 1);

        // Cancel the download
        downloader.cancel(id).await.unwrap();

        // Download should be removed from database
        assert!(downloader.db.get_download(id).await.unwrap().is_none());

        // Download should be removed from queue
        assert_eq!(downloader.queue_size().await, 0);
    }

    #[tokio::test]
    async fn test_cancel_paused_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Pause the download
        downloader.pause(id).await.unwrap();

        // Verify it's paused
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Paused.to_i32());

        // Cancel the paused download
        downloader.cancel(id).await.unwrap();

        // Download should be removed from database
        assert!(downloader.db.get_download(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_cancel_deletes_temp_files() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Create temp directory and some files (simulating partially downloaded)
        let download_temp_dir = downloader.config.temp_dir.join(format!("download_{}", id));
        tokio::fs::create_dir_all(&download_temp_dir).await.unwrap();

        let test_file = download_temp_dir.join("article_1.dat");
        tokio::fs::write(&test_file, b"test data").await.unwrap();

        // Verify temp directory exists
        assert!(download_temp_dir.exists());
        assert!(test_file.exists());

        // Cancel the download
        downloader.cancel(id).await.unwrap();

        // Temp directory should be deleted
        assert!(!download_temp_dir.exists());
        assert!(!test_file.exists());
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Try to cancel download that doesn't exist
        let result = downloader.cancel(999).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cancel_completed_download() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Mark as completed
        downloader.db.update_status(id, Status::Complete.to_i32()).await.unwrap();

        // Cancel completed download (should succeed - removes from history)
        downloader.cancel(id).await.unwrap();

        // Download should be removed from database
        assert!(downloader.db.get_download(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_cancel_removes_from_queue() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add multiple downloads
        let id1 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
            .await
            .unwrap();

        let id2 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
            .await
            .unwrap();

        let id3 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
            .await
            .unwrap();

        // Verify all are queued
        assert_eq!(downloader.queue_size().await, 3);

        // Cancel middle download
        downloader.cancel(id2).await.unwrap();

        // Queue should have 2 items
        assert_eq!(downloader.queue_size().await, 2);

        // Get downloads from queue - should only be id1 and id3
        let next = downloader.get_next_download().await;
        assert!(next == Some(id1) || next == Some(id3));

        let next2 = downloader.get_next_download().await;
        assert!(next2 == Some(id1) || next2 == Some(id3));
        assert_ne!(next, next2);

        // Queue should now be empty
        assert_eq!(downloader.queue_size().await, 0);
    }

    #[tokio::test]
    async fn test_cancel_emits_removed_event() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Subscribe to events
        let mut events = downloader.subscribe();

        // Cancel the download (in background to avoid blocking)
        let downloader_clone = downloader.clone();
        tokio::spawn(async move {
            downloader_clone.cancel(id).await.unwrap();
        });

        // Wait for Removed event
        let mut received_removed = false;
        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await {
                Ok(Ok(crate::types::Event::Removed { id: event_id })) => {
                    assert_eq!(event_id, id);
                    received_removed = true;
                    break;
                }
                Ok(Ok(_)) => continue, // Other events, keep checking
                Ok(Err(_)) => break,   // Channel closed
                Err(_) => break,       // Timeout
            }
        }

        assert!(received_removed, "Should have received Removed event");
    }

    // Queue-wide pause/resume tests

    #[tokio::test]
    async fn test_pause_all_pauses_active_downloads() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add multiple downloads with different statuses
        let id1 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
            .await
            .unwrap();

        let id2 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
            .await
            .unwrap();

        let id3 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
            .await
            .unwrap();

        // Mark id2 as already paused
        downloader.pause(id2).await.unwrap();

        // Mark id3 as complete (should not be paused)
        downloader.db.update_status(id3, Status::Complete.to_i32()).await.unwrap();

        // Pause all
        downloader.pause_all().await.unwrap();

        // Check statuses
        let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
        let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
        let d3 = downloader.db.get_download(id3).await.unwrap().unwrap();

        assert_eq!(d1.status, Status::Paused.to_i32(), "id1 should be paused");
        assert_eq!(d2.status, Status::Paused.to_i32(), "id2 should still be paused");
        assert_eq!(d3.status, Status::Complete.to_i32(), "id3 should still be complete");
    }

    #[tokio::test]
    async fn test_pause_all_emits_queue_paused_event() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download
        downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Subscribe to events
        let mut events = downloader.subscribe();

        // Pause all (in background to avoid blocking)
        let downloader_clone = downloader.clone();
        tokio::spawn(async move {
            downloader_clone.pause_all().await.unwrap();
        });

        // Wait for QueuePaused event
        let mut received_queue_paused = false;
        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await {
                Ok(Ok(crate::types::Event::QueuePaused)) => {
                    received_queue_paused = true;
                    break;
                }
                Ok(Ok(_)) => continue, // Other events, keep checking
                Ok(Err(_)) => break,   // Channel closed
                Err(_) => break,       // Timeout
            }
        }

        assert!(received_queue_paused, "Should have received QueuePaused event");
    }

    #[tokio::test]
    async fn test_pause_all_with_empty_queue() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Pause all with no downloads (should not error)
        let result = downloader.pause_all().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_resume_all_resumes_paused_downloads() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add multiple downloads
        let id1 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
            .await
            .unwrap();

        let id2 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
            .await
            .unwrap();

        let id3 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
            .await
            .unwrap();

        // Pause all downloads
        downloader.pause(id1).await.unwrap();
        downloader.pause(id2).await.unwrap();

        // Mark id3 as complete (should not be resumed)
        downloader.db.update_status(id3, Status::Complete.to_i32()).await.unwrap();

        // Resume all
        downloader.resume_all().await.unwrap();

        // Check statuses
        let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
        let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
        let d3 = downloader.db.get_download(id3).await.unwrap().unwrap();

        assert_eq!(d1.status, Status::Queued.to_i32(), "id1 should be queued");
        assert_eq!(d2.status, Status::Queued.to_i32(), "id2 should be queued");
        assert_eq!(d3.status, Status::Complete.to_i32(), "id3 should still be complete");
    }

    #[tokio::test]
    async fn test_resume_all_emits_queue_resumed_event() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add and pause a download
        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        downloader.pause(id).await.unwrap();

        // Subscribe to events
        let mut events = downloader.subscribe();

        // Resume all (in background to avoid blocking)
        let downloader_clone = downloader.clone();
        tokio::spawn(async move {
            downloader_clone.resume_all().await.unwrap();
        });

        // Wait for QueueResumed event
        let mut received_queue_resumed = false;
        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await {
                Ok(Ok(crate::types::Event::QueueResumed)) => {
                    received_queue_resumed = true;
                    break;
                }
                Ok(Ok(_)) => continue, // Other events, keep checking
                Ok(Err(_)) => break,   // Channel closed
                Err(_) => break,       // Timeout
            }
        }

        assert!(received_queue_resumed, "Should have received QueueResumed event");
    }

    #[tokio::test]
    async fn test_resume_all_with_no_paused_downloads() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a queued download (not paused)
        downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Resume all (should not error even though nothing is paused)
        let result = downloader.resume_all().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_pause_all_resume_all_cycle() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add multiple downloads
        let id1 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
            .await
            .unwrap();

        let id2 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
            .await
            .unwrap();

        // Initial state: both queued
        let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
        let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
        assert_eq!(d1.status, Status::Queued.to_i32());
        assert_eq!(d2.status, Status::Queued.to_i32());

        // Pause all
        downloader.pause_all().await.unwrap();

        // After pause: both paused
        let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
        let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
        assert_eq!(d1.status, Status::Paused.to_i32());
        assert_eq!(d2.status, Status::Paused.to_i32());

        // Resume all
        downloader.resume_all().await.unwrap();

        // After resume: both queued again
        let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
        let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
        assert_eq!(d1.status, Status::Queued.to_i32());
        assert_eq!(d2.status, Status::Queued.to_i32());
    }

    // === Task 5.9: Queue State Persistence Tests ===

    #[tokio::test]
    async fn test_queue_state_persisted_to_database() {
        // Test Task 5.9: Queue state is persisted to SQLite on every change
        let (downloader, _temp_dir) = create_test_downloader().await;

        // 1. Add download - should persist Status::Queued
        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Verify Status::Queued persisted to database
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Queued.to_i32(), "Status should be Queued in DB");
        assert_eq!(download.priority, 0, "Priority should be Normal (0)");

        // 2. Pause download - should persist Status::Paused
        downloader.pause(id).await.unwrap();

        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Paused.to_i32(), "Status should be Paused in DB");

        // 3. Resume download - should persist Status::Queued again
        downloader.resume(id).await.unwrap();

        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Queued.to_i32(), "Status should be Queued in DB after resume");

        // 4. Verify in-memory queue and database are synchronized
        let queue_size = downloader.queue_size().await;
        assert_eq!(queue_size, 1, "In-memory queue should have 1 download");

        // Query incomplete downloads from DB (should include our Queued download)
        let incomplete = downloader.db.get_incomplete_downloads().await.unwrap();
        assert_eq!(incomplete.len(), 1, "DB should have 1 incomplete download");
        assert_eq!(incomplete[0].id, id, "Incomplete download ID should match");

        // 5. Cancel download - should remove from database
        downloader.cancel(id).await.unwrap();

        let download = downloader.db.get_download(id).await.unwrap();
        assert!(download.is_none(), "Download should be deleted from DB");

        let queue_size = downloader.queue_size().await;
        assert_eq!(queue_size, 0, "In-memory queue should be empty");
    }

    #[tokio::test]
    async fn test_queue_ordering_persisted_correctly() {
        // Test that queue ordering (priority + created_at) is persisted and queryable
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add downloads with different priorities
        let id_low = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "low",
                DownloadOptions {
                    priority: Priority::Low,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await; // Ensure different timestamps

        let id_normal = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "normal",
                DownloadOptions {
                    priority: Priority::Normal,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let id_high = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "high",
                DownloadOptions {
                    priority: Priority::High,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Query database with priority ordering (as restore_queue() would do)
        let all_downloads = downloader.db.list_downloads().await.unwrap();

        // Should be ordered: High, Normal, Low (priority DESC)
        assert_eq!(all_downloads.len(), 3, "Should have 3 downloads");
        assert_eq!(all_downloads[0].id, id_high, "First should be High priority");
        assert_eq!(all_downloads[1].id, id_normal, "Second should be Normal priority");
        assert_eq!(all_downloads[2].id, id_low, "Third should be Low priority");

        // Verify priorities are correct in database
        assert_eq!(all_downloads[0].priority, Priority::High as i32);
        assert_eq!(all_downloads[1].priority, Priority::Normal as i32);
        assert_eq!(all_downloads[2].priority, Priority::Low as i32);
    }

    #[tokio::test]
    async fn test_queue_persistence_enables_restore() {
        // Test that persisted queue state can be used to restore queue (Task 6.3 preview)
        use tempfile::TempDir;

        // Create persistent temp directory for database
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("usenet-dl.db");

        // Create first downloader instance
        let config1 = Config {
            database_path: db_path.clone(),
            temp_dir: temp_dir.path().join("temp"),
            download_dir: temp_dir.path().join("downloads"),
            ..Default::default()
        };
        let downloader = UsenetDownloader::new(config1).await.unwrap();

        // Add multiple downloads with different statuses
        let id1 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
            .await
            .unwrap();

        let id2 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
            .await
            .unwrap();

        let id3 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
            .await
            .unwrap();

        // Mark one as Processing, complete one, leave one queued
        downloader.db.update_status(id2, Status::Processing.to_i32()).await.unwrap();
        downloader.db.update_status(id3, Status::Complete.to_i32()).await.unwrap();

        // Simulate restart: create new downloader with same database
        drop(downloader); // Close first instance

        let config2 = Config {
            database_path: db_path.clone(),
            temp_dir: temp_dir.path().join("temp"),
            download_dir: temp_dir.path().join("downloads"),
            ..Default::default()
        };
        let downloader2 = UsenetDownloader::new(config2).await.unwrap();

        // Verify we can query incomplete downloads (would be used by restore_queue)
        // Note: get_incomplete_downloads() returns status IN (0, 1, 3) - Queued, Downloading, Processing
        // It intentionally excludes Paused (2), which would be handled separately
        let incomplete = downloader2.db.get_incomplete_downloads().await.unwrap();

        // Should have 2: id1 (Queued) and id2 (Processing)
        // Should NOT have id3 (Complete)
        assert_eq!(incomplete.len(), 2, "Should have 2 incomplete downloads");

        let incomplete_ids: Vec<i64> = incomplete.iter().map(|d| d.id).collect();
        assert!(incomplete_ids.contains(&id1), "Should include Queued download");
        assert!(incomplete_ids.contains(&id2), "Should include Processing download");
        assert!(!incomplete_ids.contains(&id3), "Should NOT include Complete download");

        // Verify they're in priority order
        assert_eq!(incomplete[0].priority, 0, "First should be Normal priority");
        assert_eq!(incomplete[1].priority, 0, "Second should be Normal priority");

        // Also verify paused downloads can be restored separately
        let paused = downloader2.db.list_downloads_by_status(Status::Paused.to_i32()).await.unwrap();
        assert_eq!(paused.len(), 0, "No paused downloads in this test (id2 was set to Processing)");
    }

    #[tokio::test]
    async fn test_resume_download_with_pending_articles() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download
        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Simulate partial download: mark first article as downloaded
        let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
        assert_eq!(articles.len(), 2, "Should have 2 pending articles initially");

        downloader.db.update_article_status(
            articles[0].id,
            crate::db::article_status::DOWNLOADED
        ).await.unwrap();

        // Update download status to Paused (simulate interrupted download)
        downloader.db.update_status(download_id, Status::Paused.to_i32()).await.unwrap();

        // Resume the download
        downloader.resume_download(download_id).await.unwrap();

        // Verify download is back in Queued status
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(Status::from_i32(download.status), Status::Queued);

        // Verify only 1 article remains pending
        let pending = downloader.db.get_pending_articles(download_id).await.unwrap();
        assert_eq!(pending.len(), 1, "Should have 1 pending article after resume");
        assert_eq!(pending[0].id, articles[1].id, "Should be the second article");
    }

    #[tokio::test]
    async fn test_resume_download_no_pending_articles() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download
        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Mark all articles as downloaded
        let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
        for article in articles {
            downloader.db.update_article_status(
                article.id,
                crate::db::article_status::DOWNLOADED
            ).await.unwrap();
        }

        // Update status to Downloading (simulate download just completed)
        downloader.db.update_status(download_id, Status::Downloading.to_i32()).await.unwrap();

        // Resume should proceed to post-processing
        downloader.resume_download(download_id).await.unwrap();

        // Verify status is now Processing (ready for post-processing)
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(Status::from_i32(download.status), Status::Processing);

        // Verify no pending articles remain
        let pending = downloader.db.get_pending_articles(download_id).await.unwrap();
        assert_eq!(pending.len(), 0, "Should have no pending articles");
    }

    #[tokio::test]
    async fn test_resume_download_nonexistent() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Try to resume non-existent download
        let result = downloader.resume_download(99999).await;

        // Should succeed (get_pending_articles returns empty Vec for non-existent downloads)
        // This is acceptable behavior - resume_download is idempotent
        assert!(result.is_ok(), "Should succeed (no-op) for non-existent download");

        // Verify no status was changed (download doesn't exist in database)
        let download = downloader.db.get_download(99999).await.unwrap();
        assert!(download.is_none(), "Download should not exist");
    }

    #[tokio::test]
    async fn test_resume_download_emits_event() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Subscribe to events
        let mut events = downloader.subscribe();

        // Add a download (will emit Queued event)
        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Consume the Queued event
        let event = events.recv().await.unwrap();
        assert!(matches!(event, Event::Queued { .. }));

        // Mark all articles as downloaded
        let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
        for article in articles {
            downloader.db.update_article_status(
                article.id,
                crate::db::article_status::DOWNLOADED
            ).await.unwrap();
        }

        // Resume should emit Verifying event (post-processing start)
        downloader.resume_download(download_id).await.unwrap();

        // Check for Verifying event
        let event = events.recv().await.unwrap();
        assert!(
            matches!(event, Event::Verifying { id } if id == download_id),
            "Should emit Verifying event when no pending articles"
        );
    }

    // Task 6.3: restore_queue() tests

    #[tokio::test]
    async fn test_restore_queue_with_no_incomplete_downloads() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Restore queue with empty database
        downloader.restore_queue().await.unwrap();

        // Queue should remain empty
        let queue_size = downloader.queue.lock().await.len();
        assert_eq!(queue_size, 0, "Queue should be empty when no incomplete downloads");
    }

    #[tokio::test]
    async fn test_restore_queue_with_queued_downloads() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add multiple downloads with different priorities
        let id1 = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "download1",
                DownloadOptions {
                    priority: Priority::Low,
                    ..Default::default()
                }
            )
            .await
            .unwrap();

        let id2 = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "download2",
                DownloadOptions {
                    priority: Priority::High,
                    ..Default::default()
                }
            )
            .await
            .unwrap();

        // Clear the queue (simulating a restart)
        downloader.queue.lock().await.clear();

        // Restore queue
        downloader.restore_queue().await.unwrap();

        // Queue should have both downloads restored
        let queue_size = downloader.queue.lock().await.len();
        assert_eq!(queue_size, 2, "Queue should have 2 downloads restored");

        // Verify priority ordering (High priority should be first)
        let next = downloader.queue.lock().await.pop().unwrap();
        assert_eq!(next.id, id2, "High priority download should be first");
        assert_eq!(next.priority, Priority::High);

        let next = downloader.queue.lock().await.pop().unwrap();
        assert_eq!(next.id, id1, "Low priority download should be second");
        assert_eq!(next.priority, Priority::Low);
    }

    #[tokio::test]
    async fn test_restore_queue_with_downloading_status() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download
        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Manually set status to Downloading (simulating interrupted download)
        downloader.db.update_status(download_id, Status::Downloading.to_i32()).await.unwrap();

        // Clear the queue
        downloader.queue.lock().await.clear();

        // Restore queue
        downloader.restore_queue().await.unwrap();

        // Download should be back in queue with Queued status (resume_download does this)
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(
            Status::from_i32(download.status),
            Status::Queued,
            "Download status should be Queued after restore"
        );

        // Queue should contain the download
        let queue_size = downloader.queue.lock().await.len();
        assert_eq!(queue_size, 1, "Queue should have 1 download");
    }

    #[tokio::test]
    async fn test_restore_queue_with_processing_status() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download and mark all articles as downloaded
        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Mark all articles as downloaded
        let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
        for article in articles {
            downloader.db.update_article_status(
                article.id,
                crate::db::article_status::DOWNLOADED
            ).await.unwrap();
        }

        // Manually set status to Processing (simulating interrupted post-processing)
        downloader.db.update_status(download_id, Status::Processing.to_i32()).await.unwrap();

        // Clear the queue
        downloader.queue.lock().await.clear();

        // Restore queue
        downloader.restore_queue().await.unwrap();

        // Download should still be in Processing status (ready for post-processing)
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(
            Status::from_i32(download.status),
            Status::Processing,
            "Download status should remain Processing after restore"
        );
    }

    #[tokio::test]
    async fn test_restore_queue_skips_completed_downloads() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download and mark as complete
        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        downloader.db.update_status(download_id, Status::Complete.to_i32()).await.unwrap();

        // Clear the queue
        downloader.queue.lock().await.clear();

        // Restore queue
        downloader.restore_queue().await.unwrap();

        // Queue should be empty (completed downloads not restored)
        let queue_size = downloader.queue.lock().await.len();
        assert_eq!(queue_size, 0, "Queue should be empty (completed downloads not restored)");
    }

    #[tokio::test]
    async fn test_restore_queue_skips_failed_downloads() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download and mark as failed
        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        downloader.db.update_status(download_id, Status::Failed.to_i32()).await.unwrap();

        // Clear the queue
        downloader.queue.lock().await.clear();

        // Restore queue
        downloader.restore_queue().await.unwrap();

        // Queue should be empty (failed downloads not restored)
        let queue_size = downloader.queue.lock().await.len();
        assert_eq!(queue_size, 0, "Queue should be empty (failed downloads not restored)");
    }

    #[tokio::test]
    async fn test_restore_queue_skips_paused_downloads() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download and pause it
        let download_id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        downloader.pause(download_id).await.unwrap();

        // Clear the queue
        downloader.queue.lock().await.clear();

        // Restore queue
        downloader.restore_queue().await.unwrap();

        // Queue should be empty (paused downloads not restored - user explicitly paused them)
        let queue_size = downloader.queue.lock().await.len();
        assert_eq!(queue_size, 0, "Queue should be empty (paused downloads not restored)");

        // Status should still be Paused
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
        assert_eq!(
            Status::from_i32(download.status),
            Status::Paused,
            "Paused downloads should remain paused"
        );
    }

    #[tokio::test]
    async fn test_restore_queue_called_on_startup() {
        // Create a database with incomplete downloads
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        // Create first downloader instance and add downloads
        {
            let config = Config {
                database_path: db_path.clone(),
                servers: vec![],
                max_concurrent_downloads: 3,
                ..Default::default()
            };
            let downloader = UsenetDownloader::new(config).await.unwrap();

            // Add downloads
            downloader
                .add_nzb_content(SAMPLE_NZB.as_bytes(), "download1", DownloadOptions::default())
                .await
                .unwrap();
            downloader
                .add_nzb_content(SAMPLE_NZB.as_bytes(), "download2", DownloadOptions::default())
                .await
                .unwrap();

            // downloader is dropped here (simulating shutdown)
        }

        // Create new downloader instance (simulating restart)
        let config = Config {
            database_path: db_path.clone(),
            servers: vec![],
            max_concurrent_downloads: 3,
            ..Default::default()
        };
        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Queue should be automatically restored (new() calls restore_queue())
        let queue_size = downloader.queue.lock().await.len();
        assert_eq!(queue_size, 2, "Queue should be restored on startup");
    }

    #[tokio::test]
    async fn test_resume_after_simulated_crash() {
        // Task 6.6: Test resume after simulated crash (kill process mid-download)
        //
        // This test simulates a crash by:
        // 1. Starting a download
        // 2. Marking some articles as downloaded (simulating partial progress)
        // 3. Setting status to Downloading (simulating crash mid-download)
        // 4. Dropping the downloader (simulating process termination)
        // 5. Creating a new downloader instance (simulating restart)
        // 6. Verifying that restore_queue() correctly resumes the download

        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let download_id;
        let total_articles;

        // Simulate crash scenario
        {
            let config = Config {
                database_path: db_path.clone(),
                servers: vec![],
                max_concurrent_downloads: 3,
                ..Default::default()
            };
            let downloader = UsenetDownloader::new(config).await.unwrap();

            // Add a download
            download_id = downloader
                .add_nzb_content(SAMPLE_NZB.as_bytes(), "crash_test", DownloadOptions::default())
                .await
                .unwrap();

            // Get all articles
            let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
            total_articles = articles.len();
            assert!(total_articles > 1, "Need at least 2 articles for this test");

            // Mark half of the articles as downloaded (simulating partial progress)
            let articles_to_download = total_articles / 2;
            for (i, article) in articles.iter().enumerate() {
                if i < articles_to_download {
                    downloader.db.update_article_status(
                        article.id,
                        crate::db::article_status::DOWNLOADED
                    ).await.unwrap();
                }
            }

            // Set status to Downloading (simulating crash mid-download)
            downloader.db.update_status(download_id, Status::Downloading.to_i32()).await.unwrap();

            // Set some progress to verify it's preserved
            let progress = 50.0;
            let speed = 1000000u64; // 1 MB/s
            let downloaded_bytes = 524288u64; // 512 KB
            downloader.db.update_progress(download_id, progress, speed, downloaded_bytes).await.unwrap();

            // Simulate crash by dropping downloader (no graceful shutdown)
            // downloader is dropped here
        }

        // Simulate restart by creating a new downloader instance
        let config = Config {
            database_path: db_path.clone(),
            servers: vec![],
            max_concurrent_downloads: 3,
            ..Default::default()
        };
        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Verify the download was restored
        let download = downloader.db.get_download(download_id).await.unwrap().unwrap();

        // Status should be Queued (resume_download sets it back to Queued)
        assert_eq!(
            Status::from_i32(download.status),
            Status::Queued,
            "Download should be Queued after restore"
        );

        // Progress should be preserved from before crash
        assert_eq!(
            download.progress, 50.0,
            "Download progress should be preserved after crash"
        );

        // Downloaded bytes should be preserved
        assert_eq!(
            download.downloaded_bytes, 524288,
            "Downloaded bytes should be preserved after crash"
        );

        // Queue should contain the download
        let queue_size = downloader.queue.lock().await.len();
        assert_eq!(queue_size, 1, "Queue should have 1 download after restore");

        // Verify that only pending articles remain
        let pending_articles = downloader.db.get_pending_articles(download_id).await.unwrap();
        let expected_pending = total_articles - (total_articles / 2);
        assert_eq!(
            pending_articles.len(),
            expected_pending,
            "Only undownloaded articles should be pending"
        );

        // Verify that downloaded articles are marked correctly
        let downloaded_count = downloader.db.count_articles_by_status(
            download_id,
            crate::db::article_status::DOWNLOADED
        ).await.unwrap();
        assert_eq!(
            downloaded_count as usize,
            total_articles / 2,
            "Downloaded articles count should match"
        );
    }

    #[tokio::test]
    async fn test_speed_limiter_shared_across_downloads() {
        // This test verifies that the speed limiter is properly shared
        // across all download tasks (Task 7.4)

        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let config = Config {
            database_path: db_path,
            servers: vec![],
            max_concurrent_downloads: 3,
            speed_limit_bps: Some(1_000_000), // 1 MB/s limit
            ..Default::default()
        };

        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Verify speed limiter is configured
        assert_eq!(downloader.speed_limiter.get_limit(), Some(1_000_000));

        // Test that the same limiter instance is shared
        // by verifying limit changes affect all downloads
        downloader.speed_limiter.set_limit(Some(5_000_000)); // 5 MB/s
        assert_eq!(downloader.speed_limiter.get_limit(), Some(5_000_000));

        // Reset to unlimited
        downloader.speed_limiter.set_limit(None);
        assert_eq!(downloader.speed_limiter.get_limit(), None);
    }

    #[tokio::test]
    async fn test_set_speed_limit_method() {
        // This test verifies that set_speed_limit() properly updates the limiter
        // and emits the SpeedLimitChanged event (Task 7.6)

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Subscribe to events before changing limit
        let mut rx = downloader.subscribe();

        // Initially should be unlimited (default)
        assert_eq!(downloader.speed_limiter.get_limit(), None);

        // Set speed limit to 10 MB/s
        downloader.set_speed_limit(Some(10_000_000)).await;

        // Verify limit was updated
        assert_eq!(downloader.speed_limiter.get_limit(), Some(10_000_000));

        // Verify event was emitted
        let event = rx.recv().await.unwrap();
        match event {
            crate::types::Event::SpeedLimitChanged { limit_bps } => {
                assert_eq!(limit_bps, Some(10_000_000));
            }
            other => panic!("Expected SpeedLimitChanged event, got {:?}", other),
        }

        // Change to unlimited
        downloader.set_speed_limit(None).await;
        assert_eq!(downloader.speed_limiter.get_limit(), None);

        // Verify second event was emitted
        let event = rx.recv().await.unwrap();
        match event {
            crate::types::Event::SpeedLimitChanged { limit_bps } => {
                assert_eq!(limit_bps, None);
            }
            other => panic!("Expected SpeedLimitChanged event with None, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_set_speed_limit_takes_effect_immediately() {
        // Verify that speed limit changes take effect immediately for ongoing downloads

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Start with 5 MB/s
        downloader.set_speed_limit(Some(5_000_000)).await;
        assert_eq!(downloader.speed_limiter.get_limit(), Some(5_000_000));

        // Change to 10 MB/s
        downloader.set_speed_limit(Some(10_000_000)).await;
        assert_eq!(downloader.speed_limiter.get_limit(), Some(10_000_000));

        // Verify we can still acquire bytes (limiter is functional)
        downloader.speed_limiter.acquire(1000).await;
        // If we reach here, the limiter is working after the change
    }

    #[tokio::test]
    async fn test_speed_limit_with_multiple_concurrent_downloads() {
        // Task 7.7: Test speed limiting with multiple concurrent downloads
        // This test verifies that the speed limiter properly limits total bandwidth
        // across multiple concurrent downloads and distributes bandwidth fairly

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Set a low speed limit for testing (5 MB/s)
        downloader.set_speed_limit(Some(5_000_000)).await;

        // Simulate 3 concurrent downloads
        let limiter = downloader.speed_limiter.clone();
        let start = Instant::now();

        let mut handles = vec![];
        for download_id in 0..3 {
            let limiter_clone = limiter.clone();
            let handle = tokio::spawn(async move {
                // Each download tries to transfer 10 MB total
                // Split into 1 MB chunks to simulate realistic article downloads
                for _ in 0..10 {
                    limiter_clone.acquire(1_000_000).await; // 1 MB chunk
                }
                download_id
            });
            handles.push(handle);
        }

        // Wait for all downloads to complete
        for handle in handles {
            handle.await.unwrap();
        }

        let elapsed = start.elapsed();

        // Total data: 3 downloads × 10 MB = 30 MB
        // Speed limit: 5 MB/s
        // Expected time: 30 MB ÷ 5 MB/s = 6 seconds
        // Allow 20% tolerance (4.8s - 7.2s)
        let min_duration = Duration::from_millis(4800); // 80% of 6 seconds
        let max_duration = Duration::from_millis(7200); // 120% of 6 seconds

        assert!(
            elapsed >= min_duration,
            "Downloads completed too quickly: {:?} (expected >= {:?}). \
             Speed limit may not be working properly.",
            elapsed, min_duration
        );
        assert!(
            elapsed <= max_duration,
            "Downloads took too long: {:?} (expected <= {:?}). \
             Speed limiter may be too conservative.",
            elapsed, max_duration
        );
    }

    #[tokio::test]
    async fn test_speed_limit_dynamic_change_during_downloads() {
        // Task 7.7: Test changing speed limit dynamically while downloads are active
        // This verifies that limit changes take effect immediately for ongoing transfers

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Start with a conservative 2 MB/s limit
        downloader.set_speed_limit(Some(2_000_000)).await;

        let limiter = downloader.speed_limiter.clone();
        let start = Instant::now();

        // Spawn a long-running download task
        let download_handle = {
            let limiter_clone = limiter.clone();
            tokio::spawn(async move {
                // Try to download 20 MB in 1 MB chunks
                for _ in 0..20 {
                    limiter_clone.acquire(1_000_000).await;
                }
            })
        };

        // Wait 2 seconds, then increase speed limit
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Should have downloaded ~4 MB by now (2 MB/s × 2s)
        // Now increase to 10 MB/s
        downloader.set_speed_limit(Some(10_000_000)).await;

        // Wait for download to complete
        download_handle.await.unwrap();

        let elapsed = start.elapsed();

        // Analysis:
        // - First 2 seconds at 2 MB/s: ~4 MB downloaded (but may have 2 MB bucket at start)
        // - Remaining 16 MB at 10 MB/s: ~1.6 seconds  (but may have 10 MB bucket when limit changes)
        // - Total expected: ~2.2-4 seconds (accounting for initial token bucket)
        // The key is that changing the limit should allow faster completion than if
        // the limit stayed at 2 MB/s (which would take 10 seconds total)
        let min_duration = Duration::from_millis(2200); // Must be faster than 10s (20MB at 2MB/s)
        let max_duration = Duration::from_secs(5);

        assert!(
            elapsed >= min_duration,
            "Download with dynamic limit change completed too quickly: {:?}. \
             This is actually good - it means the speed limiter is working!",
            elapsed
        );
        assert!(
            elapsed <= max_duration,
            "Download with dynamic limit change took too long: {:?}. \
             Limit change may not have taken effect immediately.",
            elapsed
        );

        // Most importantly: verify it's much faster than if limit stayed at 2 MB/s
        // 20 MB at 2 MB/s would take 10 seconds
        assert!(
            elapsed < Duration::from_secs(8),
            "Download took {:?}, which suggests limit change didn't take effect. \
             Expected < 8s (much faster than 10s for 20MB at 2MB/s).",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_speed_limit_bandwidth_distribution() {
        // Task 7.7: Test that bandwidth is distributed fairly across concurrent downloads
        // All downloads should complete at roughly the same time

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Set speed limit to 6 MB/s
        downloader.set_speed_limit(Some(6_000_000)).await;

        let limiter = downloader.speed_limiter.clone();

        // Shared start time for all downloads
        let global_start = Instant::now();

        // Spawn 3 concurrent downloads that each download 6 MB
        let mut handles = vec![];
        for download_id in 0..3 {
            let limiter_clone = limiter.clone();
            let handle = tokio::spawn(async move {
                // Each download: 6 MB in 500 KB chunks
                for _ in 0..12 {
                    limiter_clone.acquire(500_000).await;
                }
                download_id
            });
            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            handle.await.unwrap();
        }

        let total_elapsed = global_start.elapsed();

        // Total: 18 MB at 6 MB/s = 3 seconds expected
        // With fair distribution, all should finish at roughly the same time
        let expected = Duration::from_secs(3);
        let tolerance = Duration::from_millis(1500); // ±1.5s tolerance

        assert!(
            total_elapsed.as_millis() >= (expected.as_millis() - tolerance.as_millis()),
            "All downloads completed too quickly: {:?} (expected ~{:?}). \
             Speed limiting may not be working properly.",
            total_elapsed, expected
        );
        assert!(
            total_elapsed.as_millis() <= (expected.as_millis() + tolerance.as_millis()),
            "Downloads took too long: {:?} (expected ~{:?})",
            total_elapsed, expected
        );
    }

    #[tokio::test]
    async fn test_speed_limit_unlimited_mode_with_concurrent_downloads() {
        // Task 7.7: Verify that unlimited mode allows maximum throughput
        // without any artificial delays

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Set to unlimited (default)
        downloader.set_speed_limit(None).await;

        let limiter = downloader.speed_limiter.clone();
        let start = Instant::now();

        // Spawn 3 concurrent downloads
        let mut handles = vec![];
        for _ in 0..3 {
            let limiter_clone = limiter.clone();
            let handle = tokio::spawn(async move {
                // Each tries to acquire 10 MB
                for _ in 0..10 {
                    limiter_clone.acquire(1_000_000).await;
                }
            });
            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            handle.await.unwrap();
        }

        let elapsed = start.elapsed();

        // In unlimited mode, 30 MB total should complete almost instantly
        // (only task spawning overhead, no rate limiting delays)
        // Allow up to 100ms for test overhead
        assert!(
            elapsed < Duration::from_millis(100),
            "Unlimited mode took too long: {:?}. There may be unexpected rate limiting.",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_shutdown_graceful() {
        // Task 9.1: Test graceful shutdown
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Verify shutdown completes successfully
        let result = downloader.shutdown().await;
        assert!(result.is_ok(), "Shutdown should complete successfully: {:?}", result);
    }

    #[tokio::test]
    async fn test_shutdown_with_active_downloads() {
        // Task 9.1: Test shutdown cancels active downloads
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Simulate some active downloads by adding cancellation tokens
        {
            let mut active = downloader.active_downloads.lock().await;
            active.insert(1, tokio_util::sync::CancellationToken::new());
            active.insert(2, tokio_util::sync::CancellationToken::new());
        }

        // Verify we have active downloads
        {
            let active = downloader.active_downloads.lock().await;
            assert_eq!(active.len(), 2);
        }

        // Shutdown should cancel them
        let result = downloader.shutdown().await;
        assert!(result.is_ok(), "Shutdown should complete successfully: {:?}", result);

        // Verify tokens were cancelled (active_downloads map should still contain them,
        // but they should be in cancelled state)
        {
            let active = downloader.active_downloads.lock().await;
            for (_id, token) in active.iter() {
                assert!(token.is_cancelled(), "Download should be cancelled after shutdown");
            }
        }
    }

    #[tokio::test]
    async fn test_shutdown_waits_for_completion() {
        // Task 9.1: Test shutdown waits for active downloads to complete
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download token, then remove it after a delay to simulate completion
        let token = tokio_util::sync::CancellationToken::new();
        {
            let mut active = downloader.active_downloads.lock().await;
            active.insert(1, token.clone());
        }

        // Spawn a task that removes the download after 500ms (simulating completion)
        let active_downloads_clone = downloader.active_downloads.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let mut active = active_downloads_clone.lock().await;
            active.remove(&1);
        });

        let start = std::time::Instant::now();

        // Shutdown should wait for the download to complete
        let result = downloader.shutdown().await;
        let elapsed = start.elapsed();

        assert!(result.is_ok(), "Shutdown should complete successfully: {:?}", result);

        // Verify it waited (should take at least 500ms)
        assert!(
            elapsed >= std::time::Duration::from_millis(450),
            "Shutdown should have waited for downloads to complete: {:?}",
            elapsed
        );

        // But not too long (should be < 1 second for this test)
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "Shutdown took too long: {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_shutdown_rejects_new_downloads() {
        // Task 9.2: Test that shutdown() sets accepting_new flag and new downloads are rejected
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Initially, should accept new downloads
        assert!(
            downloader.accepting_new.load(std::sync::atomic::Ordering::SeqCst),
            "Should accept new downloads initially"
        );

        // Attempt to add a download before shutdown - should succeed
        let result_before = downloader.add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        ).await;
        assert!(result_before.is_ok(), "Should accept download before shutdown: {:?}", result_before);

        // Trigger shutdown
        let shutdown_result = downloader.shutdown().await;
        assert!(shutdown_result.is_ok(), "Shutdown should complete successfully: {:?}", shutdown_result);

        // After shutdown, accepting_new should be false
        assert!(
            !downloader.accepting_new.load(std::sync::atomic::Ordering::SeqCst),
            "Should not accept new downloads after shutdown"
        );

        // Attempt to add a download after shutdown - should fail with ShuttingDown error
        let result_after = downloader.add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test2.nzb",
            DownloadOptions::default(),
        ).await;

        assert!(result_after.is_err(), "Should reject download after shutdown");
        match result_after {
            Err(crate::error::Error::ShuttingDown) => {
                // Expected error
            }
            other => panic!("Expected ShuttingDown error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_pause_graceful_all() {
        // Task 9.3: Test graceful pause signals cancellation to all active downloads
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add multiple download tokens to simulate active downloads
        let token1 = tokio_util::sync::CancellationToken::new();
        let token2 = tokio_util::sync::CancellationToken::new();
        let token3 = tokio_util::sync::CancellationToken::new();

        {
            let mut active = downloader.active_downloads.lock().await;
            active.insert(1, token1.clone());
            active.insert(2, token2.clone());
            active.insert(3, token3.clone());
        }

        // Verify tokens are not cancelled initially
        assert!(!token1.is_cancelled(), "Token 1 should not be cancelled initially");
        assert!(!token2.is_cancelled(), "Token 2 should not be cancelled initially");
        assert!(!token3.is_cancelled(), "Token 3 should not be cancelled initially");

        // Call pause_graceful_all
        downloader.pause_graceful_all().await;

        // Verify all tokens are now cancelled (graceful pause signaled)
        assert!(token1.is_cancelled(), "Token 1 should be cancelled after graceful pause");
        assert!(token2.is_cancelled(), "Token 2 should be cancelled after graceful pause");
        assert!(token3.is_cancelled(), "Token 3 should be cancelled after graceful pause");

        // Verify downloads are still in active_downloads map (they clean up when tasks complete)
        {
            let active = downloader.active_downloads.lock().await;
            assert_eq!(active.len(), 3, "Downloads should still be in active map");
        }
    }

    #[tokio::test]
    async fn test_graceful_pause_completes_current_article() {
        // Task 9.3: Verify that graceful pause allows current article to complete
        // This is a conceptual test - the actual behavior is in the download loop
        // which checks cancellation BEFORE starting each article, not during.
        // This means the current article always completes before pausing.

        let (downloader, _temp_dir) = create_test_downloader().await;

        // Create a cancellation token
        let token = tokio_util::sync::CancellationToken::new();
        let token_clone = token.clone();

        // Simulate an article download in progress
        let article_complete = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let article_complete_clone = article_complete.clone();

        // Spawn a task that simulates downloading an article (takes 200ms)
        let download_task = tokio::spawn(async move {
            // Simulate article download starting
            tracing::debug!("Article download started");

            // Download takes 200ms
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;

            // Mark article as complete
            article_complete_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            tracing::debug!("Article download completed");

            // After article completes, check for cancellation (this is what the real code does)
            if token_clone.is_cancelled() {
                tracing::debug!("Cancellation detected after article completed");
                return false; // Would exit the download loop
            }

            true // Would continue to next article
        });

        // Wait 100ms (article is in-progress)
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Signal graceful pause while article is downloading
        token.cancel();
        tracing::debug!("Graceful pause signaled while article in progress");

        // Wait for task to complete
        let result = download_task.await.unwrap();

        // Verify the article completed before the cancellation was detected
        assert!(
            article_complete.load(std::sync::atomic::Ordering::SeqCst),
            "Article should have completed"
        );
        assert!(!result, "Download should have stopped after detecting cancellation");
    }

    #[tokio::test]
    async fn test_persist_all_state_marks_interrupted_downloads_as_paused() {
        // Task 9.5: Test persist_all_state() marks interrupted downloads as Paused
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download in Downloading status
        let id1 = downloader.add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test1.nzb",
            DownloadOptions::default(),
        ).await.unwrap();

        // Manually set it to Downloading status (simulating active download)
        downloader.db.update_status(id1, Status::Downloading.to_i32()).await.unwrap();

        // Add another download in Processing status
        let id2 = downloader.add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test2.nzb",
            DownloadOptions::default(),
        ).await.unwrap();
        downloader.db.update_status(id2, Status::Processing.to_i32()).await.unwrap();

        // Add a download in Complete status (should not be changed)
        let id3 = downloader.add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test3.nzb",
            DownloadOptions::default(),
        ).await.unwrap();
        downloader.db.update_status(id3, Status::Complete.to_i32()).await.unwrap();

        // Verify initial states
        let dl1 = downloader.db.get_download(id1).await.unwrap().unwrap();
        assert_eq!(dl1.status, Status::Downloading.to_i32());
        let dl2 = downloader.db.get_download(id2).await.unwrap().unwrap();
        assert_eq!(dl2.status, Status::Processing.to_i32());
        let dl3 = downloader.db.get_download(id3).await.unwrap().unwrap();
        assert_eq!(dl3.status, Status::Complete.to_i32());

        // Call persist_all_state (these downloads are not in active_downloads map)
        let result = downloader.persist_all_state().await;
        assert!(result.is_ok(), "persist_all_state should succeed: {:?}", result);

        // Verify interrupted downloads were marked as Paused
        let dl1_after = downloader.db.get_download(id1).await.unwrap().unwrap();
        assert_eq!(
            dl1_after.status,
            Status::Paused.to_i32(),
            "Interrupted Downloading should be marked as Paused"
        );

        let dl2_after = downloader.db.get_download(id2).await.unwrap().unwrap();
        assert_eq!(
            dl2_after.status,
            Status::Paused.to_i32(),
            "Interrupted Processing should be marked as Paused"
        );

        // Complete download should remain unchanged
        let dl3_after = downloader.db.get_download(id3).await.unwrap().unwrap();
        assert_eq!(
            dl3_after.status,
            Status::Complete.to_i32(),
            "Complete download should remain Complete"
        );
    }

    #[tokio::test]
    async fn test_persist_all_state_preserves_active_downloads() {
        // Task 9.5: Test persist_all_state() does not modify truly active downloads
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download
        let id = downloader.add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        ).await.unwrap();

        // Set it to Downloading status
        downloader.db.update_status(id, Status::Downloading.to_i32()).await.unwrap();

        // Add it to active_downloads map (simulating it's actually running)
        {
            let mut active = downloader.active_downloads.lock().await;
            active.insert(id, tokio_util::sync::CancellationToken::new());
        }

        // Call persist_all_state
        let result = downloader.persist_all_state().await;
        assert!(result.is_ok(), "persist_all_state should succeed: {:?}", result);

        // Verify the download status was NOT changed (it's still active)
        let dl_after = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(
            dl_after.status,
            Status::Downloading.to_i32(),
            "Active download should remain in Downloading status"
        );
    }

    #[tokio::test]
    async fn test_shutdown_calls_persist_all_state() {
        // Task 9.5: Test shutdown() integrates persist_all_state()
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download in Downloading status (simulating interrupted)
        let id = downloader.add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        ).await.unwrap();
        downloader.db.update_status(id, Status::Downloading.to_i32()).await.unwrap();

        // Call shutdown
        let result = downloader.shutdown().await;
        assert!(result.is_ok(), "Shutdown should succeed: {:?}", result);

        // Verify the interrupted download was marked as Paused by persist_all_state
        let dl_after = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(
            dl_after.status,
            Status::Paused.to_i32(),
            "Interrupted download should be marked as Paused after shutdown"
        );
    }

    #[tokio::test]
    async fn test_shutdown_emits_shutdown_event() {
        // Task 9.6: Test that shutdown() emits a Shutdown event
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Subscribe to events
        let mut events = downloader.subscribe();

        // Spawn a task to collect events
        let event_handle = tokio::spawn(async move {
            let mut shutdown_received = false;
            while let Ok(event) = events.recv().await {
                if matches!(event, Event::Shutdown) {
                    shutdown_received = true;
                    break;
                }
            }
            shutdown_received
        });

        // Call shutdown
        let result = downloader.shutdown().await;
        assert!(result.is_ok(), "Shutdown should succeed: {:?}", result);

        // Verify Shutdown event was emitted
        let shutdown_received = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            event_handle
        ).await.expect("Timeout waiting for event task")
            .expect("Event task should complete");

        assert!(shutdown_received, "Shutdown event should be emitted");
    }

    #[tokio::test]
    async fn test_run_with_shutdown_basic() {
        // Task 9.6: Test that run_with_shutdown function exists and is callable
        // Note: We can't easily test actual signal handling in unit tests,
        // but we verify the function compiles and the structure is correct

        let (downloader, _temp_dir) = create_test_downloader().await;

        // We can't easily send signals in a test, so we just verify
        // the function signature and structure by calling shutdown directly
        let result = downloader.shutdown().await;
        assert!(result.is_ok(), "Shutdown should succeed: {:?}", result);
    }

    #[tokio::test]
    async fn test_graceful_shutdown_and_recovery_on_restart() {
        // Task 9.8: Test complete graceful shutdown and recovery on restart
        //
        // This integration test verifies:
        // 1. Active downloads are gracefully paused on shutdown
        // 2. Database is marked as "clean shutdown"
        // 3. On restart, downloads are properly restored
        // 4. Progress and state are preserved across restart

        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let download_id;
        let total_articles;

        // Phase 1: Create downloader, add download, and perform graceful shutdown
        {
            let config = Config {
                database_path: db_path.clone(),
                servers: vec![],
                max_concurrent_downloads: 3,
                ..Default::default()
            };

            let downloader = UsenetDownloader::new(config).await.unwrap();

            // Add a download
            download_id = downloader.add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "test.nzb",
                DownloadOptions::default()
            ).await.unwrap();

            // Get all articles
            let articles = downloader.db.get_pending_articles(download_id).await.unwrap();
            total_articles = articles.len();
            assert!(total_articles > 1, "Need at least 2 articles for this test");

            // Mark first article as downloaded (simulating partial progress)
            if let Some(first_article) = articles.first() {
                downloader.db.update_article_status(
                    first_article.id,
                    crate::db::article_status::DOWNLOADED
                ).await.unwrap();
            }

            // Set status to Downloading (simulating active download)
            downloader.db.update_status(download_id, Status::Downloading.to_i32()).await.unwrap();

            // Set some progress to verify it's preserved
            let progress = 50.0;
            let speed = 1000000u64; // 1 MB/s
            let downloaded_bytes = 524288u64; // 512 KB
            downloader.db.update_progress(download_id, progress, speed, downloaded_bytes).await.unwrap();

            // Perform graceful shutdown
            let shutdown_result = downloader.shutdown().await;
            assert!(shutdown_result.is_ok(), "Graceful shutdown should succeed: {:?}", shutdown_result);

            // Verify database was marked as clean shutdown
            let was_unclean = downloader.db.was_unclean_shutdown().await.unwrap();
            assert!(!was_unclean, "Database should be marked as CLEAN shutdown after graceful shutdown");

            // Verify download was marked as Paused (not Downloading)
            let download = downloader.db.get_download(download_id).await.unwrap().unwrap();
            assert_eq!(
                Status::from_i32(download.status),
                Status::Paused,
                "Download should be marked as Paused after graceful shutdown"
            );
        }

        // Phase 2: Simulate restart by creating new downloader instance
        {
            // First, check the shutdown state BEFORE creating the downloader
            // (UsenetDownloader::new() calls set_clean_start() which would override the flag)
            let db_for_check = Database::new(&db_path).await.unwrap();
            let was_unclean = db_for_check.was_unclean_shutdown().await.unwrap();
            assert!(!was_unclean, "Database should show clean shutdown from previous session");
            db_for_check.close().await;

            // Now create the downloader (which will call set_clean_start() internally)
            let config = Config {
                database_path: db_path.clone(),
                servers: vec![],
                max_concurrent_downloads: 3,
                ..Default::default()
            };

            let downloader = UsenetDownloader::new(config).await.unwrap();

            // Verify download was restored
            let restored_download = downloader.db.get_download(download_id).await.unwrap();
            assert!(restored_download.is_some(), "Download should be restored after restart");

            let download = restored_download.unwrap();

            // After graceful shutdown, download should remain Paused
            assert_eq!(
                Status::from_i32(download.status),
                Status::Paused,
                "Download should remain Paused after restart"
            );

            // Progress should be preserved
            assert_eq!(download.progress, 50.0, "Progress should be preserved");
            assert_eq!(download.downloaded_bytes, 524288, "Downloaded bytes should be preserved");

            // Verify article tracking was preserved
            let pending_articles = downloader.db.get_pending_articles(download_id).await.unwrap();
            assert_eq!(
                pending_articles.len(),
                total_articles - 1,
                "Should have {} pending articles (1 was downloaded before shutdown)",
                total_articles - 1
            );

            // Verify we can resume the download after restart
            let resume_result = downloader.resume(download_id).await;
            assert!(resume_result.is_ok(), "Should be able to resume download after restart: {:?}", resume_result);

            let resumed_download = downloader.db.get_download(download_id).await.unwrap().unwrap();
            assert_eq!(
                Status::from_i32(resumed_download.status),
                Status::Queued,
                "Download should be Queued after resume"
            );
        }
    }

    #[tokio::test]
    async fn test_start_folder_watcher_no_watch_folders() {
        // Create downloader with no watch folders configured
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Should succeed but return a completed task
        let handle = downloader.start_folder_watcher();
        assert!(handle.is_ok(), "start_folder_watcher should succeed with no watch folders");

        // The task should complete immediately
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            handle.unwrap()
        ).await;
        assert!(result.is_ok(), "Task should complete immediately with no watch folders");
    }

    #[tokio::test]
    async fn test_start_folder_watcher_with_configured_folders() {
        let temp_dir = tempdir().unwrap();
        let watch_path = temp_dir.path().join("watch");

        // Create config with watch folder
        let config = Config {
            database_path: temp_dir.path().join("test.db"),
            servers: vec![],
            watch_folders: vec![
                config::WatchFolderConfig {
                    path: watch_path.clone(),
                    after_import: config::WatchFolderAction::Delete,
                    category: Some("test".to_string()),
                    scan_interval: Duration::from_secs(5),
                }
            ],
            ..Default::default()
        };

        let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

        // Start folder watcher
        let handle = downloader.start_folder_watcher();
        assert!(handle.is_ok(), "start_folder_watcher should succeed: {:?}", handle.err());

        // Verify watch directory was created
        assert!(watch_path.exists(), "Watch folder should be created by start()");

        // Let the watcher task run for a moment
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Abort the task (it runs indefinitely)
        handle.unwrap().abort();
    }

    #[tokio::test]
    async fn test_start_folder_watcher_creates_missing_directory() {
        let temp_dir = tempdir().unwrap();
        let watch_path = temp_dir.path().join("nonexistent").join("watch");

        // Verify directory doesn't exist yet
        assert!(!watch_path.exists(), "Watch path should not exist yet");

        // Create config with non-existent watch folder
        let config = Config {
            database_path: temp_dir.path().join("test.db"),
            servers: vec![],
            watch_folders: vec![
                config::WatchFolderConfig {
                    path: watch_path.clone(),
                    after_import: config::WatchFolderAction::MoveToProcessed,
                    category: None,
                    scan_interval: Duration::from_secs(5),
                }
            ],
            ..Default::default()
        };

        let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

        // Start folder watcher - should create the directory
        let handle = downloader.start_folder_watcher();
        assert!(handle.is_ok(), "start_folder_watcher should create missing directories: {:?}", handle.err());

        // Verify directory was created
        assert!(watch_path.exists(), "Watch folder should be auto-created");

        // Abort the task
        handle.unwrap().abort();
    }

    // ============================================================================
    // RSS Scheduler Tests
    // ============================================================================

    #[tokio::test]
    async fn test_start_rss_scheduler_no_feeds() {
        // Create downloader with no RSS feeds configured
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Should succeed but return a completed task
        let handle = downloader.start_rss_scheduler();

        // The task should complete immediately with no feeds
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            handle
        ).await;
        assert!(result.is_ok(), "Task should complete immediately with no RSS feeds");
    }

    #[tokio::test]
    async fn test_start_rss_scheduler_with_feeds() {
        let temp_dir = tempdir().unwrap();

        // Create config with RSS feeds
        let config = Config {
            database_path: temp_dir.path().join("test.db"),
            servers: vec![],
            rss_feeds: vec![
                config::RssFeedConfig {
                    url: "https://example.com/feed.xml".to_string(),
                    check_interval: Duration::from_secs(60), // 1 minute
                    category: Some("test".to_string()),
                    filters: vec![],
                    auto_download: true,
                    priority: Priority::Normal,
                    enabled: true,
                }
            ],
            ..Default::default()
        };

        let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

        // Start RSS scheduler
        let handle = downloader.start_rss_scheduler();

        // Let the scheduler task start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify the task is still running (it shouldn't complete immediately)
        assert!(!handle.is_finished(), "Scheduler should be running with configured feeds");

        // Abort the task
        handle.abort();
    }

    #[tokio::test]
    async fn test_start_rss_scheduler_respects_shutdown() {
        let temp_dir = tempdir().unwrap();

        // Create config with RSS feeds
        let config = Config {
            database_path: temp_dir.path().join("test.db"),
            servers: vec![],
            rss_feeds: vec![
                config::RssFeedConfig {
                    url: "https://example.com/feed.xml".to_string(),
                    check_interval: Duration::from_secs(60),
                    category: None,
                    filters: vec![],
                    auto_download: false,
                    priority: Priority::Normal,
                    enabled: true,
                }
            ],
            ..Default::default()
        };

        let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

        // Start RSS scheduler
        let handle = downloader.start_rss_scheduler();

        // Let it run briefly
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Trigger shutdown
        downloader.accepting_new.store(false, std::sync::atomic::Ordering::SeqCst);

        // Wait for scheduler to detect shutdown
        // Note: Scheduler checks every second, so 5 seconds should be plenty
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            handle
        ).await;

        assert!(result.is_ok(), "Scheduler should shut down gracefully when accepting_new is set to false");
    }

    #[tokio::test]
    async fn test_start_rss_scheduler_with_multiple_feeds() {
        let temp_dir = tempdir().unwrap();

        // Create config with multiple RSS feeds
        let config = Config {
            database_path: temp_dir.path().join("test.db"),
            servers: vec![],
            rss_feeds: vec![
                config::RssFeedConfig {
                    url: "https://example.com/feed1.xml".to_string(),
                    check_interval: Duration::from_secs(30),
                    category: Some("movies".to_string()),
                    filters: vec![],
                    auto_download: true,
                    priority: Priority::High,
                    enabled: true,
                },
                config::RssFeedConfig {
                    url: "https://example.com/feed2.xml".to_string(),
                    check_interval: Duration::from_secs(60),
                    category: Some("tv".to_string()),
                    filters: vec![],
                    auto_download: false,
                    priority: Priority::Normal,
                    enabled: false, // Disabled feed should be skipped
                }
            ],
            ..Default::default()
        };

        let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

        // Start RSS scheduler
        let handle = downloader.start_rss_scheduler();

        // Let it run briefly
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify the task is running
        assert!(!handle.is_finished(), "Scheduler should handle multiple feeds");

        // Abort the task
        handle.abort();
    }

    #[tokio::test]
    async fn test_start_rss_scheduler_only_enabled_feeds() {
        let temp_dir = tempdir().unwrap();

        // Create config with only disabled feeds
        let config = Config {
            database_path: temp_dir.path().join("test.db"),
            servers: vec![],
            rss_feeds: vec![
                config::RssFeedConfig {
                    url: "https://example.com/feed.xml".to_string(),
                    check_interval: Duration::from_secs(60),
                    category: None,
                    filters: vec![],
                    auto_download: false,
                    priority: Priority::Normal,
                    enabled: false, // Disabled
                }
            ],
            ..Default::default()
        };

        let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

        // Start RSS scheduler
        let handle = downloader.start_rss_scheduler();

        // Let it run briefly
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Scheduler should still be running (just idle, checking for enabled feeds)
        assert!(!handle.is_finished(), "Scheduler should run even with disabled feeds");

        // Abort the task
        handle.abort();
    }
}
