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
/// Main downloader instance (cloneable - all fields are Arc-wrapped)
#[derive(Clone)]
pub struct UsenetDownloader {
    /// Database instance for persistence (wrapped in Arc for sharing across tasks)
    db: std::sync::Arc<Database>,
    /// Event broadcast channel sender (multiple subscribers supported)
    event_tx: tokio::sync::broadcast::Sender<crate::types::Event>,
    /// Configuration (wrapped in Arc for sharing across tasks)
    config: std::sync::Arc<Config>,
    /// NNTP connection pools (one per server, wrapped in Arc for sharing across tasks)
    nntp_pools: std::sync::Arc<Vec<nntp_rs::NntpPool>>,
    /// Priority queue for managing download order (protected by Mutex)
    queue: std::sync::Arc<tokio::sync::Mutex<std::collections::BinaryHeap<QueuedDownload>>>,
    /// Semaphore to limit concurrent downloads (respects max_concurrent_downloads config)
    concurrent_limit: std::sync::Arc<tokio::sync::Semaphore>,
    /// Map of active downloads to their cancellation tokens (for pause/cancel operations)
    active_downloads: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<DownloadId, tokio_util::sync::CancellationToken>>>,
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

        let downloader = Self {
            db: std::sync::Arc::new(db),
            event_tx,
            config: std::sync::Arc::new(config),
            nntp_pools: std::sync::Arc::new(nntp_pools),
            queue,
            concurrent_limit,
            active_downloads,
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
            if let Some(cat_config) = self.config.categories.get(category) {
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
            if let Some(cat_config) = self.config.categories.get(category) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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

        let downloader = UsenetDownloader {
            db: std::sync::Arc::new(db),
            event_tx,
            config: std::sync::Arc::new(config),
            nntp_pools: std::sync::Arc::new(nntp_pools),
            queue,
            concurrent_limit,
            active_downloads,
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
}
