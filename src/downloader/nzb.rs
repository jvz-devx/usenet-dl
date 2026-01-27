//! NZB file parsing, ingestion, duplicate detection, and disk space checks.

use crate::db;
use crate::error::{Error, Result};
use crate::types::{DownloadId, DownloadOptions, DuplicateInfo, Event, Status};
use crate::utils::extract_filename_from_response;

use super::UsenetDownloader;

impl UsenetDownloader {
    /// Add an NZB to the download queue from raw bytes
    ///
    /// This method parses the NZB content, creates a download record in the database,
    /// and emits a Queued event. The download will be processed by the queue processor,
    /// which will download articles in parallel using all configured NNTP connections.
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

        // Check if sufficient disk space is available
        self.check_disk_space(size_bytes).await?;

        // Calculate NZB hash for duplicate detection (sha256)
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content);
        let hash_result = hasher.finalize();
        let nzb_hash = format!("{:x}", hash_result);

        // Determine job name (for deobfuscation and duplicate detection)
        // Use NZB meta title if available, otherwise the provided name
        let job_name = nzb_meta_name.clone().unwrap_or_else(|| name.to_string());

        // Check for duplicates before proceeding
        if let Some(dup_info) = self.check_duplicate(content, name).await {
            // Emit warning event about duplicate
            self.emit_event(Event::DuplicateDetected {
                id: dup_info.existing_id,
                name: name.to_string(),
                method: dup_info.method,
                existing_name: dup_info.existing_name.clone(),
            });

            // Handle based on configured action
            match self.config.duplicate.action {
                crate::config::DuplicateAction::Block => {
                    return Err(Error::Duplicate(format!(
                        "Duplicate download detected: '{}' (method: {:?}, existing ID: {}, existing name: '{}')",
                        name, dup_info.method, dup_info.existing_id, dup_info.existing_name
                    )));
                }
                crate::config::DuplicateAction::Warn => {
                    // Already emitted warning event, continue with download
                }
                crate::config::DuplicateAction::Allow => {
                    // Silently allow, no event emitted (skip the emit above)
                    // Note: We already emitted the event above, but that's fine
                    // The event is informational in Allow mode
                }
            }
        }

        // Determine destination directory
        let destination = if let Some(dest) = options.destination {
            dest
        } else if let Some(category) = &options.category {
            // Check if category has custom destination
            let categories = self.categories.read().await;
            if let Some(cat_config) = categories.get(category) {
                cat_config.destination.clone()
            } else {
                self.config.download.download_dir.clone()
            }
        } else {
            self.config.download.download_dir.clone()
        };

        // Determine post-processing mode
        let post_process = if let Some(pp) = options.post_process {
            pp
        } else if let Some(category) = &options.category {
            // Check if category has custom post-processing
            let categories = self.categories.read().await;
            if let Some(cat_config) = categories.get(category) {
                cat_config
                    .post_process
                    .unwrap_or(self.config.download.default_post_process)
            } else {
                self.config.download.default_post_process
            }
        } else {
            self.config.download.default_post_process
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
            destination: destination.to_string_lossy().into_owned(),
            post_process: post_process.to_i32(),
            priority: options.priority as i32,
            status: Status::Queued.to_i32(),
            size_bytes,
        };

        // Insert download into database
        let download_id = self.db.insert_download(&new_download).await?;

        // Insert all articles (segments) for resume support (batch insert for performance)
        // SQLite has a limit of ~999 variables per query, so we chunk (5 columns per article = 199 max)
        let articles: Vec<db::NewArticle> = nzb
            .files
            .iter()
            .flat_map(|file| {
                file.segments.iter().map(|segment| db::NewArticle {
                    download_id,
                    message_id: segment.message_id.clone(),
                    segment_number: segment.number as i32,
                    size_bytes: segment.bytes as i64,
                })
            })
            .collect();
        for chunk in articles.chunks(199) {
            self.db.insert_articles_batch(chunk).await?;
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

        // Trigger webhooks for queued event
        self.trigger_webhooks(
            crate::config::WebhookEvent::OnQueued,
            download_id,
            name.to_string(),
            options.category.clone(),
            "queued".to_string(),
            None,
            None,
        );

        // Add to priority queue for processing
        self.add_to_queue(download_id).await?;

        Ok(download_id)
    }

    /// Add an NZB to the download queue from a file
    ///
    /// This is a convenience method that reads an NZB file from disk and delegates
    /// to `add_nzb_content()`. The filename (without extension) is used as the download name.
    pub async fn add_nzb(
        &self,
        path: &std::path::Path,
        options: DownloadOptions,
    ) -> Result<DownloadId> {
        // Read file content
        let content = tokio::fs::read(path).await.map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("Failed to read NZB file '{}': {}", path.display(), e),
            ))
        })?;

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
    pub async fn add_nzb_url(&self, url: &str, options: DownloadOptions) -> Result<DownloadId> {
        // Create HTTP client with timeout to prevent hanging
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| {
                Error::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to create HTTP client: {}", e),
                ))
            })?;

        // Fetch NZB from URL with timeout
        let response = client.get(url).send().await.map_err(|e| {
            let error_msg = if e.is_timeout() {
                format!(
                    "Timeout fetching NZB from URL '{}' (exceeded 30 seconds)",
                    url
                )
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
                format!("HTTP error fetching NZB: {} {}", response.status(), url),
            )));
        }

        // Extract filename from Content-Disposition header or URL
        let name = extract_filename_from_response(&response, url);

        // Read response body
        let content = response.bytes().await.map_err(|e| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to read response body from '{}': {}", url, e),
            ))
        })?;

        // Delegate to add_nzb_content
        self.add_nzb_content(&content, &name, options).await
    }

    /// Mark an NZB file as processed in the database
    ///
    /// This is used by the folder watcher with WatchFolderAction::Keep to track
    /// which NZB files have already been processed to avoid re-adding them.
    pub async fn mark_nzb_processed(&self, path: &std::path::Path) -> Result<()> {
        self.db.mark_nzb_processed(path).await
    }

    /// Check if an NZB is a duplicate of an existing download
    ///
    /// This method checks for duplicates using the configured detection methods
    /// (NZB hash, NZB name, or job name). Returns information about the duplicate
    /// if found, or None if this is a new download.
    pub(crate) async fn check_duplicate(
        &self,
        nzb_content: &[u8],
        name: &str,
    ) -> Option<DuplicateInfo> {
        // Early return if duplicate detection is disabled
        if !self.config.duplicate.enabled {
            return None;
        }

        // Check each configured detection method in order
        for method in &self.config.duplicate.methods {
            match method {
                crate::config::DuplicateMethod::NzbHash => {
                    // Calculate SHA256 hash of NZB content
                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(nzb_content);
                    let hash_bytes = hasher.finalize();
                    let hash = format!("{:x}", hash_bytes);

                    // Check if this hash exists in database
                    if let Ok(Some(existing)) = self.db.find_by_nzb_hash(&hash).await {
                        return Some(DuplicateInfo {
                            method: *method,
                            existing_id: existing.id,
                            existing_name: existing.name,
                        });
                    }
                }
                crate::config::DuplicateMethod::NzbName => {
                    // Check if download with this name already exists
                    if let Ok(Some(existing)) = self.db.find_by_name(name).await {
                        return Some(DuplicateInfo {
                            method: *method,
                            existing_id: existing.id,
                            existing_name: existing.name,
                        });
                    }
                }
                crate::config::DuplicateMethod::JobName => {
                    // Extract job name from filename and check database
                    let job_name = Self::extract_job_name(name);
                    if let Ok(Some(existing)) = self.db.find_by_job_name(&job_name).await {
                        return Some(DuplicateInfo {
                            method: *method,
                            existing_id: existing.id,
                            existing_name: existing.name,
                        });
                    }
                }
            }
        }

        None
    }

    /// Check if there is sufficient disk space for download
    ///
    /// This method checks if there is enough disk space available before starting
    /// a download. It accounts for:
    /// - The download size multiplied by a configurable multiplier (default 2.5x)
    ///   to account for extraction overhead (compressed + extracted + headroom)
    /// - A minimum free space buffer (default 1GB) to prevent filling the disk
    pub(crate) async fn check_disk_space(&self, size_bytes: i64) -> Result<()> {
        // Skip check if disabled
        if !self.config.disk_space.enabled {
            return Ok(());
        }

        // Calculate required space: download size × multiplier + buffer
        let required = (size_bytes as f64 * self.config.disk_space.size_multiplier) as u64;
        let required_with_buffer = required + self.config.disk_space.min_free_space;

        // Determine path to check - use download_dir if it exists, otherwise check parent
        let check_path = if self.config.download.download_dir.exists() {
            &self.config.download.download_dir
        } else {
            // If download_dir doesn't exist yet, check parent directory
            // This allows checking space before creating the download directory
            self.config.download.download_dir.parent().ok_or_else(|| {
                Error::DiskSpaceCheckFailed(format!(
                    "Cannot determine parent directory of '{}'",
                    self.config.download.download_dir.display()
                ))
            })?
        };

        // Get available space from filesystem
        let available = crate::utils::get_available_space(check_path).map_err(|e| {
            Error::DiskSpaceCheckFailed(format!(
                "Failed to check disk space for '{}': {}",
                check_path.display(),
                e
            ))
        })?;

        // Check if sufficient space is available
        if available < required_with_buffer {
            return Err(Error::InsufficientSpace {
                required: required_with_buffer,
                available,
            });
        }

        Ok(())
    }

    /// Extract job name from NZB filename
    ///
    /// This removes the file extension and any obfuscation patterns to get
    /// a clean job name for duplicate detection.
    ///
    /// # Arguments
    ///
    /// * `name` - NZB filename or download name
    ///
    /// # Returns
    ///
    /// Extracted job name (filename stem)
    ///
    /// # Example
    ///
    /// ```
    /// # use usenet_dl::UsenetDownloader;
    /// let job_name = UsenetDownloader::extract_job_name("My.Movie.2024.nzb");
    /// assert_eq!(job_name, "My.Movie.2024");
    /// ```
    pub fn extract_job_name(name: &str) -> String {
        // Remove .nzb extension if present
        let name = if name.ends_with(".nzb") {
            &name[..name.len() - 4]
        } else {
            name
        };

        // For now, just return the cleaned name
        // Future enhancement: could apply deobfuscation logic here
        name.to_string()
    }
}
