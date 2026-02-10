//! Download lifecycle control — pause, resume, cancel, priority, reprocess.

use crate::error::{DatabaseError, DownloadError, Error, Result};
use crate::types::{DownloadId, Event, Priority, Stage, Status};
use std::path::PathBuf;

use super::UsenetDownloader;

impl UsenetDownloader {
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
        let download = self.db.get_download(id).await?.ok_or_else(|| {
            Error::Database(DatabaseError::NotFound(format!(
                "Download {} not found",
                id
            )))
        })?;

        let current_status = Status::from_i32(download.status);

        // Check if download can be paused
        match current_status {
            Status::Paused => {
                // Already paused, nothing to do
                return Ok(());
            }
            Status::Complete | Status::Failed => {
                return Err(Error::Download(DownloadError::InvalidState {
                    id: id.into(),
                    operation: "pause".to_string(),
                    current_state: format!("{:?}", current_status),
                }));
            }
            Status::Queued | Status::Downloading | Status::Processing => {
                // Can be paused
            }
        }

        // If download is actively running, cancel its task
        let mut active_downloads = self.queue_state.active_downloads.lock().await;
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
        let download = self.db.get_download(id).await?.ok_or_else(|| {
            Error::Database(DatabaseError::NotFound(format!(
                "Download {} not found",
                id
            )))
        })?;

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
                return Err(Error::Download(DownloadError::InvalidState {
                    id: id.into(),
                    operation: "resume".to_string(),
                    current_state: format!("{:?}", current_status),
                }));
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
    pub async fn resume_download(&self, id: DownloadId) -> Result<()> {
        // Get pending articles for this download
        let pending_articles = self.db.get_pending_articles(id).await?;

        if pending_articles.is_empty() {
            // All articles downloaded — mark as Processing.
            // The caller is responsible for spawning post-processing if needed.
            tracing::info!(
                download_id = id.0,
                "No pending articles - marking as Processing"
            );

            self.db
                .update_status(id, Status::Processing.to_i32())
                .await?;

            Ok(())
        } else {
            // Resume downloading remaining articles
            tracing::info!(
                download_id = id.0,
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

    /// Cancel a download and delete its files
    ///
    /// This method removes a download from the queue, stops it if actively running,
    /// deletes all downloaded files from the temp directory, and removes it from the database.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to cancel
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
        let _download = self.db.get_download(id).await?.ok_or_else(|| {
            Error::Database(DatabaseError::NotFound(format!(
                "Download {} not found",
                id
            )))
        })?;

        // If download is actively running, cancel its task
        let mut active_downloads = self.queue_state.active_downloads.lock().await;
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
        let download_temp_dir = self
            .config
            .download
            .temp_dir
            .join(format!("download_{}", id.0));
        if download_temp_dir.exists()
            && let Err(e) = tokio::fs::remove_dir_all(&download_temp_dir).await
        {
            tracing::warn!(
                download_id = id.0,
                path = ?download_temp_dir,
                error = %e,
                "Failed to delete download temp directory"
            );
            // Continue anyway - database deletion is more important
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
        let download = self.db.get_download(id).await?.ok_or_else(|| {
            Error::Database(DatabaseError::NotFound(format!(
                "Download {} not found",
                id
            )))
        })?;

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
    pub async fn reprocess(&self, id: DownloadId) -> Result<()> {
        // Get download from database
        let _download = self
            .db
            .get_download(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Download {} not found", id.0)))?;

        // Determine download path (temp directory)
        let download_path = self
            .config
            .download
            .temp_dir
            .join(format!("download_{}", id.0));

        // Verify download files still exist
        if !download_path.exists() {
            return Err(Error::NotFound(format!(
                "Download files not found at {}. Cannot reprocess.",
                download_path.display()
            )));
        }

        tracing::info!(
            download_id = id.0,
            path = %download_path.display(),
            "Starting reprocessing"
        );

        // Reset status and re-queue for post-processing
        self.db
            .update_status(id, Status::Processing.to_i32())
            .await?;

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
                    download_id = id.0,
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
    pub async fn reextract(&self, id: DownloadId) -> Result<()> {
        // Get download from database
        let download = self
            .db
            .get_download(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Download {} not found", id.0)))?;

        // Determine download path (temp directory)
        let download_path = self
            .config
            .download
            .temp_dir
            .join(format!("download_{}", id.0));

        // Verify download files still exist
        if !download_path.exists() {
            return Err(Error::NotFound(format!(
                "Download files not found at {}. Cannot re-extract.",
                download_path.display()
            )));
        }

        tracing::info!(
            download_id = id.0,
            path = %download_path.display(),
            "Starting re-extraction (skip verify/repair)"
        );

        // Reset status to processing
        self.db
            .update_status(id, Status::Processing.to_i32())
            .await?;

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
        let destination = PathBuf::from(download.destination.clone());
        let post_processor = self.processing.post_processor.clone();
        tokio::spawn(async move {
            // Run re-extraction (extract + move, skip verify/repair)
            match post_processor
                .reextract(id, download_path, destination)
                .await
            {
                Ok(final_path) => {
                    downloader
                        .handle_reextract_success(id, final_path, download)
                        .await;
                }
                Err(e) => {
                    downloader.handle_reextract_failure(id, e, download).await;
                }
            }
        });

        Ok(())
    }

    /// Handle successful re-extraction completion
    async fn handle_reextract_success(
        &self,
        id: DownloadId,
        final_path: PathBuf,
        download: crate::db::Download,
    ) {
        tracing::info!(download_id = id.0, ?final_path, "Re-extraction complete");

        // Update status to complete
        if let Err(e) = self.db.update_status(id, Status::Complete.to_i32()).await {
            tracing::error!(
                download_id = id.0,
                error = %e,
                "Failed to update status to complete"
            );
        }

        // Emit Complete event
        self.emit_event(Event::Complete {
            id,
            path: final_path.clone(),
        });

        // Trigger webhooks for complete event
        self.trigger_webhooks(super::webhooks::TriggerWebhooksParams {
            event_type: crate::config::WebhookEvent::OnComplete,
            download_id: id,
            name: download.name.clone(),
            category: download.category.clone(),
            status: "complete".to_string(),
            destination: Some(final_path.clone()),
            error: None,
        });

        // Trigger scripts for complete event
        self.trigger_scripts(super::webhooks::TriggerScriptsParams {
            event_type: crate::config::ScriptEvent::OnComplete,
            download_id: id,
            name: download.name.clone(),
            category: download.category.clone(),
            status: "complete".to_string(),
            destination: Some(final_path),
            error: None,
            size_bytes: download.size_bytes as u64,
        });
    }

    /// Handle re-extraction failure
    async fn handle_reextract_failure(
        &self,
        id: DownloadId,
        error: crate::error::Error,
        download: crate::db::Download,
    ) {
        // Convert error to string once, reuse throughout
        let error_msg = error.to_string();

        tracing::error!(
            download_id = id.0,
            error = %error_msg,
            "Re-extraction failed"
        );

        // Update status to failed
        if let Err(db_err) = self.db.update_status(id, Status::Failed.to_i32()).await {
            tracing::error!(
                download_id = id.0,
                error = %db_err,
                "Failed to update status to failed"
            );
        }

        // Set error message
        if let Err(db_err) = self.db.set_error(id, &error_msg).await {
            tracing::error!(
                download_id = id.0,
                error = %db_err,
                "Failed to set error message"
            );
        }

        // Emit Failed event
        self.emit_event(Event::Failed {
            id,
            stage: Stage::Extract,
            error: error_msg.clone(),
            files_kept: true,
        });

        // Trigger webhooks for failed event
        self.trigger_webhooks(super::webhooks::TriggerWebhooksParams {
            event_type: crate::config::WebhookEvent::OnFailed,
            download_id: id,
            name: download.name.clone(),
            category: download.category.clone(),
            status: "failed".to_string(),
            destination: None,
            error: Some(error_msg.clone()),
        });

        // Trigger scripts for failed event
        self.trigger_scripts(super::webhooks::TriggerScriptsParams {
            event_type: crate::config::ScriptEvent::OnFailed,
            download_id: id,
            name: download.name.clone(),
            category: download.category.clone(),
            status: "failed".to_string(),
            destination: None,
            error: Some(error_msg),
            size_bytes: download.size_bytes as u64,
        });
    }

    /// Pause all active downloads
    ///
    /// This method pauses all downloads that are currently queued, downloading, or processing.
    /// Already paused, completed, or failed downloads are not affected.
    pub async fn pause_all(&self) -> Result<()> {
        // Get all downloads that can be paused (Queued, Downloading, Processing)
        let all_downloads = self.db.list_downloads().await?;

        let mut paused_count = 0;

        for download in all_downloads {
            let status = Status::from_i32(download.status);

            // Only pause active downloads
            match status {
                Status::Queued | Status::Downloading | Status::Processing => {
                    if let Err(e) = self.pause(DownloadId(download.id)).await {
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

        tracing::info!(paused_count = paused_count, "Paused all active downloads");

        // Emit global QueuePaused event
        self.emit_event(crate::types::Event::QueuePaused);

        Ok(())
    }

    /// Resume all paused downloads
    ///
    /// This method resumes all downloads that are currently paused.
    /// Downloads in other states (queued, downloading, complete, failed) are not affected.
    pub async fn resume_all(&self) -> Result<()> {
        // Get all paused downloads
        let paused_downloads = self
            .db
            .list_downloads_by_status(Status::Paused.to_i32())
            .await?;

        let mut resumed_count = 0;

        for download in paused_downloads {
            if let Err(e) = self.resume(DownloadId(download.id)).await {
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
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::db::Database;
    use crate::downloader::{ProcessingPipeline, QueueState, RuntimeConfig, UsenetDownloader};
    use crate::error::{DownloadError, Error};
    use crate::types::{DownloadId, DownloadOptions, Event, Priority, Status};
    use crate::{post_processing, speed_limiter};
    use std::sync::Arc;
    use tempfile::tempdir;

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

    async fn create_test_downloader() -> (UsenetDownloader, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let mut config = Config::default();
        config.persistence.database_path = db_path;
        config.servers = vec![];
        config.download.max_concurrent_downloads = 3;

        let db = Database::new(&config.persistence.database_path)
            .await
            .unwrap();

        let (event_tx, _rx) = tokio::sync::broadcast::channel(1000);
        let nntp_pools = Vec::new();
        let queue = Arc::new(tokio::sync::Mutex::new(std::collections::BinaryHeap::new()));
        let concurrent_limit = Arc::new(tokio::sync::Semaphore::new(
            config.download.max_concurrent_downloads,
        ));
        let active_downloads = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let speed_limiter = speed_limiter::SpeedLimiter::new(config.download.speed_limit_bps);
        let config_arc = Arc::new(config.clone());
        let categories = Arc::new(tokio::sync::RwLock::new(
            config.persistence.categories.clone(),
        ));
        let schedule_rules = Arc::new(tokio::sync::RwLock::new(vec![]));
        let next_schedule_rule_id = Arc::new(std::sync::atomic::AtomicI64::new(0));
        let parity_handler: Arc<dyn crate::ParityHandler> = Arc::new(crate::NoOpParityHandler);
        let db_arc = Arc::new(db);
        let post_processor = Arc::new(post_processing::PostProcessor::new(
            event_tx.clone(),
            config_arc.clone(),
            parity_handler.clone(),
            db_arc.clone(),
        ));

        let queue_state = QueueState {
            queue,
            concurrent_limit,
            active_downloads,
            accepting_new: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        };
        let runtime_config = RuntimeConfig {
            categories,
            schedule_rules,
            next_schedule_rule_id,
        };
        let processing = ProcessingPipeline {
            post_processor,
            parity_handler,
        };

        let downloader = UsenetDownloader {
            db: db_arc,
            event_tx,
            config: config_arc,
            nntp_pools: Arc::new(nntp_pools),
            speed_limiter,
            queue_state,
            runtime_config,
            processing,
        };

        (downloader, temp_dir)
    }

    // --- pause() tests ---

    #[tokio::test]
    async fn test_pause_queued_download_sets_status_to_paused() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Verify initial status is Queued
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Queued.to_i32());

        downloader.pause(id).await.unwrap();

        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(
            download.status,
            Status::Paused.to_i32(),
            "pausing a Queued download should set DB status to Paused"
        );
    }

    #[tokio::test]
    async fn test_pause_downloading_triggers_cancellation_token() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Simulate an active download by inserting a cancellation token
        let cancel_token = tokio_util::sync::CancellationToken::new();
        let token_clone = cancel_token.clone();
        downloader
            .queue_state
            .active_downloads
            .lock()
            .await
            .insert(id, cancel_token);

        // Set status to Downloading in DB
        downloader
            .db
            .update_status(id, Status::Downloading.to_i32())
            .await
            .unwrap();

        downloader.pause(id).await.unwrap();

        assert!(
            token_clone.is_cancelled(),
            "cancellation token should be triggered when pausing an active download"
        );

        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(
            download.status,
            Status::Paused.to_i32(),
            "status should be Paused after pausing an active download"
        );

        // Active downloads map should no longer contain this download
        let active = downloader.queue_state.active_downloads.lock().await;
        assert!(
            !active.contains_key(&id),
            "download should be removed from active_downloads map"
        );
    }

    #[tokio::test]
    async fn test_pause_already_paused_is_idempotent() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        downloader.pause(id).await.unwrap();

        // Pausing again should succeed silently
        let result = downloader.pause(id).await;
        assert!(
            result.is_ok(),
            "pausing an already-paused download should be idempotent"
        );

        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(
            download.status,
            Status::Paused.to_i32(),
            "status should remain Paused"
        );
    }

    #[tokio::test]
    async fn test_pause_complete_download_returns_invalid_state() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        downloader
            .db
            .update_status(id, Status::Complete.to_i32())
            .await
            .unwrap();

        let result = downloader.pause(id).await;

        match result {
            Err(Error::Download(DownloadError::InvalidState {
                id: err_id,
                operation,
                current_state,
            })) => {
                assert_eq!(
                    err_id, id.0,
                    "error should reference the correct download ID"
                );
                assert_eq!(
                    operation, "pause",
                    "error should specify 'pause' as the operation"
                );
                assert!(
                    current_state.contains("Complete"),
                    "error should report current state as Complete, got: {}",
                    current_state
                );
            }
            other => panic!(
                "expected InvalidState error for pausing Complete download, got: {:?}",
                other
            ),
        }

        // Status should be unchanged
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(
            download.status,
            Status::Complete.to_i32(),
            "status should remain Complete after failed pause"
        );
    }

    #[tokio::test]
    async fn test_pause_failed_download_returns_invalid_state() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        downloader
            .db
            .update_status(id, Status::Failed.to_i32())
            .await
            .unwrap();

        let result = downloader.pause(id).await;

        match result {
            Err(Error::Download(DownloadError::InvalidState {
                id: err_id,
                operation,
                current_state,
            })) => {
                assert_eq!(err_id, id.0);
                assert_eq!(operation, "pause");
                assert!(
                    current_state.contains("Failed"),
                    "error should report current state as Failed, got: {}",
                    current_state
                );
            }
            other => panic!(
                "expected InvalidState error for pausing Failed download, got: {:?}",
                other
            ),
        }
    }

    // --- resume() tests ---

    #[tokio::test]
    async fn test_resume_paused_download_sets_queued_and_adds_to_queue() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        downloader.pause(id).await.unwrap();

        // Queue should be empty after pause
        let queue_size = downloader.queue_state.queue.lock().await.len();
        assert_eq!(queue_size, 0, "queue should be empty after pause");

        downloader.resume(id).await.unwrap();

        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(
            download.status,
            Status::Queued.to_i32(),
            "resumed download should have status Queued"
        );

        let queue_size = downloader.queue_state.queue.lock().await.len();
        assert_eq!(
            queue_size, 1,
            "resumed download should be added back to the queue"
        );
    }

    #[tokio::test]
    async fn test_resume_queued_download_is_idempotent() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Download starts as Queued
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Queued.to_i32());

        // Resuming a Queued download should return Ok
        let result = downloader.resume(id).await;
        assert!(
            result.is_ok(),
            "resuming a Queued download should be idempotent"
        );

        // Status should still be Queued, not double-queued
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(download.status, Status::Queued.to_i32());
    }

    #[tokio::test]
    async fn test_resume_complete_download_returns_invalid_state() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        downloader
            .db
            .update_status(id, Status::Complete.to_i32())
            .await
            .unwrap();

        let result = downloader.resume(id).await;

        match result {
            Err(Error::Download(DownloadError::InvalidState {
                id: err_id,
                operation,
                current_state,
            })) => {
                assert_eq!(err_id, id.0);
                assert_eq!(operation, "resume");
                assert!(
                    current_state.contains("Complete"),
                    "error should report Complete state, got: {}",
                    current_state
                );
            }
            other => panic!(
                "expected InvalidState for resuming Complete download, got: {:?}",
                other
            ),
        }
    }

    // --- cancel() tests ---

    #[tokio::test]
    async fn test_cancel_removes_from_db_and_emits_removed_event() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        let mut events = downloader.subscribe();

        downloader.cancel(id).await.unwrap();

        // Download should be gone from DB
        let download = downloader.db.get_download(id).await.unwrap();
        assert!(
            download.is_none(),
            "cancelled download should be removed from DB"
        );

        // Queue should be empty
        let queue_size = downloader.queue_state.queue.lock().await.len();
        assert_eq!(queue_size, 0, "queue should be empty after cancel");

        // Should receive Removed event
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                match events.recv().await {
                    Ok(Event::Removed { id: event_id }) => return event_id,
                    Ok(_) => continue,
                    Err(_) => panic!("event channel closed without Removed event"),
                }
            }
        })
        .await
        .expect("timed out waiting for Removed event");

        assert_eq!(
            event, id,
            "Removed event should carry the cancelled download's ID"
        );
    }

