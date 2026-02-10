//! Download task execution — core download lifecycle and article fetching.

use crate::types::{DownloadId, Event, Status};
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::UsenetDownloader;

/// Maximum article failure ratio before considering a download failed (50%)
const MAX_FAILURE_RATIO: f64 = 0.5;

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
            let article_provider = Arc::clone(&ctx.article_provider);
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
                    article_provider,
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
        article_provider,
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
    // MAX_FAILURE_RATIO boundary tests
    // -----------------------------------------------------------------------

    #[test]
    fn max_failure_ratio_exactly_at_boundary_does_not_fail() {
        // finalize_download uses strictly-greater: (failed / total) > MAX_FAILURE_RATIO
        // At exactly 50% failures (ratio == 0.5), the download should NOT be marked failed
        let success_count: usize = 50;
        let failed_count: usize = 50;
        let total = success_count + failed_count;
        let ratio = failed_count as f64 / total as f64;

        assert!(
            !(success_count == 0 || ratio > MAX_FAILURE_RATIO),
            "exactly 50% failures should NOT trigger failure (uses > not >=)"
        );
    }

    #[test]
    fn max_failure_ratio_just_above_boundary_fails() {
        let success_count: usize = 49;
        let failed_count: usize = 51;
        let total = success_count + failed_count;
        let ratio = failed_count as f64 / total as f64;

        assert!(
            success_count == 0 || ratio > MAX_FAILURE_RATIO,
            "51% failures should trigger failure"
        );
    }

    #[test]
    fn max_failure_ratio_all_failures_always_fails() {
        let success_count: usize = 0;
        let failed_count: usize = 100;

        // Even without checking the ratio, success_count == 0 triggers failure
        assert!(
            success_count == 0
                || (failed_count as f64 / (success_count + failed_count) as f64)
                    > MAX_FAILURE_RATIO,
            "zero successes should always trigger failure regardless of ratio"
        );
    }

    #[test]
    fn max_failure_ratio_single_failure_in_large_set_does_not_fail() {
        let success_count: usize = 999;
        let failed_count: usize = 1;
        let total = success_count + failed_count;
        let ratio = failed_count as f64 / total as f64;

        assert!(
            !(success_count == 0 || ratio > MAX_FAILURE_RATIO),
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
            crate::types::Event::DownloadComplete { id } => {
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
            crate::types::Event::DownloadFailed { id, error } => {
                assert_eq!(id, dl_id);
                assert_eq!(error, "batch fetch failed");
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
            pipeline_depth: 10,
        })
        .await;

        // Assert success
        let batch_results = result.unwrap();
        assert_eq!(batch_results.len(), 2);
        assert_eq!(batch_results[0], (1, 50));
        assert_eq!(batch_results[1], (2, 60));

        // Files exist on disk
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
        assert_eq!(
            downloaded_bytes.load(Ordering::Relaxed),
            110,
            "50 + 60 = 110 bytes"
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
            if matches!(event, crate::types::Event::DownloadComplete { id } if id == dl_id) {
                found_complete = true;
            }
        }
        assert!(
            found_complete,
            "DownloadComplete event should have been emitted"
        );
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
            if matches!(event, crate::types::Event::DownloadComplete { id } if id == dl_id) {
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
