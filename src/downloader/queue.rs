//! Priority queue management for download ordering.

use crate::error::{DatabaseError, Error, Result};
use crate::types::{DownloadId, Priority, Status};

use super::{QueuedDownload, UsenetDownloader};

impl UsenetDownloader {
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
    pub(crate) async fn add_to_queue(&self, id: DownloadId) -> Result<()> {
        // Fetch download from database to get priority and created_at
        let download = self.db.get_download(id).await?.ok_or_else(|| {
            Error::Database(DatabaseError::NotFound(format!(
                "Download {} not found",
                id
            )))
        })?;

        let queued_download = QueuedDownload {
            id,
            priority: Priority::from_i32(download.priority),
            created_at: download.created_at,
        };

        // Add to priority queue
        let mut queue = self.queue_state.queue.lock().await;
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
    pub(crate) async fn remove_from_queue(&self, id: DownloadId) -> bool {
        let mut queue = self.queue_state.queue.lock().await;

        let original_len = queue.len();

        // Collect all items except the one we want to remove
        let items: Vec<_> = queue.drain().filter(|item| item.id != id).collect();

        let was_removed = items.len() < original_len;

        // Rebuild queue without the removed item
        *queue = items.into_iter().collect();

        was_removed
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
    pub async fn restore_queue(&self) -> Result<Vec<DownloadId>> {
        tracing::info!("Restoring queue from database");

        // Get all incomplete downloads (status IN (0=Queued, 1=Downloading, 3=Processing))
        let incomplete_downloads = self.db.get_incomplete_downloads().await?;

        if incomplete_downloads.is_empty() {
            tracing::info!("No incomplete downloads to restore");
            return Ok(Vec::new());
        }

        tracing::info!(
            count = incomplete_downloads.len(),
            "Found incomplete downloads to restore"
        );

        // Store count before iterating
        let restore_count = incomplete_downloads.len();

        // Collect IDs that need post-processing (status became Processing after resume)
        let mut needs_post_processing = Vec::new();

        // Process each download based on its status
        for download in incomplete_downloads {
            let id = DownloadId(download.id);
            let status = Status::from_i32(download.status);

            match status {
                Status::Downloading | Status::Processing => {
                    // These were actively running - resume them
                    tracing::info!(
                        download_id = download.id,
                        status = ?status,
                        "Resuming interrupted download"
                    );
                    self.resume_download(id).await?;

                    // Check if resume_download set the status to Processing
                    // (meaning all articles are downloaded and it needs post-processing)
                    let updated = self.db.get_download(id).await?;
                    if let Some(dl) = updated
                        && Status::from_i32(dl.status) == Status::Processing
                    {
                        needs_post_processing.push(id);
                    }
                }
                Status::Queued => {
                    // These were waiting in queue - add back to queue
                    tracing::info!(
                        download_id = download.id,
                        "Re-adding queued download to priority queue"
                    );
                    self.add_to_queue(id).await?;
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

        tracing::info!(restored_count = restore_count, "Queue restoration complete");

        Ok(needs_post_processing)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::db::Database;
    use crate::downloader::{ProcessingPipeline, QueueState, RuntimeConfig, UsenetDownloader};
    use crate::error::{DatabaseError, Error};
    use crate::types::{DownloadId, DownloadOptions, Priority, Status};
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

    // --- add_to_queue() tests ---

    #[tokio::test]
    async fn test_add_to_queue_download_appears_in_queue() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        // add_nzb_content already calls add_to_queue internally,
        // verify it's there
        let queue = downloader.queue_state.queue.lock().await;
        assert_eq!(queue.len(), 1, "queue should contain the added download");

        let peeked = queue.peek().unwrap();
        assert_eq!(peeked.id, id, "queued download ID should match");
        assert_eq!(
            peeked.priority,
            Priority::Normal,
            "default priority should be Normal"
        );
    }

    #[tokio::test]
    async fn test_add_to_queue_nonexistent_download_returns_not_found() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let result = downloader.add_to_queue(DownloadId(99999)).await;

        match result {
            Err(Error::Database(DatabaseError::NotFound(msg))) => {
                assert!(
                    msg.contains("99999"),
                    "error should mention the nonexistent ID, got: {}",
                    msg
                );
            }
            other => panic!("expected NotFound error, got: {:?}", other),
        }
    }

    // --- remove_from_queue() tests ---

    #[tokio::test]
    async fn test_remove_from_queue_returns_true_and_removes() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
            .await
            .unwrap();

        assert_eq!(downloader.queue_state.queue.lock().await.len(), 1);

        let removed = downloader.remove_from_queue(id).await;
        assert!(
            removed,
            "remove_from_queue should return true for existing download"
        );

        assert_eq!(
            downloader.queue_state.queue.lock().await.len(),
            0,
            "queue should be empty after removal"
        );
    }

    #[tokio::test]
    async fn test_remove_from_queue_nonexistent_returns_false() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let removed = downloader.remove_from_queue(DownloadId(99999)).await;
        assert!(
            !removed,
            "remove_from_queue should return false for nonexistent ID"
        );
    }

    // --- priority ordering tests ---

    #[tokio::test]
    async fn test_priority_ordering_high_before_normal_before_low() {
        let (downloader, _temp_dir) = create_test_downloader().await;

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

        let mut queue = downloader.queue_state.queue.lock().await;

        let first = queue.pop().unwrap();
        assert_eq!(first.id, id_high, "High priority should be dequeued first");

        let second = queue.pop().unwrap();
        assert_eq!(
            second.id, id_normal,
            "Normal priority should be dequeued second"
        );

        let third = queue.pop().unwrap();
        assert_eq!(third.id, id_low, "Low priority should be dequeued third");

        assert!(queue.pop().is_none(), "queue should be empty");
    }

    // --- restore_queue() tests ---

    #[tokio::test]
    async fn test_restore_queue_with_queued_downloads_readds_to_queue() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add downloads
        let id1 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
            .await
            .unwrap();

        let id2 = downloader
            .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
            .await
            .unwrap();

        // Clear the in-memory queue to simulate restart
        {
            let mut queue = downloader.queue_state.queue.lock().await;
            queue.clear();
        }

        // Queue should be empty now
        assert_eq!(downloader.queue_state.queue.lock().await.len(), 0);

        // Restore queue from DB
        let needs_pp = downloader.restore_queue().await.unwrap();
        assert!(
            needs_pp.is_empty(),
            "Queued downloads should not need post-processing"
        );

        // Queue should be repopulated
        let queue_size = downloader.queue_state.queue.lock().await.len();
        assert_eq!(
            queue_size, 2,
            "restore_queue should re-add all Queued downloads"
        );

        // Verify both downloads are in the queue
        let mut queue = downloader.queue_state.queue.lock().await;
        let ids: Vec<DownloadId> = std::iter::from_fn(|| queue.pop().map(|q| q.id)).collect();
        assert!(ids.contains(&id1), "queue should contain id1");
        assert!(ids.contains(&id2), "queue should contain id2");
    }

    #[tokio::test]
    async fn test_restore_queue_downloading_with_pending_articles_requeues() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download (creates articles with status=PENDING)
        let id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "downloading-test",
                DownloadOptions::default(),
            )
            .await
            .unwrap();