    #[tokio::test]
    async fn test_cancel_deletes_temp_directory() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // Create simulated temp directory with a file
        let download_temp_dir = downloader
            .config
            .download
            .temp_dir
            .join(format!("download_{}", id));
        tokio::fs::create_dir_all(&download_temp_dir).await.unwrap();
        let test_file = download_temp_dir.join("article_1.dat");
        tokio::fs::write(&test_file, b"test data").await.unwrap();
        assert!(download_temp_dir.exists());

        downloader.cancel(id).await.unwrap();

        assert!(
            !download_temp_dir.exists(),
            "temp directory should be deleted after cancel"
        );
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_download_returns_not_found() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let result = downloader.cancel(DownloadId(99999)).await;

        match result {
            Err(Error::Database(crate::error::DatabaseError::NotFound(msg))) => {
                assert!(
                    msg.contains("99999"),
                    "error message should contain the ID, got: {}",
                    msg
                );
            }
            other => panic!("expected NotFound error, got: {:?}", other),
        }
    }

    // --- set_priority() tests ---

    #[tokio::test]
    async fn test_set_priority_on_queued_download_reorders_queue() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id_normal = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "normal", DownloadOptions::default())
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

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

        // Before priority change, normal should come first
        {
            let queue = downloader.queue_state.queue.lock().await;
            let first = queue.peek().unwrap();
            assert_eq!(
                first.id, id_normal,
                "normal should be first before priority change"
            );
        }

        // Upgrade the low download to High
        downloader
            .set_priority(id_low, Priority::High)
            .await
            .unwrap();

        // After priority change, the formerly-low download should be first
        {
            let mut queue = downloader.queue_state.queue.lock().await;
            let first = queue.pop().unwrap();
            assert_eq!(
                first.id, id_low,
                "download upgraded to High should now be first in queue"
            );
            assert_eq!(
                first.priority,
                Priority::High,
                "queue entry should reflect the new priority"
            );
        }

        // Verify DB was updated
        let download = downloader.db.get_download(id_low).await.unwrap().unwrap();
        assert_eq!(
            download.priority,
            Priority::High as i32,
            "priority should be updated in DB"
        );
    }

    // --- pause_all() tests ---

    #[tokio::test]
    async fn test_pause_all_pauses_queued_skips_paused_complete_failed() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Create downloads in various states
        let id_queued = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "queued", DownloadOptions::default())
            .await
            .unwrap();

        let id_paused = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "paused", DownloadOptions::default())
            .await
            .unwrap();
        downloader.pause(id_paused).await.unwrap();

        let id_complete = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "complete",
                DownloadOptions::default(),
            )
            .await
            .unwrap();
        downloader
            .db
            .update_status(id_complete, Status::Complete.to_i32())
            .await
            .unwrap();

        let id_failed = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "failed", DownloadOptions::default())
            .await
            .unwrap();
        downloader
            .db
            .update_status(id_failed, Status::Failed.to_i32())
            .await
            .unwrap();

        downloader.pause_all().await.unwrap();

        let d_queued = downloader
            .db
            .get_download(id_queued)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            d_queued.status,
            Status::Paused.to_i32(),
            "Queued download should be paused by pause_all"
        );

        let d_paused = downloader
            .db
            .get_download(id_paused)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            d_paused.status,
            Status::Paused.to_i32(),
            "already-Paused download should remain Paused"
        );

        let d_complete = downloader
            .db
            .get_download(id_complete)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            d_complete.status,
            Status::Complete.to_i32(),
            "Complete download should not be affected by pause_all"
        );

        let d_failed = downloader
            .db
            .get_download(id_failed)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            d_failed.status,
            Status::Failed.to_i32(),
            "Failed download should not be affected by pause_all"
        );
    }

    // --- resume_all() tests ---

    #[tokio::test]
    async fn test_resume_all_resumes_only_paused_and_emits_queue_resumed() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id1 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "paused1", DownloadOptions::default())
            .await
            .unwrap();
        downloader.pause(id1).await.unwrap();

        let id2 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "paused2", DownloadOptions::default())
            .await
            .unwrap();
        downloader.pause(id2).await.unwrap();

        let id_complete = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "complete",
                DownloadOptions::default(),
            )
            .await
            .unwrap();
        downloader
            .db
            .update_status(id_complete, Status::Complete.to_i32())
            .await
            .unwrap();

        let mut events = downloader.subscribe();

        downloader.resume_all().await.unwrap();

        // Paused downloads should now be Queued
        let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
        assert_eq!(
            d1.status,
            Status::Queued.to_i32(),
            "paused download should be resumed to Queued"
        );

        let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
        assert_eq!(
            d2.status,
            Status::Queued.to_i32(),
            "paused download should be resumed to Queued"
        );

        // Complete download should be untouched
        let d_complete = downloader
            .db
            .get_download(id_complete)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            d_complete.status,
            Status::Complete.to_i32(),
            "Complete download should not be affected by resume_all"
        );

        // Should emit QueueResumed event
        let received = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                match events.recv().await {
                    Ok(Event::QueueResumed) => return true,
                    Ok(_) => continue,
                    Err(_) => return false,
                }
            }
        })
        .await
        .expect("timed out waiting for QueueResumed event");

        assert!(received, "resume_all should emit QueueResumed event");
    }
}
