//! Download task orchestration — top-level lifecycle for a single download.

use std::collections::HashMap;
use std::sync::Arc;

use crate::types::Event;

use super::batching::{download_articles, fetch_download_record};
use super::context::{DownloadTaskContext, OutputFiles};
use super::finalization::finalize_download;

/// Aggregated result counts from downloading article batches.
pub(super) struct DownloadResults {
    pub(super) success_count: usize,
    pub(super) failed_count: usize,
    pub(super) first_error: Option<String>,
    /// Total articles in the download (for stats)
    pub(super) total_articles: usize,
    /// Count of individually-failed articles (tracked via atomic, separate from batch failures)
    pub(super) individually_failed: u64,
}

/// Core download task -- orchestrates the full lifecycle of a single download.
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
        // Legacy downloads without download_files rows -- no DirectWrite
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
