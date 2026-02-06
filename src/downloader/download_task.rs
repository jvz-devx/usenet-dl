//! Download task execution — core download lifecycle and article fetching.

use crate::types::{DownloadId, Event, Status};
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::UsenetDownloader;

/// Maximum article failure ratio before considering a download failed (50%)
const MAX_FAILURE_RATIO: f64 = 0.5;

/// Result type for a collection of downloaded article batches.
/// Each batch either succeeds with a list of (segment_number, size_bytes) pairs,
/// or fails with an error message and the number of articles in the batch.
type BatchResultVec = Vec<std::result::Result<Vec<(i32, u64)>, (String, usize)>>;

/// Shared context for a single download task, reducing parameter passing between helpers.
pub(crate) struct DownloadTaskContext {
    pub(crate) id: DownloadId,
    pub(crate) db: Arc<crate::db::Database>,
    pub(crate) event_tx: tokio::sync::broadcast::Sender<Event>,
    pub(crate) nntp_pools: Arc<Vec<nntp_rs::NntpPool>>,
    pub(crate) config: Arc<crate::config::Config>,
    pub(crate) active_downloads: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<DownloadId, tokio_util::sync::CancellationToken>,
        >,
    >,
    pub(crate) speed_limiter: crate::speed_limiter::SpeedLimiter,
    pub(crate) cancel_token: tokio_util::sync::CancellationToken,
    pub(crate) downloader: UsenetDownloader,
}

impl DownloadTaskContext {
    /// Remove this download from the active downloads map.
    async fn remove_from_active(&self) {
        let mut active = self.active_downloads.lock().await;
        active.remove(&self.id);
    }

