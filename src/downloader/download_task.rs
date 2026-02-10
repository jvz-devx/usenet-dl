//! Download task execution — core download lifecycle and article fetching.

use crate::types::{DownloadId, Event, Status};
use futures::stream::{self, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::UsenetDownloader;

/// Manages output file handles for DirectWrite — one file per NZB file entry.
///
/// Uses `std::os::unix::fs::FileExt::write_all_at()` which takes `&self` (not `&mut self`),
/// enabling lock-free concurrent writes from different batches to the same file.
/// Each segment writes to non-overlapping byte ranges via yEnc part offsets.
pub(crate) struct OutputFiles {
    /// file_index → (File handle, filename)
    files: HashMap<i32, (std::fs::File, String)>,
}

impl OutputFiles {
    /// Create OutputFiles by pre-creating empty files for each download file entry.
    fn create(
        download_files: &[crate::db::DownloadFile],
        temp_dir: &std::path::Path,
    ) -> std::io::Result<Self> {
        let mut files = HashMap::with_capacity(download_files.len());
        for df in download_files {
            let path = temp_dir.join(&df.filename);
            let file = std::fs::File::create(&path)?;
            files.insert(df.file_index, (file, df.filename.clone()));
        }
        Ok(Self { files })
    }
}

/// Check whether an NNTP error indicates a missing/expired article (vs connection/protocol failure).
fn is_missing_article_error(err: &nntp_rs::NntpError) -> bool {
    match err {
        nntp_rs::NntpError::NoSuchArticle(_) => true,
        nntp_rs::NntpError::Protocol { code, .. } if *code == 430 => true,
        other => {
            let msg = other.to_string();
            msg.contains("No such article") || msg.contains("no such article")
        }
    }
}

/// Abstraction over NNTP article fetching, enabling testability.
#[async_trait::async_trait]
pub(crate) trait ArticleProvider: Send + Sync {
    async fn fetch_articles(
        &self,
        message_ids: &[&str],
        pipeline_depth: usize,
    ) -> nntp_rs::Result<Vec<nntp_rs::NntpBinaryResponse>>;
}

/// Production [`ArticleProvider`] that iterates NNTP connection pools.
pub(crate) struct NntpArticleProvider {
    pools: Arc<Vec<nntp_rs::NntpPool>>,
}

impl NntpArticleProvider {
    pub(crate) fn new(pools: Arc<Vec<nntp_rs::NntpPool>>) -> Self {
        Self { pools }
    }
}

#[async_trait::async_trait]
impl ArticleProvider for NntpArticleProvider {
    async fn fetch_articles(
        &self,
        message_ids: &[&str],
        pipeline_depth: usize,
    ) -> nntp_rs::Result<Vec<nntp_rs::NntpBinaryResponse>> {
        if self.pools.is_empty() {
            return Err(nntp_rs::NntpError::Other(
                "No NNTP pools configured".to_string(),
            ));
        }

        let mut last_error = None;
        for (pool_idx, pool) in self.pools.iter().enumerate() {
            match pool.get().await {
                Ok(mut conn) => {
                    return conn
                        .fetch_articles_pipelined(message_ids, pipeline_depth)
                        .await;
                }
                Err(e) => {
                    tracing::warn!(
                        pool_index = pool_idx,
                        error = %e,
                        "Failed to get connection from NNTP pool, trying next server"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| nntp_rs::NntpError::Other("All NNTP servers failed".to_string())))
    }
}

/// Result type for a collection of downloaded article batches.
/// Each batch either succeeds with a list of (segment_number, size_bytes) pairs,
/// or fails with an error message and the number of articles in the batch.
type BatchResultVec = Vec<std::result::Result<Vec<(i32, u64)>, (String, usize)>>;

/// Shared context for a single download task, reducing parameter passing between helpers.
pub(crate) struct DownloadTaskContext {
    pub(crate) id: DownloadId,
    pub(crate) db: Arc<crate::db::Database>,
    pub(crate) event_tx: tokio::sync::broadcast::Sender<Event>,
    pub(crate) article_provider: Arc<dyn ArticleProvider>,
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
        self.mark_failed_with_stats(error, None, None, None).await;
    }

    /// Mark the download as failed with an error message and optional article stats.
    async fn mark_failed_with_stats(
        &self,
        error: &str,
        articles_succeeded: Option<u64>,
        articles_failed: Option<u64>,
        articles_total: Option<u64>,
    ) {
        let _ = self
            .db
            .update_status(self.id, Status::Failed.to_i32())
            .await;
        let _ = self.db.set_error(self.id, error).await;
        self.event_tx
            .send(Event::DownloadFailed {
                id: self.id,
                error: error.to_string(),
                articles_succeeded,
                articles_failed,
                articles_total,
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
    /// Total articles in the download (for stats)
    total_articles: usize,
    /// Count of individually-failed articles (tracked via atomic, separate from batch failures)
    individually_failed: u64,
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
        ctx.event_tx
            .send(Event::DownloadComplete {
                id,
                articles_failed: None,
                articles_total: None,
            })
            .ok();
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

    // Phase 3b: Create output files for DirectWrite
    let download_files = match ctx.db.get_download_files(id).await {
        Ok(files) => files,
        Err(e) => {
            let msg = format!("Failed to get download files: {}", e);
            tracing::error!(download_id = id.0, error = %e, "Failed to get download files");
            ctx.mark_failed(&msg).await;
            ctx.remove_from_active().await;
            return;
        }
    };

    let output_files = if download_files.is_empty() {
        // Legacy downloads without download_files rows — no DirectWrite
        Arc::new(OutputFiles {
            files: HashMap::new(),
        })
    } else {
        match OutputFiles::create(&download_files, &download_temp_dir) {
            Ok(of) => Arc::new(of),
            Err(e) => {
                let msg = format!("Failed to create output files: {}", e);
                tracing::error!(download_id = id.0, error = %e, "Failed to create output files");
                ctx.mark_failed(&msg).await;
                ctx.remove_from_active().await;
                return;
            }
        }
    };

    // Phase 4: Download articles
    let _total_articles = pending_articles.len();
    let total_size_bytes = download.size_bytes as u64;
    let results = download_articles(
        &ctx,
        pending_articles,
        total_size_bytes,
        &download_temp_dir,
        &output_files,
    )
    .await;

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
            failed_articles: None,
            total_articles: None,
            health_percent: None,
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
    output_files: &Arc<OutputFiles>,
) -> DownloadResults {
    let id = ctx.id;
    let total_articles = pending_articles.len();
    let counters = DownloadCounters {
        downloaded_articles: Arc::new(AtomicU64::new(0)),
        downloaded_bytes: Arc::new(AtomicU64::new(0)),
        failed_articles: Arc::new(AtomicU64::new(0)),
    };
    let download_start = std::time::Instant::now();

    // Set up background tasks for progress reporting and DB updates
    let (progress_task, batch_tx, batch_task) = spawn_background_tasks(
        id,
        total_articles,
        total_size_bytes,
        download_start,
        &counters,
        ctx,
    );

    // Spawn fast-fail watcher: if most articles in an early sample are missing, cancel early
    let fast_fail_task = spawn_fast_fail_watcher(
        &counters.downloaded_articles,
        &counters.failed_articles,
        ctx.config.download.fast_fail_threshold,
        ctx.config.download.fast_fail_sample_size,
        ctx.cancel_token.clone(),
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
        downloaded_bytes: &counters.downloaded_bytes,
        downloaded_articles: &counters.downloaded_articles,
        failed_articles: &counters.failed_articles,
        download_temp_dir,
        output_files,
        concurrency,
        pipeline_depth,
    })
    .await;

    // Clean up background tasks
    fast_fail_task.abort();
    cleanup_background_tasks(id, progress_task, batch_tx, batch_task).await;

    // Aggregate and return results
    let mut agg = aggregate_results(results);
    agg.total_articles = total_articles;
    agg.individually_failed = counters.failed_articles.load(Ordering::Relaxed);
    agg
}

/// Atomic counters shared across the download pipeline.
struct DownloadCounters {
    downloaded_articles: Arc<AtomicU64>,
    downloaded_bytes: Arc<AtomicU64>,
    failed_articles: Arc<AtomicU64>,
}

/// Spawn progress reporter and database batch updater background tasks.
fn spawn_background_tasks(
    id: DownloadId,
    total_articles: usize,
    total_size_bytes: u64,
    download_start: std::time::Instant,
    counters: &DownloadCounters,
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
            downloaded_articles: Arc::clone(&counters.downloaded_articles),
            downloaded_bytes: Arc::clone(&counters.downloaded_bytes),
            failed_articles: Arc::clone(&counters.failed_articles),
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

/// Spawn a fast-fail watcher that cancels the download if too many early articles are missing.
///
/// Polls the atomic counters every 200ms. After `sample_size` articles have been attempted
/// (downloaded + failed), if the failure ratio >= `threshold`, cancels via the token.
fn spawn_fast_fail_watcher(
    downloaded_articles: &Arc<AtomicU64>,
    failed_articles: &Arc<AtomicU64>,
    threshold: f64,
    sample_size: usize,
    cancel_token: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let downloaded = Arc::clone(downloaded_articles);
    let failed = Arc::clone(failed_articles);
    let child_token = cancel_token.child_token();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(200));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let d = downloaded.load(Ordering::Relaxed);
                    let f = failed.load(Ordering::Relaxed);
                    let attempted = d + f;
                    if attempted >= sample_size as u64 && attempted > 0 {
                        let fail_ratio = f as f64 / attempted as f64;
                        if fail_ratio >= threshold {
                            tracing::warn!(
                                failed = f,
                                attempted = attempted,
                                ratio = %format!("{:.1}%", fail_ratio * 100.0),
                                "Fast-fail: too many articles missing, cancelling download"
                            );
                            cancel_token.cancel();
                            return;
                        }
                        // Once we've passed the sample without triggering, stop watching
                        return;
                    }
                }
                _ = child_token.cancelled() => {
                    return;
                }
            }
        }
    })
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
    failed_articles: &'a Arc<AtomicU64>,
    download_temp_dir: &'a std::path::Path,
    output_files: &'a Arc<OutputFiles>,
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
        failed_articles,
        download_temp_dir,
        output_files,
        concurrency,
        pipeline_depth,
    } = params;
    stream::iter(article_batches)
        .map(|article_batch| {
            let article_provider = Arc::clone(&ctx.article_provider);
            let batch_tx = batch_tx.clone();
            let speed_limiter = ctx.speed_limiter.clone();
            let cancel_token = ctx.cancel_token.clone();
            let download_temp_dir = download_temp_dir.to_path_buf();
            let downloaded_bytes = Arc::clone(downloaded_bytes);
            let downloaded_articles = Arc::clone(downloaded_articles);
            let failed_articles = Arc::clone(failed_articles);
            let output_files = Arc::clone(output_files);

            async move {
                fetch_article_batch(FetchArticleBatchParams {
                    id,
                    article_batch,
                    article_provider,
                    batch_tx,
                    speed_limiter,
                    cancel_token,
                    download_temp_dir,
                    downloaded_bytes,
                    downloaded_articles,
                    failed_articles,
                    output_files,
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
        total_articles: 0,      // Set by caller after aggregation
        individually_failed: 0, // Set by caller from atomic counter
    }
}

/// Parameters for fetching a batch of articles
struct FetchArticleBatchParams {
    /// Download ID
    id: DownloadId,
    /// Articles to fetch in this batch
    article_batch: Vec<crate::db::Article>,
    /// Article provider for fetching articles from NNTP servers
    article_provider: Arc<dyn ArticleProvider>,
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
    /// Atomic counter for individually-failed articles (missing/expired)
    failed_articles: Arc<AtomicU64>,
    /// Output file handles for DirectWrite
    output_files: Arc<OutputFiles>,
    /// Pipeline depth for NNTP commands
    pipeline_depth: usize,
}

/// Fetch a single batch of articles via pipelined NNTP commands.
///
/// On missing-article errors, falls back to per-article retry so that available
/// articles in the batch are still downloaded and only truly missing ones are marked failed.
async fn fetch_article_batch(
    params: FetchArticleBatchParams,
) -> std::result::Result<Vec<(i32, u64)>, (String, usize)> {
    let FetchArticleBatchParams {
        id,
        article_batch,
        article_provider,
        batch_tx,
        speed_limiter,
        cancel_token,
        download_temp_dir,
        downloaded_bytes,
        downloaded_articles,
        failed_articles,
        output_files,
        pipeline_depth,
    } = params;
    let batch_size = article_batch.len();

    // Check if download was cancelled
    if cancel_token.is_cancelled() {
        return Err(("Download cancelled".to_string(), batch_size));
    }

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

    // Fetch articles via the article provider
    let responses = match article_provider
        .fetch_articles(&message_id_refs, pipeline_depth)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            // If the error indicates a missing article, retry each article individually
            // so we can salvage the ones that exist
            if is_missing_article_error(&e) {
                tracing::debug!(
                    download_id = id.0,
                    batch_size = batch_size,
                    error = %e,
                    "Batch failed with missing article, retrying individually"
                );
                return retry_articles_individually(RetryArticlesParams {
                    id,
                    article_batch,
                    article_provider,
                    batch_tx,
                    cancel_token,
                    download_temp_dir,
                    downloaded_bytes,
                    downloaded_articles,
                    failed_articles,
                    output_files,
                })
                .await;
            }

            // Non-article errors (connection, timeout) fail the whole batch
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

    // Process each article response — decode yEnc and DirectWrite to output files
    let mut batch_results = Vec::with_capacity(batch_size);

    for (article, response) in article_batch.iter().zip(responses.iter()) {
        match decode_and_write(article, &response.data, &output_files, &download_temp_dir) {
            Ok(decoded_size) => {
                if let Err(e) = batch_tx
                    .send((article.id, crate::db::article_status::DOWNLOADED))
                    .await
                {
                    tracing::warn!(download_id = id.0, article_id = article.id, error = %e, "Failed to send status update to batch channel");
                }

                downloaded_articles.fetch_add(1, Ordering::Relaxed);
                downloaded_bytes.fetch_add(decoded_size, Ordering::Relaxed);

                batch_results.push((article.segment_number, decoded_size));
            }
            Err(e) => {
                tracing::error!(download_id = id.0, article_id = article.id, error = %e, "Failed to decode/write article");
                return Err((format!("Failed to decode/write article: {}", e), batch_size));
            }
        }
    }

    Ok(batch_results)
}

/// Decode a yEnc-encoded article and write the decoded data to the correct output file.
///
/// If `output_files` has a mapping for the article's `file_index`, uses DirectWrite
/// (positional write via `write_all_at`). Otherwise falls back to writing the raw data
/// as `article_{segment}.dat` (for legacy downloads without file metadata).
///
/// Returns the number of decoded bytes written.
fn decode_and_write(
    article: &crate::db::Article,
    data: &[u8],
    output_files: &OutputFiles,
    download_temp_dir: &std::path::Path,
) -> std::result::Result<u64, String> {
    use std::os::unix::fs::FileExt;

    // Try yEnc decode
    match nntp_rs::yenc_decode(data) {
        Ok(decoded) => {
            let decoded_size = decoded.data.len() as u64;

            if let Some((file_handle, _filename)) = output_files.files.get(&article.file_index) {
                // Calculate byte offset (yEnc begin is 1-based)
                let offset = decoded
                    .part
                    .as_ref()
                    .map(|p| p.begin - 1) // multi-part: write at byte offset
                    .unwrap_or(0); // single-part: write at start

                // Pre-allocate file to full size on first segment write (creates sparse file)
                if decoded.header.size > 0 {
                    let current_len = file_handle.metadata().map(|m| m.len()).unwrap_or(0);
                    if current_len == 0 {
                        file_handle
                            .set_len(decoded.header.size)
                            .map_err(|e| format!("Failed to pre-allocate file: {}", e))?;
                    }
                }

                // Write decoded data at correct offset (lock-free via pwrite)
                file_handle
                    .write_all_at(&decoded.data, offset)
                    .map_err(|e| format!("Failed to write at offset {}: {}", offset, e))?;
            } else {
                // Fallback: no output file mapping — write raw decoded data as article file
                let article_file =
                    download_temp_dir.join(format!("article_{}.dat", article.segment_number));
                std::fs::write(&article_file, &decoded.data)
                    .map_err(|e| format!("Failed to write article file: {}", e))?;
            }

            Ok(decoded_size)
        }
        Err(_) => {
            // yEnc decode failed — write raw data as fallback
            let article_file =
                download_temp_dir.join(format!("article_{}.dat", article.segment_number));
            let raw_size = data.len() as u64;
            std::fs::write(&article_file, data)
                .map_err(|e| format!("Failed to write raw article file: {}", e))?;
            Ok(raw_size)
        }
    }
}

/// Parameters for retrying articles individually after a batch failure.
struct RetryArticlesParams {
    id: DownloadId,
    article_batch: Vec<crate::db::Article>,
    article_provider: Arc<dyn ArticleProvider>,
    batch_tx: tokio::sync::mpsc::Sender<(i64, i32)>,
    cancel_token: tokio_util::sync::CancellationToken,
    download_temp_dir: std::path::PathBuf,
    downloaded_bytes: Arc<AtomicU64>,
    downloaded_articles: Arc<AtomicU64>,
    failed_articles: Arc<AtomicU64>,
    output_files: Arc<OutputFiles>,
}

/// Retry each article in a failed batch individually (pipeline_depth=1).
///
/// Articles that succeed are written to disk and marked DOWNLOADED.
/// Articles that fail are marked FAILED and counted in the `failed_articles` atomic.
/// Returns Ok with successful results if any articles succeeded, Err if ALL failed.
async fn retry_articles_individually(
    params: RetryArticlesParams,
) -> std::result::Result<Vec<(i32, u64)>, (String, usize)> {
    let RetryArticlesParams {
        id,
        article_batch,
        article_provider,
        batch_tx,
        cancel_token,
        download_temp_dir,
        downloaded_bytes,
        downloaded_articles,
        failed_articles,
        output_files,
    } = params;
    let batch_size = article_batch.len();
    let mut successful_results = Vec::new();
    let mut first_error: Option<String> = None;

    for article in &article_batch {
        if cancel_token.is_cancelled() {
            break;
        }

        let msg_id = if article.message_id.starts_with('<') {
            article.message_id.clone()
        } else {
            format!("<{}>", article.message_id)
        };

        match article_provider.fetch_articles(&[&msg_id], 1).await {
            Ok(responses) if !responses.is_empty() => {
                let response = &responses[0];

                match decode_and_write(article, &response.data, &output_files, &download_temp_dir) {
                    Ok(decoded_size) => {
                        let _ = batch_tx
                            .send((article.id, crate::db::article_status::DOWNLOADED))
                            .await;
                        downloaded_articles.fetch_add(1, Ordering::Relaxed);
                        downloaded_bytes.fetch_add(decoded_size, Ordering::Relaxed);
                        successful_results.push((article.segment_number, decoded_size));
                    }
                    Err(e) => {
                        tracing::debug!(
                            download_id = id.0,
                            article_id = article.id,
                            error = %e,
                            "Failed to decode/write article during individual retry"
                        );
                        failed_articles.fetch_add(1, Ordering::Relaxed);
                        if first_error.is_none() {
                            first_error = Some(format!("Failed to decode/write article: {}", e));
                        }
                        let _ = batch_tx
                            .send((article.id, crate::db::article_status::FAILED))
                            .await;
                        continue;
                    }
                }
            }
            Ok(_) => {
                // Empty response = article missing
                tracing::debug!(
                    download_id = id.0,
                    article_id = article.id,
                    message_id = %article.message_id,
                    "Article missing (empty response)"
                );
                failed_articles.fetch_add(1, Ordering::Relaxed);
                if first_error.is_none() {
                    first_error = Some(format!("No such article: {}", article.message_id));
                }
                let _ = batch_tx
                    .send((article.id, crate::db::article_status::FAILED))
                    .await;
            }
            Err(e) => {
                tracing::debug!(
                    download_id = id.0,
                    article_id = article.id,
                    message_id = %article.message_id,
                    error = %e,
                    "Article fetch failed during individual retry"
                );
                failed_articles.fetch_add(1, Ordering::Relaxed);
                if first_error.is_none() {
                    first_error = Some(format!("No such article: {}", article.message_id));
                }
                let _ = batch_tx
                    .send((article.id, crate::db::article_status::FAILED))
                    .await;
            }
        }
    }

    if successful_results.is_empty() {
        Err((
            first_error.unwrap_or_else(|| "All articles in batch failed".to_string()),
            batch_size,
        ))
    } else {
        // Partial success: return the articles we got, failures are already tracked
        // via the failed_articles atomic and batch_tx
        Ok(successful_results)
    }
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
        total_articles,
        individually_failed,
    } = results;

    // Combine batch-level failures with individual article failures
    let total_failed = failed_count as u64 + individually_failed;
    let total = success_count as u64 + total_failed;
    let max_failure_ratio = ctx.config.download.max_failure_ratio;

    // Handle partial or total failures
    if total_failed > 0 {
        tracing::warn!(
            download_id = id.0,
            batch_failed = failed_count,
            individually_failed = individually_failed,
            total_failed = total_failed,
            succeeded = success_count,
            total = total,
            total_articles = total_articles,
            "Download completed with some failures"
        );

        if success_count == 0
            || (total > 0 && (total_failed as f64 / total as f64) > max_failure_ratio)
        {
            let error_msg = if let Some(ref first) = first_error {
                format!(
                    "{} of {} articles failed ({:.0}%). First error: {}",
                    total_failed,
                    total,
                    if total > 0 {
                        total_failed as f64 / total as f64 * 100.0
                    } else {
                        100.0
                    },
                    first,
                )
            } else {
                format!(
                    "{} of {} articles failed ({:.0}%)",
                    total_failed,
                    total,
                    if total > 0 {
                        total_failed as f64 / total as f64 * 100.0
                    } else {
                        100.0
                    },
                )
            };
            tracing::error!(
                download_id = id.0,
                total_failed = total_failed,
                succeeded = success_count,
                "Download failed - too many article failures"
            );
            ctx.mark_failed_with_stats(
                &error_msg,
                Some(success_count as u64),
                Some(total_failed),
                Some(total_articles as u64),
            )
            .await;
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

    ctx.event_tx
        .send(Event::DownloadComplete {
            id,
            articles_failed: if total_failed > 0 {
                Some(total_failed)
            } else {
                None
            },
            articles_total: Some(total_articles as u64),
        })
        .ok();
    ctx.remove_from_active().await;
    ctx.spawn_post_processing();
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ServerConfig};

    fn config_with_servers(servers: Vec<ServerConfig>) -> Config {
        Config {
            servers,
            ..Config::default()
        }
    }

    fn server(connections: usize, pipeline_depth: usize) -> ServerConfig {
        ServerConfig {
            host: "news.example.com".to_string(),
            port: 563,
            tls: true,
            username: None,
            password: None,
            connections,
            priority: 0,
            pipeline_depth,
        }
    }

    fn make_article(id: i64, segment: i32, size: i64) -> crate::db::Article {
        crate::db::Article {
            id,
            download_id: 1,
            message_id: format!("<article-{id}@test>"),
            segment_number: segment,
            file_index: 0,
            size_bytes: size,
            status: crate::db::article_status::PENDING,
            downloaded_at: None,
        }
    }

    // -----------------------------------------------------------------------
    // prepare_batches: concurrency calculation
    // -----------------------------------------------------------------------

    #[test]
    fn prepare_batches_sums_connections_across_all_servers() {
        let config = config_with_servers(vec![server(8, 10), server(4, 10)]);
        let articles = (0..5).map(|i| make_article(i, i as i32, 100)).collect();

        let (concurrency, _, _) = prepare_batches(&config, articles);

        assert_eq!(
            concurrency, 12,
            "concurrency should be sum of all server connections"
        );
    }

    #[test]
    fn prepare_batches_single_server_concurrency() {
        let config = config_with_servers(vec![server(20, 10)]);
        let articles = vec![make_article(1, 1, 100)];

        let (concurrency, _, _) = prepare_batches(&config, articles);

        assert_eq!(concurrency, 20);
    }

    #[test]
    fn prepare_batches_no_servers_gives_zero_concurrency() {
        let config = config_with_servers(vec![]);
        let articles = vec![make_article(1, 1, 100)];

        let (concurrency, _, _) = prepare_batches(&config, articles);

        assert_eq!(concurrency, 0);
    }

    // -----------------------------------------------------------------------
    // prepare_batches: pipeline depth
    // -----------------------------------------------------------------------

    #[test]
    fn prepare_batches_uses_first_server_pipeline_depth() {
        let config = config_with_servers(vec![server(4, 25), server(4, 50)]);
        let articles = (0..100).map(|i| make_article(i, i as i32, 100)).collect();

        let (_, pipeline_depth, _) = prepare_batches(&config, articles);

        assert_eq!(
            pipeline_depth, 25,
            "pipeline_depth should come from first server"
        );
    }

    #[test]
    fn prepare_batches_clamps_zero_pipeline_depth_to_one() {
        let config = config_with_servers(vec![server(4, 0)]);
        let articles = (0..5).map(|i| make_article(i, i as i32, 100)).collect();

        let (_, pipeline_depth, _) = prepare_batches(&config, articles);

        assert_eq!(
            pipeline_depth, 1,
            "pipeline_depth of 0 should be clamped to 1"
        );
    }

    #[test]
    fn prepare_batches_defaults_pipeline_depth_when_no_servers() {
        let config = config_with_servers(vec![]);
        let articles = vec![make_article(1, 1, 100)];

        let (_, pipeline_depth, _) = prepare_batches(&config, articles);

        assert_eq!(
            pipeline_depth, 10,
            "should default to 10 when no servers present"
        );
    }

    // -----------------------------------------------------------------------
    // prepare_batches: batch creation
    // -----------------------------------------------------------------------

    #[test]
    fn prepare_batches_creates_correct_batch_sizes() {
        let config = config_with_servers(vec![server(4, 3)]);
        let articles: Vec<_> = (0..10).map(|i| make_article(i, i as i32, 100)).collect();

        let (_, _, batches) = prepare_batches(&config, articles);

        assert_eq!(batches.len(), 4, "10 articles / batch size 3 = 4 batches");
        assert_eq!(batches[0].len(), 3);
        assert_eq!(batches[1].len(), 3);
        assert_eq!(batches[2].len(), 3);
        assert_eq!(batches[3].len(), 1, "last batch gets the remainder");
    }

    #[test]
    fn prepare_batches_preserves_article_order() {
        let config = config_with_servers(vec![server(2, 2)]);
        let articles: Vec<_> = (0..4).map(|i| make_article(i, i as i32, 100)).collect();

        let (_, _, batches) = prepare_batches(&config, articles);

        assert_eq!(batches[0][0].id, 0);
        assert_eq!(batches[0][1].id, 1);
        assert_eq!(batches[1][0].id, 2);
        assert_eq!(batches[1][1].id, 3);
    }

    #[test]
    fn prepare_batches_empty_articles_produces_no_batches() {
        let config = config_with_servers(vec![server(4, 10)]);

        let (_, _, batches) = prepare_batches(&config, vec![]);

        assert!(batches.is_empty());
    }

    #[test]
    fn prepare_batches_single_article_makes_one_batch() {
        let config = config_with_servers(vec![server(4, 10)]);
        let articles = vec![make_article(42, 1, 500)];

        let (_, _, batches) = prepare_batches(&config, articles);

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 1);
        assert_eq!(batches[0][0].id, 42);
    }

    #[test]
    fn prepare_batches_exactly_one_batch_when_articles_equal_pipeline_depth() {
        let config = config_with_servers(vec![server(4, 5)]);
        let articles: Vec<_> = (0..5).map(|i| make_article(i, i as i32, 100)).collect();

        let (_, _, batches) = prepare_batches(&config, articles);

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 5);
    }

    // -----------------------------------------------------------------------
    // aggregate_results: all success
    // -----------------------------------------------------------------------

    #[test]
    fn aggregate_results_all_success() {
        let results: BatchResultVec = vec![Ok(vec![(1, 100), (2, 200)]), Ok(vec![(3, 300)])];

        let agg = aggregate_results(results);

        assert_eq!(agg.success_count, 3);
        assert_eq!(agg.failed_count, 0);
        assert!(agg.first_error.is_none());
    }

    // -----------------------------------------------------------------------
    // aggregate_results: all failure
    // -----------------------------------------------------------------------

    #[test]
    fn aggregate_results_all_failure() {
        let results: BatchResultVec = vec![
            Err(("timeout".to_string(), 5)),
            Err(("connection reset".to_string(), 3)),
        ];

        let agg = aggregate_results(results);

        assert_eq!(agg.success_count, 0);
        assert_eq!(agg.failed_count, 8);
        assert_eq!(agg.first_error.as_deref(), Some("timeout"));
    }

    // -----------------------------------------------------------------------
    // aggregate_results: mixed results
    // -----------------------------------------------------------------------

    #[test]
    fn aggregate_results_mixed_preserves_first_error() {
        let results: BatchResultVec = vec![
            Ok(vec![(1, 100)]),
            Err(("first failure".to_string(), 2)),
            Ok(vec![(2, 200), (3, 300)]),
            Err(("second failure".to_string(), 1)),
        ];

        let agg = aggregate_results(results);

        assert_eq!(agg.success_count, 3);
        assert_eq!(agg.failed_count, 3);
        assert_eq!(
            agg.first_error.as_deref(),
            Some("first failure"),
            "first_error should be from the first Err encountered"
        );
    }

    // -----------------------------------------------------------------------
    // aggregate_results: empty
    // -----------------------------------------------------------------------

    #[test]
    fn aggregate_results_empty_input() {
        let results: BatchResultVec = vec![];

        let agg = aggregate_results(results);

        assert_eq!(agg.success_count, 0);
        assert_eq!(agg.failed_count, 0);
        assert!(agg.first_error.is_none());
    }

    // -----------------------------------------------------------------------
    // aggregate_results: edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn aggregate_results_single_success_batch() {
        let results: BatchResultVec = vec![Ok(vec![(1, 50)])];

        let agg = aggregate_results(results);

        assert_eq!(agg.success_count, 1);
        assert_eq!(agg.failed_count, 0);
    }

    #[test]
    fn aggregate_results_success_batch_with_empty_vec() {
        let results: BatchResultVec = vec![Ok(vec![])];

        let agg = aggregate_results(results);

        assert_eq!(
            agg.success_count, 0,
            "an Ok with empty vec contributes 0 to success_count"
        );
        assert_eq!(agg.failed_count, 0);
    }

    #[test]
    fn aggregate_results_failure_with_zero_batch_size() {
        let results: BatchResultVec = vec![Err(("weird error".to_string(), 0))];

        let agg = aggregate_results(results);

        assert_eq!(agg.success_count, 0);
        assert_eq!(agg.failed_count, 0);
        assert_eq!(agg.first_error.as_deref(), Some("weird error"));
    }

    // -----------------------------------------------------------------------
    // max_failure_ratio boundary tests (uses config default of 0.5)
    // -----------------------------------------------------------------------

    #[test]
    fn max_failure_ratio_exactly_at_boundary_does_not_fail() {
        // finalize_download uses strictly-greater: (failed / total) > max_failure_ratio
        // At exactly 50% failures (ratio == 0.5), the download should NOT be marked failed
        let max_ratio = crate::config::DownloadConfig::default().max_failure_ratio;
        let success_count: usize = 50;
        let failed_count: usize = 50;
        let total = success_count + failed_count;
        let ratio = failed_count as f64 / total as f64;

        assert!(
            !(success_count == 0 || ratio > max_ratio),
            "exactly 50% failures should NOT trigger failure (uses > not >=)"
        );
    }

    #[test]
    fn max_failure_ratio_just_above_boundary_fails() {
        let max_ratio = crate::config::DownloadConfig::default().max_failure_ratio;
        let success_count: usize = 49;
        let failed_count: usize = 51;
        let total = success_count + failed_count;
        let ratio = failed_count as f64 / total as f64;

        assert!(
            success_count == 0 || ratio > max_ratio,
            "51% failures should trigger failure"
        );
    }

    #[test]
    fn max_failure_ratio_all_failures_always_fails() {
        let max_ratio = crate::config::DownloadConfig::default().max_failure_ratio;
        let success_count: usize = 0;
        let failed_count: usize = 100;

        // Even without checking the ratio, success_count == 0 triggers failure
        assert!(
            success_count == 0
                || (failed_count as f64 / (success_count + failed_count) as f64) > max_ratio,
            "zero successes should always trigger failure regardless of ratio"
        );
    }

    #[test]
    fn max_failure_ratio_single_failure_in_large_set_does_not_fail() {
        let max_ratio = crate::config::DownloadConfig::default().max_failure_ratio;
        let success_count: usize = 999;
        let failed_count: usize = 1;
        let total = success_count + failed_count;
        let ratio = failed_count as f64 / total as f64;

        assert!(
            !(success_count == 0 || ratio > max_ratio),
            "0.1% failure rate should not trigger failure"
        );
    }

    // ===================================================================
    // MockArticleProvider and test context helpers
    // ===================================================================

    use std::collections::VecDeque;

    struct MockArticleProvider {
        responses: std::sync::Mutex<VecDeque<nntp_rs::Result<Vec<nntp_rs::NntpBinaryResponse>>>>,
    }

    impl MockArticleProvider {
        /// Returns Ok with NntpBinaryResponse for each data Vec
        fn succeeding(data: Vec<Vec<u8>>) -> Self {
            let responses: Vec<nntp_rs::NntpBinaryResponse> = data
                .into_iter()
                .map(|d| nntp_rs::NntpBinaryResponse {
                    code: 222,
                    message: "Body follows".into(),
                    data: d,
                })
                .collect();
            Self {
                responses: std::sync::Mutex::new(VecDeque::from(vec![Ok(responses)])),
            }
        }

        /// Returns Err for every call
        fn failing(err_msg: &str) -> Self {
            Self {
                responses: std::sync::Mutex::new(VecDeque::from(vec![Err(
                    nntp_rs::NntpError::Other(err_msg.to_string()),
                )])),
            }
        }

        /// Custom sequence of responses
        fn with_responses(
            responses: Vec<nntp_rs::Result<Vec<nntp_rs::NntpBinaryResponse>>>,
        ) -> Self {
            Self {
                responses: std::sync::Mutex::new(VecDeque::from(responses)),
            }
        }
    }

    #[async_trait::async_trait]
    impl super::ArticleProvider for MockArticleProvider {
        async fn fetch_articles(
            &self,
            _message_ids: &[&str],
            _pipeline_depth: usize,
        ) -> nntp_rs::Result<Vec<nntp_rs::NntpBinaryResponse>> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| {
                    Err(nntp_rs::NntpError::Other(
                        "No more mock responses".to_string(),
                    ))
                })
        }
    }

    /// Helper: create a NewDownload for test insertion.
    fn make_new_download(temp_dir: &tempfile::TempDir) -> crate::db::NewDownload {
        crate::db::NewDownload {
            name: "test".to_string(),
            nzb_path: "/tmp/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: temp_dir.path().join("dest").to_string_lossy().to_string(),
            post_process: 0,
            priority: 0,
            status: crate::types::Status::Queued.to_i32(),
            size_bytes: 1000,
        }
    }

    /// Helper: insert N pending articles for a download.
    async fn insert_test_articles(
        db: &crate::db::Database,
        download_id: crate::types::DownloadId,
        count: usize,
    ) -> Vec<i64> {
        let mut article_ids = Vec::with_capacity(count);
        for i in 0..count {
            let article = crate::db::NewArticle {
                download_id,
                message_id: format!("<seg-{}@test>", i + 1),
                segment_number: (i + 1) as i32,
                file_index: 0,
                size_bytes: 100,
            };
            let id = db.insert_article(&article).await.unwrap();
            article_ids.push(id);
        }
        article_ids
    }

    /// Build a full DownloadTaskContext backed by a real SQLite DB and the given mock provider.
    async fn make_test_context(
        provider: Arc<dyn super::ArticleProvider>,
    ) -> (
        super::DownloadTaskContext,
        tempfile::TempDir,
        tokio::sync::broadcast::Receiver<crate::types::Event>,
    ) {
        use crate::parity::NoOpParityHandler;

        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let mut config = Config::default();
        config.persistence.database_path = db_path;
        config.servers = vec![];
        config.download.max_concurrent_downloads = 3;
        config.download.temp_dir = temp_dir.path().join("tmp");

        // Initialize database
        let db = crate::db::Database::new(&config.persistence.database_path)
            .await
            .unwrap();

        // Broadcast channel
        let (event_tx, event_rx) = tokio::sync::broadcast::channel(1000);

        // Speed limiter (unlimited)
        let speed_limiter =
            crate::speed_limiter::SpeedLimiter::new(config.download.speed_limit_bps);

        let config_arc = std::sync::Arc::new(config.clone());
        let db_arc = std::sync::Arc::new(db);

        // Active downloads map
        let active_downloads =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

        // Queue state
        let queue =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BinaryHeap::new()));
        let concurrent_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
            config.download.max_concurrent_downloads,
        ));
        let queue_state = super::super::QueueState {
            queue,
            concurrent_limit,
            active_downloads: active_downloads.clone(),
            accepting_new: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        };

        // Runtime config
        let categories = std::sync::Arc::new(tokio::sync::RwLock::new(
            config.persistence.categories.clone(),
        ));
        let schedule_rules = std::sync::Arc::new(tokio::sync::RwLock::new(vec![]));
        let next_schedule_rule_id = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
        let runtime_config = super::super::RuntimeConfig {
            categories,
            schedule_rules,
            next_schedule_rule_id,
        };

        // Parity + post-processor
        let parity_handler: std::sync::Arc<dyn crate::parity::ParityHandler> =
            std::sync::Arc::new(NoOpParityHandler);
        let post_processor = std::sync::Arc::new(crate::post_processing::PostProcessor::new(
            event_tx.clone(),
            config_arc.clone(),
            parity_handler.clone(),
            db_arc.clone(),
        ));
        let processing = super::super::ProcessingPipeline {
            post_processor,
            parity_handler,
        };

        let downloader = super::super::UsenetDownloader {
            db: db_arc.clone(),
            event_tx: event_tx.clone(),
            config: config_arc.clone(),
            nntp_pools: std::sync::Arc::new(Vec::new()),
            speed_limiter: speed_limiter.clone(),
            queue_state,
            runtime_config,
            processing,
        };

        let ctx = super::DownloadTaskContext {
            id: crate::types::DownloadId(1), // placeholder; tests override
            db: db_arc,
            event_tx,
            article_provider: provider,
            config: config_arc,
            active_downloads,
            speed_limiter,
            cancel_token: tokio_util::sync::CancellationToken::new(),
            downloader,
        };

        (ctx, temp_dir, event_rx)
    }

    // ===================================================================
    // fetch_download_record tests
    // ===================================================================

    #[tokio::test]
    async fn fetch_download_record_not_found() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
        let (ctx, _temp_dir, _rx) = make_test_context(provider).await;
        // ctx.id defaults to DownloadId(1) which doesn't exist in DB

        let result = super::fetch_download_record(&ctx).await;

        assert!(
            result.is_none(),
            "should return None when download is not in DB"
        );
    }

    #[tokio::test]
    async fn fetch_download_record_transitions_to_downloading() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
        let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

        // Insert download + 3 articles
        let new_dl = make_new_download(&temp_dir);
        let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
        ctx.id = dl_id;
        insert_test_articles(&ctx.db, dl_id, 3).await;

        let result = super::fetch_download_record(&ctx).await;

        // Returns Some with 3 pending articles
        let (_download, articles) = result.expect("should return Some");
        assert_eq!(articles.len(), 3, "should return all 3 pending articles");

        // DB status changed to Downloading
        let db_dl = ctx.db.get_download(dl_id).await.unwrap().unwrap();
        assert_eq!(
            db_dl.status,
            crate::types::Status::Downloading.to_i32(),
            "status should be Downloading"
        );

        // started_at is set
        assert!(
            db_dl.started_at.is_some(),
            "started_at should be set after transitioning to Downloading"
        );

        // Event::Downloading{percent:0.0} was emitted
        let event = rx.try_recv().unwrap();
        match event {
            crate::types::Event::Downloading {
                id,
                percent,
                speed_bps,
                ..
            } => {
                assert_eq!(id, dl_id);
                assert!((percent - 0.0).abs() < f32::EPSILON);
                assert_eq!(speed_bps, 0);
            }
            other => panic!("expected Downloading event, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn fetch_download_record_returns_only_pending_articles() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
        let (mut ctx, temp_dir, _rx) = make_test_context(provider).await;

        let new_dl = make_new_download(&temp_dir);
        let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
        ctx.id = dl_id;

        // Insert 5 articles
        let article_ids = insert_test_articles(&ctx.db, dl_id, 5).await;

        // Mark 2 as DOWNLOADED
        ctx.db
            .update_articles_status_batch(&[
                (article_ids[0], crate::db::article_status::DOWNLOADED),
                (article_ids[1], crate::db::article_status::DOWNLOADED),
            ])
            .await
            .unwrap();

        let result = super::fetch_download_record(&ctx).await;
        let (_download, articles) = result.unwrap();
        assert_eq!(
            articles.len(),
            3,
            "should return only the 3 pending articles"
        );
    }

    // ===================================================================
    // finalize_download tests
    // ===================================================================

    #[tokio::test]
    async fn finalize_all_success_marks_complete() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
        let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

        let new_dl = make_new_download(&temp_dir);
        let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
        ctx.id = dl_id;
        let db = ctx.db.clone();

        super::finalize_download(
            ctx,
            super::DownloadResults {
                success_count: 10,
                failed_count: 0,
                first_error: None,
                total_articles: 10,
                individually_failed: 0,
            },
            1000,
        )
        .await;

        // DB status = Complete
        let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
        assert_eq!(
            db_dl.status,
            crate::types::Status::Complete.to_i32(),
            "status should be Complete"
        );

        // Event: DownloadComplete
        let event = rx.try_recv().unwrap();
        match event {
            crate::types::Event::DownloadComplete { id, .. } => {
                assert_eq!(id, dl_id);
            }
            other => panic!("expected DownloadComplete event, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn finalize_above_threshold_marks_failed() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
        let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

        let new_dl = make_new_download(&temp_dir);
        let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
        ctx.id = dl_id;
        let db = ctx.db.clone();

        super::finalize_download(
            ctx,
            super::DownloadResults {
                success_count: 4,
                failed_count: 6,
                first_error: Some("batch fetch failed".to_string()),
                total_articles: 10,
                individually_failed: 0,
            },
            1000,
        )
        .await;

        // DB status = Failed
        let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
        assert_eq!(
            db_dl.status,
            crate::types::Status::Failed.to_i32(),
            "60% failure rate should mark Failed"
        );

        // Event: DownloadFailed
        let event = rx.try_recv().unwrap();
        match event {
            crate::types::Event::DownloadFailed { id, error, .. } => {
                assert_eq!(id, dl_id);
                assert!(
                    error.contains("articles failed"),
                    "error should contain article stats, got: {error}"
                );
            }
            other => panic!("expected DownloadFailed event, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn finalize_at_exact_boundary_marks_complete() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
        let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

        let new_dl = make_new_download(&temp_dir);
        let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
        ctx.id = dl_id;
        let db = ctx.db.clone();

        super::finalize_download(
            ctx,
            super::DownloadResults {
                success_count: 50,
                failed_count: 50,
                first_error: Some("some error".to_string()),
                total_articles: 100,
                individually_failed: 0,
            },
            1000,
        )
        .await;

        // At exactly 50% the condition is > 0.5, so it should NOT fail
        let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
        assert_eq!(
            db_dl.status,
            crate::types::Status::Complete.to_i32(),
            "exactly 50% failures uses strict >, so should be Complete"
        );

        let event = rx.try_recv().unwrap();
        assert!(
            matches!(event, crate::types::Event::DownloadComplete { .. }),
            "expected DownloadComplete, got {:?}",
            event
        );
    }

    #[tokio::test]
    async fn finalize_zero_success_always_fails() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
        let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

        let new_dl = make_new_download(&temp_dir);
        let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
        ctx.id = dl_id;
        let db = ctx.db.clone();

        super::finalize_download(
            ctx,
            super::DownloadResults {
                success_count: 0,
                failed_count: 5,
                first_error: Some("total failure".to_string()),
                total_articles: 5,
                individually_failed: 0,
            },
            1000,
        )
        .await;

        let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
        assert_eq!(
            db_dl.status,
            crate::types::Status::Failed.to_i32(),
            "zero success should always fail"
        );

        let event = rx.try_recv().unwrap();
        assert!(
            matches!(event, crate::types::Event::DownloadFailed { .. }),
            "expected DownloadFailed, got {:?}",
            event
        );
    }

    // ===================================================================
    // fetch_article_batch tests
    // ===================================================================

    /// Helper to create empty OutputFiles (no DirectWrite — fallback to article_N.dat)
    fn empty_output_files() -> Arc<super::OutputFiles> {
        Arc::new(super::OutputFiles {
            files: std::collections::HashMap::new(),
        })
    }

    #[tokio::test]
    async fn fetch_article_batch_cancelled() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![b"data".to_vec()]));
        let cancel_token = tokio_util::sync::CancellationToken::new();
        cancel_token.cancel(); // pre-cancel

        let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(100);
        let articles = vec![make_article(1, 1, 100)];
        let temp_dir = tempfile::tempdir().unwrap();

        let result = super::fetch_article_batch(super::FetchArticleBatchParams {
            id: crate::types::DownloadId(1),
            article_batch: articles,
            article_provider: provider,
            batch_tx,
            speed_limiter: crate::speed_limiter::SpeedLimiter::new(None),
            cancel_token,
            download_temp_dir: temp_dir.path().to_path_buf(),
            downloaded_bytes: Arc::new(AtomicU64::new(0)),
            downloaded_articles: Arc::new(AtomicU64::new(0)),
            failed_articles: Arc::new(AtomicU64::new(0)),
            output_files: empty_output_files(),
            pipeline_depth: 10,
        })
        .await;

        assert!(result.is_err(), "should return Err when cancelled");
        let (msg, count) = result.unwrap_err();
        assert_eq!(msg, "Download cancelled");
        assert_eq!(count, 1);

        // No status updates sent
        assert!(
            batch_rx.try_recv().is_err(),
            "no status updates should be sent when cancelled"
        );
    }

    #[tokio::test]
    async fn fetch_article_batch_success_writes_files() {
        let article_data = vec![b"hello world".to_vec(), b"second article".to_vec()];
        let provider = Arc::new(MockArticleProvider::succeeding(article_data.clone()));

        let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(100);
        let temp_dir = tempfile::tempdir().unwrap();
        let downloaded_bytes = Arc::new(AtomicU64::new(0));
        let downloaded_articles = Arc::new(AtomicU64::new(0));

        let articles = vec![make_article(10, 1, 50), make_article(11, 2, 60)];

        let result = super::fetch_article_batch(super::FetchArticleBatchParams {
            id: crate::types::DownloadId(1),
            article_batch: articles,
            article_provider: provider,
            batch_tx,
            speed_limiter: crate::speed_limiter::SpeedLimiter::new(None),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            download_temp_dir: temp_dir.path().to_path_buf(),
            downloaded_bytes: downloaded_bytes.clone(),
            downloaded_articles: downloaded_articles.clone(),
            failed_articles: Arc::new(AtomicU64::new(0)),
            output_files: empty_output_files(),
            pipeline_depth: 10,
        })
        .await;

        // Assert success — with empty OutputFiles, yEnc decode fails so raw data is written
        let batch_results = result.unwrap();
        assert_eq!(batch_results.len(), 2);
        // Sizes are now the raw data sizes (yEnc decode fails, fallback writes raw bytes)
        assert_eq!(batch_results[0].0, 1); // segment_number
        assert_eq!(batch_results[1].0, 2); // segment_number

        // Files exist on disk (fallback article_N.dat since no OutputFiles mapping)
        let file1 = temp_dir.path().join("article_1.dat");
        let file2 = temp_dir.path().join("article_2.dat");
        assert!(file1.exists(), "article_1.dat should exist");
        assert!(file2.exists(), "article_2.dat should exist");
        assert_eq!(
            std::fs::read(&file1).unwrap(),
            b"hello world",
            "article_1.dat content should match"
        );
        assert_eq!(
            std::fs::read(&file2).unwrap(),
            b"second article",
            "article_2.dat content should match"
        );

        // Atomics incremented
        assert_eq!(downloaded_articles.load(Ordering::Relaxed), 2);
        let total_bytes = downloaded_bytes.load(Ordering::Relaxed);
        assert_eq!(
            total_bytes,
            (b"hello world".len() + b"second article".len()) as u64,
            "bytes should reflect raw data sizes"
        );

        // DOWNLOADED status sent via batch_tx for both articles
        let (id1, status1) = batch_rx.try_recv().unwrap();
        assert_eq!(id1, 10);
        assert_eq!(status1, crate::db::article_status::DOWNLOADED);
        let (id2, status2) = batch_rx.try_recv().unwrap();
        assert_eq!(id2, 11);
        assert_eq!(status2, crate::db::article_status::DOWNLOADED);
    }

    #[tokio::test]
    async fn fetch_article_batch_provider_error() {
        let provider = Arc::new(MockArticleProvider::failing("connection refused"));

        let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(100);
        let temp_dir = tempfile::tempdir().unwrap();

        let articles = vec![make_article(20, 1, 100), make_article(21, 2, 200)];

        let result = super::fetch_article_batch(super::FetchArticleBatchParams {
            id: crate::types::DownloadId(1),
            article_batch: articles,
            article_provider: provider,
            batch_tx,
            speed_limiter: crate::speed_limiter::SpeedLimiter::new(None),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            download_temp_dir: temp_dir.path().to_path_buf(),
            downloaded_bytes: Arc::new(AtomicU64::new(0)),
            downloaded_articles: Arc::new(AtomicU64::new(0)),
            failed_articles: Arc::new(AtomicU64::new(0)),
            output_files: empty_output_files(),
            pipeline_depth: 10,
        })
        .await;

        assert!(result.is_err(), "should return Err on provider failure");
        let (msg, count) = result.unwrap_err();
        assert!(
            msg.contains("connection refused"),
            "error should contain provider message"
        );
        assert_eq!(count, 2, "batch_size should be 2");

        // FAILED status sent for all articles
        let (id1, status1) = batch_rx.try_recv().unwrap();
        assert_eq!(id1, 20);
        assert_eq!(status1, crate::db::article_status::FAILED);
        let (id2, status2) = batch_rx.try_recv().unwrap();
        assert_eq!(id2, 21);
        assert_eq!(status2, crate::db::article_status::FAILED);
    }

    // ===================================================================
    // run_download_task integration tests
    // ===================================================================

    #[tokio::test]
    async fn run_download_task_full_lifecycle() {
        // Mock provider that returns data for 3 articles
        let provider = Arc::new(MockArticleProvider::with_responses(vec![
            Ok(vec![nntp_rs::NntpBinaryResponse {
                code: 222,
                message: "Body follows".into(),
                data: b"article-1-data".to_vec(),
            }]),
            Ok(vec![nntp_rs::NntpBinaryResponse {
                code: 222,
                message: "Body follows".into(),
                data: b"article-2-data".to_vec(),
            }]),
            Ok(vec![nntp_rs::NntpBinaryResponse {
                code: 222,
                message: "Body follows".into(),
                data: b"article-3-data".to_vec(),
            }]),
        ]));

        let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

        // Insert download + 3 articles
        let new_dl = make_new_download(&temp_dir);
        let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
        ctx.id = dl_id;
        insert_test_articles(&ctx.db, dl_id, 3).await;

        // Need to configure a server for batching (pipeline_depth=1 so each article is its own batch)
        let mut config = (*ctx.config).clone();
        config.servers = vec![server(1, 1)];
        ctx.config = std::sync::Arc::new(config);

        let db = ctx.db.clone();
        super::run_download_task(ctx).await;

        // DB status = Complete
        let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
        assert_eq!(
            db_dl.status,
            crate::types::Status::Complete.to_i32(),
            "download should be Complete after successful lifecycle"
        );

        // Collect events — find DownloadComplete
        let mut found_complete = false;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, crate::types::Event::DownloadComplete { id, .. } if id == dl_id) {
                found_complete = true;
            }
        }
        assert!(
            found_complete,
            "DownloadComplete event should have been emitted"
        );
    }

    // ===================================================================
    // retry_articles_individually tests
    // ===================================================================

    #[tokio::test]
    async fn retry_articles_individually_partial_success() {
        // Mock: 3 individual fetches — article 1 succeeds, article 2 missing, article 3 succeeds
        let provider = Arc::new(MockArticleProvider::with_responses(vec![
            // Article 1: success
            Ok(vec![nntp_rs::NntpBinaryResponse {
                code: 222,
                message: "Body follows".into(),
                data: b"article-1-data".to_vec(),
            }]),
            // Article 2: NoSuchArticle
            Err(nntp_rs::NntpError::NoSuchArticle(
                "<article-2@test>".to_string(),
            )),
            // Article 3: success
            Ok(vec![nntp_rs::NntpBinaryResponse {
                code: 222,
                message: "Body follows".into(),
                data: b"article-3-data".to_vec(),
            }]),
        ]));

        let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(100);
        let temp_dir = tempfile::tempdir().unwrap();
        let downloaded_bytes = Arc::new(AtomicU64::new(0));
        let downloaded_articles = Arc::new(AtomicU64::new(0));
        let failed_articles = Arc::new(AtomicU64::new(0));

        let articles = vec![
            make_article(1, 1, 100),
            make_article(2, 2, 200),
            make_article(3, 3, 300),
        ];

        let result = super::retry_articles_individually(super::RetryArticlesParams {
            id: crate::types::DownloadId(1),
            article_batch: articles,
            article_provider: provider,
            batch_tx,
            cancel_token: tokio_util::sync::CancellationToken::new(),
            download_temp_dir: temp_dir.path().to_path_buf(),
            downloaded_bytes: downloaded_bytes.clone(),
            downloaded_articles: downloaded_articles.clone(),
            failed_articles: failed_articles.clone(),
            output_files: empty_output_files(),
        })
        .await;

        // Should succeed with 2 articles
        let batch_results = result.unwrap();
        assert_eq!(batch_results.len(), 2, "2 of 3 articles should succeed");
        // Sizes are raw data sizes (yEnc decode fails, fallback writes raw bytes)
        assert_eq!(batch_results[0].0, 1); // segment_number
        assert_eq!(batch_results[1].0, 3); // segment_number

        // Counters — sizes are raw byte counts
        assert_eq!(downloaded_articles.load(Ordering::Relaxed), 2);
        let total_bytes = downloaded_bytes.load(Ordering::Relaxed);
        assert_eq!(
            total_bytes,
            (b"article-1-data".len() + b"article-3-data".len()) as u64
        );
        assert_eq!(failed_articles.load(Ordering::Relaxed), 1);

        // Status updates via batch_tx
        let (id1, status1) = batch_rx.try_recv().unwrap();
        assert_eq!(id1, 1);
        assert_eq!(status1, crate::db::article_status::DOWNLOADED);

        let (id2, status2) = batch_rx.try_recv().unwrap();
        assert_eq!(id2, 2);
        assert_eq!(status2, crate::db::article_status::FAILED);

        let (id3, status3) = batch_rx.try_recv().unwrap();
        assert_eq!(id3, 3);
        assert_eq!(status3, crate::db::article_status::DOWNLOADED);
    }

    #[tokio::test]
    async fn retry_articles_individually_all_missing() {
        // Mock: all 3 articles return 430
        let provider = Arc::new(MockArticleProvider::with_responses(vec![
            Err(nntp_rs::NntpError::NoSuchArticle(
                "<article-1@test>".to_string(),
            )),
            Err(nntp_rs::NntpError::NoSuchArticle(
                "<article-2@test>".to_string(),
            )),
            Err(nntp_rs::NntpError::NoSuchArticle(
                "<article-3@test>".to_string(),
            )),
        ]));

        let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(100);
        let temp_dir = tempfile::tempdir().unwrap();
        let failed_articles = Arc::new(AtomicU64::new(0));

        let articles = vec![
            make_article(10, 1, 100),
            make_article(11, 2, 200),
            make_article(12, 3, 300),
        ];

        let result = super::retry_articles_individually(super::RetryArticlesParams {
            id: crate::types::DownloadId(1),
            article_batch: articles,
            article_provider: provider,
            batch_tx,
            cancel_token: tokio_util::sync::CancellationToken::new(),
            download_temp_dir: temp_dir.path().to_path_buf(),
            downloaded_bytes: Arc::new(AtomicU64::new(0)),
            downloaded_articles: Arc::new(AtomicU64::new(0)),
            failed_articles: failed_articles.clone(),
            output_files: empty_output_files(),
        })
        .await;

        // Should fail — all articles missing
        assert!(result.is_err(), "should return Err when all articles fail");
        let (msg, count) = result.unwrap_err();
        assert!(
            msg.contains("No such article"),
            "error should mention missing article, got: {msg}"
        );
        assert_eq!(count, 3, "batch_size should be 3");

        // All 3 articles counted as failed
        assert_eq!(failed_articles.load(Ordering::Relaxed), 3);

        // All 3 marked FAILED via batch_tx
        for expected_id in [10, 11, 12] {
            let (id, status) = batch_rx.try_recv().unwrap();
            assert_eq!(id, expected_id);
            assert_eq!(status, crate::db::article_status::FAILED);
        }
    }

    // ===================================================================
    // fast-fail watcher tests
    // ===================================================================

    #[tokio::test]
    async fn fast_fail_cancels_when_mostly_missing() {
        let downloaded_articles = Arc::new(AtomicU64::new(0));
        let failed_articles = Arc::new(AtomicU64::new(0));
        let cancel_token = tokio_util::sync::CancellationToken::new();

        // Configure: threshold 0.8, sample size 5
        let _watcher = super::spawn_fast_fail_watcher(
            &downloaded_articles,
            &failed_articles,
            0.8,
            5,
            cancel_token.clone(),
        );

        // Simulate 4 failures, 1 success (80% failure = threshold)
        failed_articles.store(4, Ordering::Relaxed);
        downloaded_articles.store(1, Ordering::Relaxed);

        // Wait for the watcher to detect it (polls every 200ms)
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert!(
            cancel_token.is_cancelled(),
            "should cancel when failure ratio >= threshold (4/5 = 0.8 >= 0.8)"
        );
    }

    #[tokio::test]
    async fn fast_fail_does_not_cancel_below_threshold() {
        let downloaded_articles = Arc::new(AtomicU64::new(0));
        let failed_articles = Arc::new(AtomicU64::new(0));
        let cancel_token = tokio_util::sync::CancellationToken::new();

        let _watcher = super::spawn_fast_fail_watcher(
            &downloaded_articles,
            &failed_articles,
            0.8,
            5,
            cancel_token.clone(),
        );

        // 3 failures, 2 successes (60% < 80% threshold)
        failed_articles.store(3, Ordering::Relaxed);
        downloaded_articles.store(2, Ordering::Relaxed);

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert!(
            !cancel_token.is_cancelled(),
            "should NOT cancel when failure ratio < threshold (3/5 = 0.6 < 0.8)"
        );
    }

    // ===================================================================
    // configurable failure ratio tests
    // ===================================================================

    #[tokio::test]
    async fn configurable_failure_ratio_from_config() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
        let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

        // Set a low failure ratio threshold
        let mut config = (*ctx.config).clone();
        config.download.max_failure_ratio = 0.1; // 10% threshold
        ctx.config = std::sync::Arc::new(config);

        let new_dl = make_new_download(&temp_dir);
        let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
        ctx.id = dl_id;
        let db = ctx.db.clone();

        // 85 successes, 15 failures = 15% > 10% threshold
        super::finalize_download(
            ctx,
            super::DownloadResults {
                success_count: 85,
                failed_count: 15,
                first_error: Some("missing article".to_string()),
                total_articles: 100,
                individually_failed: 0,
            },
            10000,
        )
        .await;

        let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
        assert_eq!(
            db_dl.status,
            crate::types::Status::Failed.to_i32(),
            "15% failure rate should trigger failure with max_failure_ratio=0.1"
        );

        let event = rx.try_recv().unwrap();
        match event {
            crate::types::Event::DownloadFailed { id, error, .. } => {
                assert_eq!(id, dl_id);
                assert!(
                    error.contains("articles failed"),
                    "error should contain article stats, got: {error}"
                );
            }
            other => panic!("expected DownloadFailed event, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn partial_success_emits_complete_with_stats() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
        let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

        let new_dl = make_new_download(&temp_dir);
        let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
        ctx.id = dl_id;
        let db = ctx.db.clone();

        // 90 successes, 10 failures = 10% (at default 50% threshold, this is fine)
        super::finalize_download(
            ctx,
            super::DownloadResults {
                success_count: 90,
                failed_count: 5,
                first_error: Some("missing".to_string()),
                total_articles: 100,
                individually_failed: 5, // 5 batch + 5 individual = 10 total
            },
            10000,
        )
        .await;

        let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
        assert_eq!(
            db_dl.status,
            crate::types::Status::Complete.to_i32(),
            "10% failure rate should still complete with default 50% threshold"
        );

        let event = rx.try_recv().unwrap();
        match event {
            crate::types::Event::DownloadComplete {
                id,
                articles_failed,
                articles_total,
            } => {
                assert_eq!(id, dl_id);
                assert_eq!(
                    articles_failed,
                    Some(10),
                    "should report 10 total failed articles"
                );
                assert_eq!(
                    articles_total,
                    Some(100),
                    "should report 100 total articles"
                );
            }
            other => panic!("expected DownloadComplete event, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn run_download_task_empty_articles() {
        let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
        let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

        // Insert download with 0 articles
        let new_dl = make_new_download(&temp_dir);
        let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
        ctx.id = dl_id;
        // No articles inserted

        let db = ctx.db.clone();
        super::run_download_task(ctx).await;

        // DownloadComplete should be emitted immediately
        let mut found_complete = false;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, crate::types::Event::DownloadComplete { id, .. } if id == dl_id) {
                found_complete = true;
            }
        }
        assert!(
            found_complete,
            "DownloadComplete event should be emitted immediately for empty articles"
        );

        // DB status should still be Downloading (fetch_download_record sets it)
        // since finalize_download is not called for empty articles path
        let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
        assert_eq!(
            db_dl.status,
            crate::types::Status::Downloading.to_i32(),
            "status should be Downloading (empty articles skip finalize)"
        );
    }
}