        // Simulate that download was actively downloading when the app crashed
        downloader
            .db
            .update_status(id, Status::Downloading.to_i32())
            .await
            .unwrap();

        // Clear in-memory queue to simulate restart
        {
            downloader.queue_state.queue.lock().await.clear();
        }

        let needs_pp = downloader.restore_queue().await.unwrap();

        // Articles are still pending, so it should be re-queued, not post-processed
        assert!(
            needs_pp.is_empty(),
            "download with pending articles should not need post-processing"
        );

        // Verify status was changed back to Queued for re-download
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(
            Status::from_i32(download.status),
            Status::Queued,
            "downloading download with pending articles should be re-queued"
        );

        // Verify it's in the in-memory queue
        let queue_size = downloader.queue_state.queue.lock().await.len();
        assert_eq!(queue_size, 1, "download should be added back to the queue");
    }

    #[tokio::test]
    async fn test_restore_queue_downloading_all_articles_done_needs_post_processing() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "all-done-test",
                DownloadOptions::default(),
            )
            .await
            .unwrap();

        // Mark all articles as downloaded (simulating completed download phase)
        let articles = downloader.db.get_articles(id).await.unwrap();
        assert!(
            !articles.is_empty(),
            "NZB should have created articles in the DB"
        );
        for article in &articles {
            downloader
                .db
                .update_article_status(article.id, crate::db::article_status::DOWNLOADED)
                .await
                .unwrap();
        }

        // Set status to Downloading (app crashed after all articles finished but before post-processing)
        downloader
            .db
            .update_status(id, Status::Downloading.to_i32())
            .await
            .unwrap();

        // Clear queue to simulate restart
        {
            downloader.queue_state.queue.lock().await.clear();
        }

        let needs_pp = downloader.restore_queue().await.unwrap();

        // All articles downloaded → should be marked Processing and returned for post-processing
        assert_eq!(
            needs_pp.len(),
            1,
            "download with all articles done should need post-processing"
        );
        assert_eq!(needs_pp[0], id, "returned ID should match the download");

        // Verify status is Processing in DB
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(
            Status::from_i32(download.status),
            Status::Processing,
            "download with all articles done should be marked Processing"
        );

        // Should NOT be in the queue (it needs post-processing, not downloading)
        let queue_size = downloader.queue_state.queue.lock().await.len();
        assert_eq!(
            queue_size, 0,
            "completed download should not be in the download queue"
        );
    }

    #[tokio::test]
    async fn test_restore_queue_processing_status_needs_post_processing() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        let id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "processing-test",
                DownloadOptions::default(),
            )
            .await
            .unwrap();

        // Mark all articles as downloaded
        let articles = downloader.db.get_articles(id).await.unwrap();
        for article in &articles {
            downloader
                .db
                .update_article_status(article.id, crate::db::article_status::DOWNLOADED)
                .await
                .unwrap();
        }

        // Set status to Processing (app crashed during post-processing)
        downloader
            .db
            .update_status(id, Status::Processing.to_i32())
            .await
            .unwrap();

        // Clear queue to simulate restart
        {
            downloader.queue_state.queue.lock().await.clear();
        }

        let needs_pp = downloader.restore_queue().await.unwrap();

        // Was already Processing → should be returned for post-processing
        assert_eq!(
            needs_pp.len(),
            1,
            "Processing download should be returned for post-processing"
        );
        assert_eq!(needs_pp[0], id);

        // Status should remain Processing
        let download = downloader.db.get_download(id).await.unwrap().unwrap();
        assert_eq!(
            Status::from_i32(download.status),
            Status::Processing,
            "Processing download should stay in Processing state"
        );
    }

    #[tokio::test]
    async fn test_restore_queue_with_no_incomplete_downloads_returns_empty() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Add a download and mark it complete
        let id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "complete",
                DownloadOptions::default(),
            )
            .await
            .unwrap();

        downloader
            .db
            .update_status(id, Status::Complete.to_i32())
            .await
            .unwrap();

        // Clear queue
        {
            let mut queue = downloader.queue_state.queue.lock().await;
            queue.clear();
        }

        let needs_pp = downloader.restore_queue().await.unwrap();

        assert!(
            needs_pp.is_empty(),
            "no incomplete downloads should mean no post-processing needed"
        );

        let queue_size = downloader.queue_state.queue.lock().await.len();
        assert_eq!(
            queue_size, 0,
            "queue should remain empty when no incomplete downloads exist"
        );
    }
}