    /// Mark the download as failed with an error message and emit the failure event.
    async fn mark_failed(&self, error: &str) {
        let _ = self
            .db
            .update_status(self.id, Status::Failed.to_i32())
            .await;
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
                    download_id = self.id.0,
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

/// Core download task — orchestrates the full lifecycle of a single download.
///
/// Phases:
/// 1. Fetch and validate the download record
/// 2. Transition to Downloading state
/// 3. Download all pending articles in parallel batches
/// 4. Evaluate results and finalize status
/// 5. Trigger post-processing
pub(crate) async fn run_download_task(ctx: DownloadTaskContext) {
    let id = ctx.id;

    // Phase 1: Fetch download record and pending articles
    let (download, pending_articles) = match fetch_download_record(&ctx).await {
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
    let download_temp_dir = ctx
        .config
        .download
        .temp_dir
        .join(format!("download_{}", id.0));
    if let Err(e) = tokio::fs::create_dir_all(&download_temp_dir).await {
        let msg = format!("Failed to create temp directory: {}", e);
        tracing::error!(download_id = id.0, error = %e, "Failed to create temp directory");
        ctx.mark_failed(&msg).await;
        ctx.remove_from_active().await;
        return;
    }

    // Phase 4: Download articles
    let _total_articles = pending_articles.len();
    let total_size_bytes = download.size_bytes as u64;
    let results =
        download_articles(&ctx, pending_articles, total_size_bytes, &download_temp_dir).await;

    // Phase 5: Finalize based on results
    finalize_download(ctx, results, total_size_bytes).await;
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
            tracing::warn!(download_id = id.0, "Download not found in database");
            ctx.remove_from_active().await;
            return None;
        }
        Err(e) => {
            tracing::error!(download_id = id.0, error = %e, "Failed to fetch download");
            ctx.remove_from_active().await;
            return None;
        }
    };

    // Update status to Downloading and record start time
    if let Err(e) = ctx.db.update_status(id, Status::Downloading.to_i32()).await {
        tracing::error!(download_id = id.0, error = %e, "Failed to update status");
        ctx.remove_from_active().await;
        return None;
    }
    if let Err(e) = ctx.db.set_started(id).await {
        tracing::error!(download_id = id.0, error = %e, "Failed to set start time");
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
            tracing::error!(download_id = id.0, error = %e, "Failed to get pending articles");
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

    // Set up background tasks for progress reporting and DB updates
    let (progress_task, batch_tx, batch_task) = spawn_background_tasks(
        id,
        total_articles,
        total_size_bytes,
        download_start,
        &downloaded_articles,
        &downloaded_bytes,
        ctx,
    );

    // Calculate concurrency and split articles into batches
    let (concurrency, pipeline_depth, article_batches) =
        prepare_batches(&ctx.config, pending_articles);

    // Download all batches in parallel
    let results = download_all_batches(DownloadAllBatchesParams {
        id,
        article_batches,
        ctx,
        batch_tx: &batch_tx,
        downloaded_bytes: &downloaded_bytes,
        downloaded_articles: &downloaded_articles,
        download_temp_dir,
        concurrency,
        pipeline_depth,
    })
    .await;

    // Clean up background tasks
    cleanup_background_tasks(id, progress_task, batch_tx, batch_task).await;

    // Aggregate and return results
    aggregate_results(results)
}

/// Spawn progress reporter and database batch updater background tasks.
fn spawn_background_tasks(
    id: DownloadId,
    total_articles: usize,
    total_size_bytes: u64,
    download_start: std::time::Instant,
    downloaded_articles: &Arc<AtomicU64>,
    downloaded_bytes: &Arc<AtomicU64>,
    ctx: &DownloadTaskContext,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::Sender<(i64, i32)>,
    tokio::task::JoinHandle<()>,
) {
    let progress_task = super::background_tasks::spawn_progress_reporter(
        super::background_tasks::ProgressReporterParams {
            id,
            total_articles,
            total_size_bytes,
            download_start,
            downloaded_articles: Arc::clone(downloaded_articles),
            downloaded_bytes: Arc::clone(downloaded_bytes),
            event_tx: ctx.event_tx.clone(),
            db: Arc::clone(&ctx.db),
            cancel_token: ctx.cancel_token.child_token(),
        },
    );

    let (batch_tx, batch_rx) =
        tokio::sync::mpsc::channel::<(i64, i32)>(super::background_tasks::ARTICLE_CHANNEL_BUFFER);
    let batch_task = super::background_tasks::spawn_batch_updater(
        id,
        Arc::clone(&ctx.db),
        batch_rx,
        ctx.cancel_token.child_token(),
    );

    (progress_task, batch_tx, batch_task)
}

/// Calculate concurrency settings and split articles into pipeline-sized batches.
fn prepare_batches(
    config: &crate::config::Config,
    pending_articles: Vec<crate::db::Article>,
) -> (usize, usize, Vec<Vec<crate::db::Article>>) {
    let concurrency: usize = config.servers.iter().map(|s| s.connections).sum();
    let pipeline_depth = config
        .servers
        .first()
        .map(|s| s.pipeline_depth.max(1))
        .unwrap_or(10);

    let article_batches: Vec<Vec<_>> = pending_articles
        .chunks(pipeline_depth)
        .map(|chunk| chunk.to_vec())
        .collect();

    (concurrency, pipeline_depth, article_batches)
}

/// Parameters for downloading all article batches
struct DownloadAllBatchesParams<'a> {
    id: DownloadId,
    article_batches: Vec<Vec<crate::db::Article>>,
    ctx: &'a DownloadTaskContext,
    batch_tx: &'a tokio::sync::mpsc::Sender<(i64, i32)>,
    downloaded_bytes: &'a Arc<AtomicU64>,
    downloaded_articles: &'a Arc<AtomicU64>,
    download_temp_dir: &'a std::path::Path,
    concurrency: usize,
    pipeline_depth: usize,
}

/// Download all article batches in parallel using a buffered stream.
async fn download_all_batches(params: DownloadAllBatchesParams<'_>) -> BatchResultVec {
    let DownloadAllBatchesParams {
        id,
        article_batches,
        ctx,
        batch_tx,
        downloaded_bytes,
        downloaded_articles,
        download_temp_dir,
        concurrency,
        pipeline_depth,
    } = params;
    stream::iter(article_batches)
        .map(|article_batch| {
            let pool = Arc::clone(&ctx.nntp_pools);
            let batch_tx = batch_tx.clone();
            let speed_limiter = ctx.speed_limiter.clone();
            let cancel_token = ctx.cancel_token.clone();
            let download_temp_dir = download_temp_dir.to_path_buf();
            let downloaded_bytes = Arc::clone(downloaded_bytes);
            let downloaded_articles = Arc::clone(downloaded_articles);

            async move {
                fetch_article_batch(FetchArticleBatchParams {
                    id,
                    article_batch,
                    nntp_pools: pool,
                    batch_tx,
                    speed_limiter,
                    cancel_token,
                    download_temp_dir,
                    downloaded_bytes,
                    downloaded_articles,
                    pipeline_depth,
                })
                .await
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await
}

/// Stop progress task and flush final database updates.
async fn cleanup_background_tasks(
    id: DownloadId,
    progress_task: tokio::task::JoinHandle<()>,
    batch_tx: tokio::sync::mpsc::Sender<(i64, i32)>,
    batch_task: tokio::task::JoinHandle<()>,
) {
    progress_task.abort();
    drop(batch_tx);
    if let Err(e) = batch_task.await {
        tracing::error!(download_id = id.0, error = %e, "Batch update task panicked");
    }
}

/// Aggregate batch results into success/failure counts and first error.
fn aggregate_results(results: BatchResultVec) -> DownloadResults {
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

/// Parameters for fetching a batch of articles
struct FetchArticleBatchParams {
    /// Download ID
    id: DownloadId,
    /// Articles to fetch in this batch
    article_batch: Vec<crate::db::Article>,
    /// NNTP connection pools
    nntp_pools: Arc<Vec<nntp_rs::NntpPool>>,
    /// Channel for sending article status updates
    batch_tx: tokio::sync::mpsc::Sender<(i64, i32)>,
    /// Speed limiter
    speed_limiter: crate::speed_limiter::SpeedLimiter,
    /// Cancellation token
    cancel_token: tokio_util::sync::CancellationToken,
    /// Temporary directory for download
    download_temp_dir: std::path::PathBuf,
    /// Atomic counter for downloaded bytes
    downloaded_bytes: Arc<AtomicU64>,
    /// Atomic counter for downloaded articles
    downloaded_articles: Arc<AtomicU64>,
    /// Pipeline depth for NNTP commands
    pipeline_depth: usize,
}

/// Fetch a single batch of articles via pipelined NNTP commands.
async fn fetch_article_batch(
    params: FetchArticleBatchParams,
) -> std::result::Result<Vec<(i32, u64)>, (String, usize)> {
    let FetchArticleBatchParams {
        id,
        article_batch,
        nntp_pools,
        batch_tx,
        speed_limiter,
        cancel_token,
        download_temp_dir,
        downloaded_bytes,
        downloaded_articles,
        pipeline_depth,
    } = params;
    let batch_size = article_batch.len();

    // Check if download was cancelled
    if cancel_token.is_cancelled() {
        return Err(("Download cancelled".to_string(), batch_size));
    }

    // Try each NNTP pool in order (primary first, then backup/fill servers)
    if nntp_pools.is_empty() {
        tracing::error!(download_id = id.0, "No NNTP pools configured");
        return Err(("No NNTP pools configured".to_string(), batch_size));
    }

    let mut conn = None;
    let mut last_error = String::new();
    for (pool_idx, pool) in nntp_pools.iter().enumerate() {
        match pool.get().await {
            Ok(c) => {
                conn = Some(c);
                break;
            }
            Err(e) => {
                tracing::warn!(
                    download_id = id.0,
                    pool_index = pool_idx,
                    error = %e,
                    "Failed to get connection from NNTP pool, trying next server"
                );
                last_error = format!("Failed to get NNTP connection: {}", e);
            }
        }
    }

    let mut conn = match conn {
        Some(c) => c,
        None => {
            tracing::error!(download_id = id.0, "All NNTP servers failed");
            return Err((last_error, batch_size));
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
            tracing::error!(download_id = id.0, batch_size = batch_size, error = %e, "Batch fetch failed");
            for article in &article_batch {
                if let Err(e) = batch_tx
                    .send((article.id, crate::db::article_status::FAILED))
                    .await
                {
                    tracing::warn!(download_id = id.0, article_id = article.id, error = %e, "Failed to send status update to batch channel");
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
            tracing::error!(download_id = id.0, article_id = article.id, error = %e, "Failed to write article file");
            return Err((format!("Failed to write article file: {}", e), batch_size));
        }

        if let Err(e) = batch_tx
            .send((article.id, crate::db::article_status::DOWNLOADED))
            .await
        {
            tracing::warn!(download_id = id.0, article_id = article.id, error = %e, "Failed to send status update to batch channel");
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
            download_id = id.0,
            failed = failed_count,
            succeeded = success_count,
            total = total,
            "Download completed with some failures"
        );

        if success_count == 0 || (failed_count as f64 / total as f64) > MAX_FAILURE_RATIO {
            let error_msg = first_error.unwrap_or_else(|| "Unknown error".to_string());
            tracing::error!(
                download_id = id.0,
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
    if let Err(e) = ctx.db.update_status(id, Status::Complete.to_i32()).await {
        tracing::error!(download_id = id.0, error = %e, "Failed to mark download complete");
        ctx.remove_from_active().await;
        return;
    }
    if let Err(e) = ctx.db.set_completed(id).await {
        tracing::error!(download_id = id.0, error = %e, "Failed to set completion time");
    }

    ctx.event_tx.send(Event::DownloadComplete { id }).ok();
    ctx.remove_from_active().await;
    ctx.spawn_post_processing();
}
