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
    /// Database instance for persistence (wrapped in Arc for sharing across tasks)
    db: std::sync::Arc<Database>,
    /// Event broadcast channel sender (multiple subscribers supported)
    event_tx: tokio::sync::broadcast::Sender<crate::types::Event>,
    /// Configuration (wrapped in Arc for sharing across tasks)
    config: std::sync::Arc<Config>,
    /// NNTP connection pools (one per server, wrapped in Arc for sharing across tasks)
    nntp_pools: std::sync::Arc<Vec<nntp_rs::NntpPool>>,
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

        Ok(Self {
            db: std::sync::Arc::new(db),
            event_tx,
            config: std::sync::Arc::new(config),
            nntp_pools: std::sync::Arc::new(nntp_pools),
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
            ..Default::default()
        };

        // Initialize database
        let db = Database::new(&config.database_path).await.unwrap();

        // Create broadcast channel
        let (event_tx, _rx) = tokio::sync::broadcast::channel(1000);

        // No NNTP pools since we have no servers
        let nntp_pools = Vec::new();

        let downloader = UsenetDownloader {
            db: std::sync::Arc::new(db),
            event_tx,
            config: std::sync::Arc::new(config),
            nntp_pools: std::sync::Arc::new(nntp_pools),
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
}
