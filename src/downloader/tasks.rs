//! Background task orchestration — queue processor, folder watcher, RSS scheduler,
//! time-based scheduler, and download task spawning.

use crate::config;
use crate::error::{DatabaseError, Error, Result};
use crate::folder_watcher;
use crate::rss_manager;
use crate::rss_scheduler;
use crate::scheduler;
use crate::scheduler_task;
use crate::types::{DownloadId, Event, Status};
use futures::stream::{self, StreamExt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::UsenetDownloader;

/// Interval between queue polling attempts when the queue is empty
const QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Buffer size for the article status update channel
const ARTICLE_CHANNEL_BUFFER: usize = 500;

/// Interval between progress update emissions
const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

/// Batch size threshold for flushing article status updates to the database
const ARTICLE_BATCH_SIZE: usize = 100;

/// Maximum article failure ratio before considering a download failed (50%)
const MAX_FAILURE_RATIO: f64 = 0.5;

/// Shared context for a single download task, reducing parameter passing between helpers.
struct DownloadTaskContext {
    id: DownloadId,
    db: Arc<crate::db::Database>,
    event_tx: tokio::sync::broadcast::Sender<Event>,
    nntp_pools: Arc<Vec<nntp_rs::NntpPool>>,
    config: Arc<crate::config::Config>,
    active_downloads: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<DownloadId, tokio_util::sync::CancellationToken>,
        >,
    >,
    speed_limiter: crate::speed_limiter::SpeedLimiter,
    cancel_token: tokio_util::sync::CancellationToken,
    downloader: UsenetDownloader,
}

impl DownloadTaskContext {
    /// Remove this download from the active downloads map.
    async fn remove_from_active(&self) {
        let mut active = self.active_downloads.lock().await;
        active.remove(&self.id);
    }

    /// Mark the download as failed with an error message and emit the failure event.
    async fn mark_failed(&self, error: &str) {
        let _ = self.db.update_status(self.id, Status::Failed.to_i32()).await;
        let _ = self.db.set_error(self.id, error).await;
        self.event_tx
            .send(Event::DownloadFailed {
                id: self.id,
                error: error.to_string(),
            })
            .ok();
    }

    /// Spawn post-processing as an independent background task.
    fn spawn_post_processing(self) {
        tokio::spawn(async move {
            if let Err(e) = self.downloader.start_post_processing(self.id).await {
                tracing::error!(
                    download_id = self.id,
                    error = %e,
                    "Post-processing failed"
                );
            }
        });
    }
}

/// Aggregated result counts from downloading article batches.
struct DownloadResults {
    success_count: usize,
    failed_count: usize,
    first_error: Option<String>,
}

impl UsenetDownloader {
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
    /// # Parallel Download Behavior
    ///
    /// Each spawned download task downloads articles **in parallel** using all configured
    /// NNTP connections. The concurrency is automatically calculated as the sum of connections
    /// across all servers (e.g., 50 connections = 50 articles downloading simultaneously).
    pub fn start_queue_processor(&self) -> tokio::task::JoinHandle<()> {
        let queue = self.queue.clone();
        let concurrent_limit = self.concurrent_limit.clone();
        let db = self.db.clone();
        let event_tx = self.event_tx.clone();
        let nntp_pools = self.nntp_pools.clone();
        let config = self.config.clone();
        let active_downloads = self.active_downloads.clone();
        let speed_limiter = self.speed_limiter.clone();
        let downloader = self.clone();

        tokio::spawn(async move {
            loop {
                // Get the next download from the queue
                let download_id = {
                    let mut queue_guard = queue.lock().await;
                    queue_guard.pop().map(|item| item.id)
                };

                if let Some(id) = download_id {
                    // Acquire a permit from the semaphore (blocks if at max concurrent downloads)
                    let permit = concurrent_limit.clone().acquire_owned().await;

                    let permit = match permit {
                        Ok(p) => p,
                        Err(_) => {
                            // Semaphore closed, exit processor
                            break;
                        }
                    };

                    // Create cancellation token for this download
                    let cancel_token = tokio_util::sync::CancellationToken::new();

                    // Register the cancellation token
                    {
                        let mut active = active_downloads.lock().await;
                        active.insert(id, cancel_token.clone());
                    }

                    let ctx = DownloadTaskContext {
                        id,
                        db: Arc::clone(&db),
                        event_tx: event_tx.clone(),
                        nntp_pools: Arc::clone(&nntp_pools),
                        config: Arc::clone(&config),
                        active_downloads: Arc::clone(&active_downloads),
                        speed_limiter: speed_limiter.clone(),
                        cancel_token,
                        downloader: downloader.clone(),
                    };

                    // Spawn the download task
                    tokio::spawn(async move {
                        let _permit = permit;
                        Self::run_download_task(ctx).await;
                    });
                } else {
                    // Queue is empty, wait a bit before checking again
                    tokio::time::sleep(QUEUE_POLL_INTERVAL).await;
                }
            }
        })
    }

    /// Core download task — orchestrates the full lifecycle of a single download.
    ///
    /// Phases:
    /// 1. Fetch and validate the download record
    /// 2. Transition to Downloading state
    /// 3. Download all pending articles in parallel batches
    /// 4. Evaluate results and finalize status
    /// 5. Trigger post-processing
    async fn run_download_task(ctx: DownloadTaskContext) {
        let id = ctx.id;

        // Phase 1: Fetch download record and pending articles
        let (download, pending_articles) = match Self::fetch_download_record(&ctx).await {
            Some(pair) => pair,
            None => return, // Already logged and cleaned up
        };

        // Phase 2: Handle empty article list (nothing to download)
        if pending_articles.is_empty() {
            ctx.event_tx.send(Event::DownloadComplete { id }).ok();
            ctx.remove_from_active().await;
            ctx.spawn_post_processing();
            return;
        }

        // Phase 3: Create temp directory
        let download_temp_dir = ctx.config.download.temp_dir.join(format!("download_{}", id));
        if let Err(e) = tokio::fs::create_dir_all(&download_temp_dir).await {
            let msg = format!("Failed to create temp directory: {}", e);
            tracing::error!(download_id = id, error = %e, "Failed to create temp directory");
            ctx.mark_failed(&msg).await;
            ctx.remove_from_active().await;
            return;
        }

        // Phase 4: Download articles
        let total_articles = pending_articles.len();
        let total_size_bytes = download.size_bytes as u64;
        let results = Self::download_articles(
            &ctx,
            pending_articles,
            total_size_bytes,
            &download_temp_dir,
        )
        .await;

        // Phase 5: Finalize based on results
        Self::finalize_download(ctx, results, total_articles, total_size_bytes).await;
    }

    /// Fetch the download record and its pending articles, transitioning to Downloading state.
    ///
    /// Returns `None` if the download is not found, the DB fails, or there's an error
    /// updating the status — in all cases the download is removed from active tracking.
    async fn fetch_download_record(
        ctx: &DownloadTaskContext,
    ) -> Option<(crate::db::Download, Vec<crate::db::Article>)> {
        let id = ctx.id;

        // Fetch download record
        let download = match ctx.db.get_download(id).await {
            Ok(Some(d)) => d,
            Ok(None) => {
                tracing::warn!(download_id = id, "Download not found in database");
                ctx.remove_from_active().await;
                return None;
            }
            Err(e) => {
                tracing::error!(download_id = id, error = %e, "Failed to fetch download");
                ctx.remove_from_active().await;
                return None;
            }
        };

        // Update status to Downloading and record start time
        if let Err(e) = ctx
            .db
            .update_status(id, Status::Downloading.to_i32())
            .await
        {
            tracing::error!(download_id = id, error = %e, "Failed to update status");
            ctx.remove_from_active().await;
            return None;
        }
        if let Err(e) = ctx.db.set_started(id).await {
            tracing::error!(download_id = id, error = %e, "Failed to set start time");
            ctx.remove_from_active().await;
            return None;
        }

        // Emit Downloading event (initial progress 0%)
        ctx.event_tx
            .send(Event::Downloading {
                id,
                percent: 0.0,
                speed_bps: 0,
            })
            .ok();

        // Get all pending articles
        let pending_articles = match ctx.db.get_pending_articles(id).await {
            Ok(articles) => articles,
            Err(e) => {
                tracing::error!(download_id = id, error = %e, "Failed to get pending articles");
                ctx.remove_from_active().await;
                return None;
            }
        };

        Some((download, pending_articles))
    }

    /// Download all pending articles in parallel batches with progress tracking.
    ///
    /// Sets up background tasks for progress reporting and database batching,
    /// then streams article batches through pipelined NNTP fetches.
    async fn download_articles(
        ctx: &DownloadTaskContext,
        pending_articles: Vec<crate::db::Article>,
        total_size_bytes: u64,
        download_temp_dir: &std::path::Path,
    ) -> DownloadResults {
        let id = ctx.id;
        let total_articles = pending_articles.len();
        let downloaded_articles = Arc::new(AtomicU64::new(0));
        let downloaded_bytes = Arc::new(AtomicU64::new(0));
        let download_start = std::time::Instant::now();

        // Spawn progress reporting task
        let progress_task = Self::spawn_progress_reporter(
            id,
            total_articles,
            total_size_bytes,
            download_start,
            Arc::clone(&downloaded_articles),
            Arc::clone(&downloaded_bytes),
            ctx.event_tx.clone(),
            Arc::clone(&ctx.db),
            ctx.cancel_token.child_token(),
        );

        // Spawn database update batching task
        let (batch_tx, batch_rx) =
            tokio::sync::mpsc::channel::<(i64, i32)>(ARTICLE_CHANNEL_BUFFER);
        let batch_task = Self::spawn_batch_updater(
            id,
            Arc::clone(&ctx.db),
            batch_rx,
            ctx.cancel_token.child_token(),
        );

        // Calculate concurrency and pipeline configuration
        let concurrency: usize = ctx.config.servers.iter().map(|s| s.connections).sum();
        let pipeline_depth = ctx
            .config
            .servers
            .first()
            .map(|s| s.pipeline_depth.max(1))
            .unwrap_or(10);

        // Collect chunks into owned Vecs for async closures
        let article_batches: Vec<Vec<_>> = pending_articles
            .chunks(pipeline_depth)
            .map(|chunk| chunk.to_vec())
            .collect();

        // Download articles in parallel using buffered stream
        let results: Vec<std::result::Result<Vec<(i32, u64)>, (String, usize)>> =
            stream::iter(article_batches)
                .map(|article_batch| {
                    let pool = Arc::clone(&ctx.nntp_pools);
                    let batch_tx = batch_tx.clone();
                    let speed_limiter = ctx.speed_limiter.clone();
                    let cancel_token = ctx.cancel_token.clone();
                    let download_temp_dir = download_temp_dir.to_path_buf();
                    let downloaded_bytes = Arc::clone(&downloaded_bytes);
                    let downloaded_articles = Arc::clone(&downloaded_articles);

                    async move {
                        Self::fetch_article_batch(
                            id,
                            article_batch,
                            pool,
                            batch_tx,
                            speed_limiter,
                            cancel_token,
                            download_temp_dir,
                            downloaded_bytes,
                            downloaded_articles,
                            pipeline_depth,
                        )
                        .await
                    }
                })
                .buffer_unordered(concurrency)
                .collect()
                .await;

        // Stop progress reporting task
        progress_task.abort();

        // Close batch channel and wait for final flush
        drop(batch_tx);
        if let Err(e) = batch_task.await {
            tracing::error!(download_id = id, error = %e, "Batch update task panicked");
        }

        // Aggregate results
        let mut success_count = 0;
        let mut failed_count = 0;
        let mut first_error: Option<String> = None;

        for result in results {
            match result {
                Ok(batch_results) => {
                    success_count += batch_results.len();
                }
                Err((error_msg, batch_size)) => {
                    failed_count += batch_size;
                    if first_error.is_none() {
                        first_error = Some(error_msg);
                    }
                }
            }
        }

        DownloadResults {
            success_count,
            failed_count,
            first_error,
        }
    }

    /// Fetch a single batch of articles via pipelined NNTP commands.
    async fn fetch_article_batch(
        id: DownloadId,
        article_batch: Vec<crate::db::Article>,
        nntp_pools: Arc<Vec<nntp_rs::NntpPool>>,
        batch_tx: tokio::sync::mpsc::Sender<(i64, i32)>,
        speed_limiter: crate::speed_limiter::SpeedLimiter,
        cancel_token: tokio_util::sync::CancellationToken,
        download_temp_dir: std::path::PathBuf,
        downloaded_bytes: Arc<AtomicU64>,
        downloaded_articles: Arc<AtomicU64>,
        pipeline_depth: usize,
    ) -> std::result::Result<Vec<(i32, u64)>, (String, usize)> {
        let batch_size = article_batch.len();

        // Check if download was cancelled
        if cancel_token.is_cancelled() {
            return Err(("Download cancelled".to_string(), batch_size));
        }

        // Get a connection from the first NNTP pool
        let pool = match nntp_pools.first() {
            Some(p) => p,
            None => {
                tracing::error!(download_id = id, "No NNTP pools configured");
                return Err(("No NNTP pools configured".to_string(), batch_size));
            }
        };

        let mut conn = match pool.get().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(download_id = id, error = %e, "Failed to get NNTP connection");
                return Err((
                    format!("Failed to get NNTP connection: {}", e),
                    batch_size,
                ));
            }
        };

        // Prepare message IDs for pipelined fetch
        let message_ids: Vec<std::borrow::Cow<'_, str>> = article_batch
            .iter()
            .map(|article| {
                if article.message_id.starts_with('<') {
                    std::borrow::Cow::Borrowed(article.message_id.as_str())
                } else {
                    std::borrow::Cow::Owned(format!("<{}>", article.message_id))
                }
            })
            .collect();

        let message_id_refs: Vec<&str> = message_ids.iter().map(|s| s.as_ref()).collect();

        // Acquire bandwidth tokens before downloading
        let total_batch_size: u64 = article_batch.iter().map(|a| a.size_bytes as u64).sum();
        speed_limiter.acquire(total_batch_size).await;

        // Fetch articles using pipelined API
        let responses = match conn
            .fetch_articles_pipelined(&message_id_refs, pipeline_depth)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(download_id = id, batch_size = batch_size, error = %e, "Batch fetch failed");
                for article in &article_batch {
                    if let Err(e) = batch_tx
                        .send((article.id, crate::db::article_status::FAILED))
                        .await
                    {
                        tracing::warn!(download_id = id, article_id = article.id, error = %e, "Failed to send status update to batch channel");
                    }
                }
                return Err((format!("Batch fetch failed: {}", e), batch_size));
            }
        };

        // Process each article response
        let mut batch_results = Vec::with_capacity(batch_size);

        for (article, response) in article_batch.iter().zip(responses.iter()) {
            let article_file =
                download_temp_dir.join(format!("article_{}.dat", article.segment_number));

            if let Err(e) = tokio::fs::write(&article_file, &response.data).await {
                tracing::error!(download_id = id, article_id = article.id, error = %e, "Failed to write article file");
                return Err((
                    format!("Failed to write article file: {}", e),
                    batch_size,
                ));
            }

            if let Err(e) = batch_tx
                .send((article.id, crate::db::article_status::DOWNLOADED))
                .await
            {
                tracing::warn!(download_id = id, article_id = article.id, error = %e, "Failed to send status update to batch channel");
            }

            downloaded_articles.fetch_add(1, Ordering::Relaxed);
            downloaded_bytes.fetch_add(article.size_bytes as u64, Ordering::Relaxed);

            batch_results.push((article.segment_number, article.size_bytes as u64));
        }

        Ok(batch_results)
    }

    /// Evaluate download results and finalize the download status.
    ///
    /// Marks the download as complete or failed based on the success/failure ratio,
    /// removes from active tracking, and triggers post-processing on success.
    async fn finalize_download(
        ctx: DownloadTaskContext,
        results: DownloadResults,
        total_articles: usize,
        _total_size_bytes: u64,
    ) {
        let id = ctx.id;
        let DownloadResults {
            success_count,
            failed_count,
            first_error,
        } = results;

        let total = success_count + failed_count;

        // Handle partial or total failures
        if failed_count > 0 {
            tracing::warn!(
                download_id = id,
                failed = failed_count,
                succeeded = success_count,
                total = total,
                "Download completed with some failures"
            );

            if success_count == 0 || (failed_count as f64 / total as f64) > MAX_FAILURE_RATIO {
                let error_msg = first_error.unwrap_or_else(|| "Unknown error".to_string());
                tracing::error!(
                    download_id = id,
                    failed = failed_count,
                    succeeded = success_count,
                    "Download failed - too many article failures"
                );
                ctx.mark_failed(&error_msg).await;
                ctx.remove_from_active().await;
                return;
            }
        }

        // Mark download as complete
        if let Err(e) = ctx
            .db
            .update_status(id, Status::Complete.to_i32())
            .await
        {
            tracing::error!(download_id = id, error = %e, "Failed to mark download complete");
            ctx.remove_from_active().await;
            return;
        }
        if let Err(e) = ctx.db.set_completed(id).await {
            tracing::error!(download_id = id, error = %e, "Failed to set completion time");
        }

        ctx.event_tx.send(Event::DownloadComplete { id }).ok();
        ctx.remove_from_active().await;
        ctx.spawn_post_processing();
    }

    /// Spawn a background task that periodically reports download progress.
    fn spawn_progress_reporter(
        id: DownloadId,
        total_articles: usize,
        total_size_bytes: u64,
        download_start: std::time::Instant,
        downloaded_articles: Arc<AtomicU64>,
        downloaded_bytes: Arc<AtomicU64>,
        event_tx: tokio::sync::broadcast::Sender<Event>,
        db: Arc<crate::db::Database>,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(PROGRESS_UPDATE_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let current_bytes = downloaded_bytes.load(Ordering::Relaxed);
                        let current_articles = downloaded_articles.load(Ordering::Relaxed);

                        let progress_percent = if total_size_bytes > 0 {
                            (current_bytes as f32 / total_size_bytes as f32) * 100.0
                        } else {
                            (current_articles as f32 / total_articles as f32) * 100.0
                        };

                        let elapsed_secs = download_start.elapsed().as_secs_f64();
                        let speed_bps = if elapsed_secs > 0.0 {
                            (current_bytes as f64 / elapsed_secs) as u64
                        } else {
                            0
                        };

                        if let Err(e) = db.update_progress(
                            id,
                            progress_percent,
                            speed_bps,
                            current_bytes,
                        ).await {
                            tracing::error!(download_id = id, error = %e, "Failed to update progress");
                        }

                        event_tx
                            .send(Event::Downloading {
                                id,
                                percent: progress_percent,
                                speed_bps,
                            })
                            .ok();
                    }
                    _ = cancel_token.cancelled() => {
                        break;
                    }
                }
            }
        })
    }

    /// Spawn a background task that batches article status updates for SQLite efficiency.
    fn spawn_batch_updater(
        id: DownloadId,
        db: Arc<crate::db::Database>,
        mut batch_rx: tokio::sync::mpsc::Receiver<(i64, i32)>,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut buffer = Vec::with_capacity(ARTICLE_BATCH_SIZE);
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    Some((article_id, status)) = batch_rx.recv() => {
                        buffer.push((article_id, status));

                        if buffer.len() >= ARTICLE_BATCH_SIZE {
                            if let Err(e) = db.update_articles_status_batch(&buffer).await {
                                tracing::error!(download_id = id, batch_size = buffer.len(), error = %e, "Failed to batch update article statuses");
                            }
                            buffer.clear();
                        }
                    }
                    _ = interval.tick() => {
                        if !buffer.is_empty() {
                            if let Err(e) = db.update_articles_status_batch(&buffer).await {
                                tracing::error!(download_id = id, batch_size = buffer.len(), error = %e, "Failed to batch update article statuses");
                            }
                            buffer.clear();
                        }
                    }
                    _ = cancel_token.cancelled() => {
                        if !buffer.is_empty() {
                            if let Err(e) = db.update_articles_status_batch(&buffer).await {
                                tracing::error!(download_id = id, batch_size = buffer.len(), error = %e, "Failed to batch update article statuses on cancellation");
                            }
                        }
                        break;
                    }
                }
            }

            // Final flush when task ends (channel closed)
            while let Ok((article_id, status)) = batch_rx.try_recv() {
                buffer.push((article_id, status));
            }
            if !buffer.is_empty() {
                if let Err(e) = db.update_articles_status_batch(&buffer).await {
                    tracing::error!(download_id = id, batch_size = buffer.len(), error = %e, "Failed to flush remaining article statuses");
                }
            }
        })
    }

    /// Spawn an asynchronous download task for a queued download
    ///
    /// This internal method creates a background task that handles the entire download lifecycle.
    pub fn spawn_download_task(
        &self,
        download_id: DownloadId,
    ) -> tokio::task::JoinHandle<Result<()>> {
        let db = self.db.clone();
        let event_tx = self.event_tx.clone();
        let nntp_pools = self.nntp_pools.clone();
        let config = self.config.clone();
        let downloader = self.clone();

        tokio::spawn(async move {
            // Fetch download record
            let download = match db.get_download(download_id).await? {
                Some(d) => d,
                None => {
                    return Err(Error::Database(DatabaseError::NotFound(format!(
                        "Download with ID {} not found",
                        download_id
                    ))))
                }
            };

            // Update status to Downloading and record start time
            db.update_status(download_id, Status::Downloading.to_i32())
                .await?;
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

                // Start post-processing pipeline asynchronously
                tokio::spawn(async move {
                    if let Err(e) = downloader.start_post_processing(download_id).await {
                        tracing::error!(
                            download_id,
                            error = %e,
                            "Post-processing failed"
                        );
                    }
                });

                return Ok(());
            }

            let total_articles = pending_articles.len();
            let total_size_bytes = download.size_bytes as u64;
            let downloaded_articles = Arc::new(AtomicU64::new(0));
            let downloaded_bytes = Arc::new(AtomicU64::new(0));

            // Track download start time for speed calculation
            let download_start = std::time::Instant::now();

            // Create temp directory for this download
            let download_temp_dir = config.download.temp_dir.join(format!("download_{}", download_id));
            tokio::fs::create_dir_all(&download_temp_dir)
                .await
                .map_err(|e| {
                    Error::Io(std::io::Error::new(
                        e.kind(),
                        format!("Failed to create temp directory: {}", e),
                    ))
                })?;

            // Calculate concurrency limit from server connections
            let concurrency: usize = config.servers.iter().map(|s| s.connections).sum();

            // Download articles in parallel using buffered stream
            let results: Vec<std::result::Result<(i32, u64), String>> =
                stream::iter(pending_articles)
                    .map(|article| {
                        let nntp_pools = nntp_pools.clone();
                        let db = db.clone();
                        let download_temp_dir = download_temp_dir.clone();
                        let downloaded_articles = downloaded_articles.clone();
                        let downloaded_bytes = downloaded_bytes.clone();

                        async move {
                            // Get a connection from the first NNTP pool
                            let pool = nntp_pools
                                .first()
                                .ok_or_else(|| "No NNTP pools configured".to_string())?;

                            let mut conn = pool
                                .get()
                                .await
                                .map_err(|e| format!("Failed to get NNTP connection: {}", e))?;

                            // Fetch the article from the server
                            let message_id = if article.message_id.starts_with('<') {
                                article.message_id.clone()
                            } else {
                                format!("<{}>", article.message_id)
                            };

                            let response = match conn.fetch_article_binary(&message_id).await {
                                Ok(r) => r,
                                Err(e) => {
                                    tracing::warn!(
                                        download_id = download_id,
                                        article_id = article.id,
                                        error = %e,
                                        "Article fetch failed"
                                    );
                                    let _ = db
                                        .update_article_status(
                                            article.id,
                                            crate::db::article_status::FAILED,
                                        )
                                        .await;
                                    return Err(format!("Article fetch failed: {}", e));
                                }
                            };

                            let article_file = download_temp_dir
                                .join(format!("article_{}.dat", article.segment_number));

                            if let Err(e) = tokio::fs::write(&article_file, &response.data).await {
                                tracing::warn!(
                                    download_id = download_id,
                                    article_id = article.id,
                                    error = %e,
                                    "Failed to write article file"
                                );
                                let _ = db
                                    .update_article_status(
                                        article.id,
                                        crate::db::article_status::FAILED,
                                    )
                                    .await;
                                return Err(format!("Failed to write article file: {}", e));
                            }

                            if let Err(e) = db
                                .update_article_status(
                                    article.id,
                                    crate::db::article_status::DOWNLOADED,
                                )
                                .await
                            {
                                tracing::warn!(
                                    download_id = download_id,
                                    article_id = article.id,
                                    error = %e,
                                    "Failed to update article status"
                                );
                                return Err(format!("Failed to update article status: {}", e));
                            }

                            downloaded_articles.fetch_add(1, Ordering::Relaxed);
                            downloaded_bytes
                                .fetch_add(article.size_bytes as u64, Ordering::Relaxed);

                            Ok::<(i32, u64), String>((
                                article.segment_number,
                                article.size_bytes as u64,
                            ))
                        }
                    })
                    .buffer_unordered(concurrency)
                    .collect()
                    .await;

            // Process results and check for failures
            let mut successes = 0;
            let mut failures = 0;
            let mut first_error: Option<String> = None;

            for result in results {
                match result {
                    Ok(_) => successes += 1,
                    Err(e) => {
                        failures += 1;
                        if first_error.is_none() {
                            first_error = Some(e);
                        }
                    }
                }
            }

            if failures > 0 {
                tracing::warn!(
                    download_id = download_id,
                    failed = failures,
                    succeeded = successes,
                    total = total_articles,
                    "Download completed with some failures"
                );

                if successes == 0
                    || (failures as f64 / total_articles as f64) > MAX_FAILURE_RATIO
                {
                    let error_msg = first_error.unwrap_or_else(|| "Unknown error".to_string());
                    tracing::error!(
                        download_id = download_id,
                        failed = failures,
                        succeeded = successes,
                        "Download failed - too many article failures"
                    );

                    db.update_status(download_id, Status::Failed.to_i32())
                        .await?;
                    db.set_error(download_id, &error_msg).await?;

                    event_tx
                        .send(Event::DownloadFailed {
                            id: download_id,
                            error: error_msg.clone(),
                        })
                        .ok();

                    return Err(Error::Nntp(format!(
                        "Download failed: {} of {} articles failed. First error: {}",
                        failures, total_articles, error_msg
                    )));
                }
            }

            // Emit final progress event
            let final_bytes = downloaded_bytes.load(Ordering::Relaxed);
            let final_articles = downloaded_articles.load(Ordering::Relaxed);
            let final_percent = if total_size_bytes > 0 {
                (final_bytes as f32 / total_size_bytes as f32) * 100.0
            } else {
                (final_articles as f32 / total_articles as f32) * 100.0
            };
            let elapsed_secs = download_start.elapsed().as_secs_f64();
            let final_speed_bps = if elapsed_secs > 0.0 {
                (final_bytes as f64 / elapsed_secs) as u64
            } else {
                0
            };

            db.update_progress(download_id, final_percent, final_speed_bps, final_bytes)
                .await?;

            event_tx
                .send(Event::Downloading {
                    id: download_id,
                    percent: final_percent,
                    speed_bps: final_speed_bps,
                })
                .ok();

            // All articles downloaded successfully
            db.update_status(download_id, Status::Complete.to_i32())
                .await?;
            db.set_completed(download_id).await?;

            event_tx
                .send(Event::DownloadComplete { id: download_id })
                .ok();

            // Start post-processing pipeline asynchronously
            tokio::spawn(async move {
                if let Err(e) = downloader.start_post_processing(download_id).await {
                    tracing::error!(
                        download_id,
                        error = %e,
                        "Post-processing failed"
                    );
                }
            });

            Ok(())
        })
    }

    /// Start the folder watcher background task
    pub fn start_folder_watcher(&self) -> Result<tokio::task::JoinHandle<()>> {
        let watch_folders = self.config.watch_folders.clone();

        if watch_folders.is_empty() {
            tracing::info!("No watch folders configured, skipping folder watcher");
            return Ok(tokio::spawn(async {}));
        }

        let mut watcher =
            folder_watcher::FolderWatcher::new(std::sync::Arc::new(self.clone()), watch_folders)?;

        watcher.start()?;

        let handle = tokio::spawn(async move {
            watcher.run().await;
        });

        tracing::info!("Folder watcher background task started");

        Ok(handle)
    }

    /// Start RSS feed scheduler for automatic feed checking
    pub fn start_rss_scheduler(&self) -> tokio::task::JoinHandle<()> {
        let rss_feeds = self.config.rss_feeds.clone();

        if rss_feeds.is_empty() {
            tracing::info!("No RSS feeds configured, skipping RSS scheduler");
            return tokio::spawn(async {});
        }

        let rss_manager = match rss_manager::RssManager::new(
            self.db.clone(),
            std::sync::Arc::new(self.clone()),
            rss_feeds.clone(),
        ) {
            Ok(manager) => std::sync::Arc::new(manager),
            Err(e) => {
                tracing::error!(error = %e, "Failed to create RSS manager");
                return tokio::spawn(async {});
            }
        };

        let scheduler =
            rss_scheduler::RssScheduler::new(std::sync::Arc::new(self.clone()), rss_manager);

        let handle = tokio::spawn(async move {
            scheduler.run().await;
        });

        tracing::info!("RSS scheduler background task started");

        handle
    }

    /// Start the scheduler task that checks schedule rules every minute
    pub fn start_scheduler(&self) -> tokio::task::JoinHandle<()> {
        let schedule_rules = self.config.schedule_rules.clone();

        if schedule_rules.is_empty() {
            tracing::info!("No schedule rules configured, skipping scheduler task");
            return tokio::spawn(async {});
        }

        // Convert config::ScheduleRule to scheduler::ScheduleRule
        let scheduler_rules: Vec<scheduler::ScheduleRule> = schedule_rules
            .into_iter()
            .enumerate()
            .filter_map(|(idx, rule)| {
                let start_time =
                    chrono::NaiveTime::parse_from_str(&rule.start_time, "%H:%M").ok()?;
                let end_time = chrono::NaiveTime::parse_from_str(&rule.end_time, "%H:%M").ok()?;

                let days: Vec<scheduler::Weekday> = rule
                    .days
                    .into_iter()
                    .map(|d| match d {
                        config::Weekday::Monday => scheduler::Weekday::Monday,
                        config::Weekday::Tuesday => scheduler::Weekday::Tuesday,
                        config::Weekday::Wednesday => scheduler::Weekday::Wednesday,
                        config::Weekday::Thursday => scheduler::Weekday::Thursday,
                        config::Weekday::Friday => scheduler::Weekday::Friday,
                        config::Weekday::Saturday => scheduler::Weekday::Saturday,
                        config::Weekday::Sunday => scheduler::Weekday::Sunday,
                    })
                    .collect();

                let action = match rule.action {
                    config::ScheduleAction::SpeedLimit { limit_bps } => {
                        scheduler::ScheduleAction::SpeedLimit(limit_bps)
                    }
                    config::ScheduleAction::Unlimited => scheduler::ScheduleAction::Unlimited,
                    config::ScheduleAction::Pause => scheduler::ScheduleAction::Pause,
                };

                Some(scheduler::ScheduleRule {
                    id: idx as i64,
                    name: rule.name,
                    days,
                    start_time,
                    end_time,
                    action,
                    enabled: rule.enabled,
                })
            })
            .collect();

        let scheduler = std::sync::Arc::new(scheduler::Scheduler::new(scheduler_rules));

        let scheduler_task =
            scheduler_task::SchedulerTask::new(std::sync::Arc::new(self.clone()), scheduler);

        let handle = tokio::spawn(async move {
            scheduler_task.run().await;
        });

        tracing::info!("Scheduler task started, checking rules every minute");

        handle
    }
}
