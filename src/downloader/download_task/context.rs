//! Download task context — shared state, article provider trait, and output file management.

use crate::types::{DownloadId, Event, Status};
use std::collections::HashMap;
use std::sync::Arc;

use super::super::UsenetDownloader;

/// Manages output file handles for DirectWrite -- one file per NZB file entry.
///
/// Uses positional writes (`pwrite` on Unix, `seek_write` on Windows) which take
/// `&self` (not `&mut self`), enabling lock-free concurrent writes from different
/// batches to the same file. Each segment writes to non-overlapping byte ranges
/// via yEnc part offsets.
pub(crate) struct OutputFiles {
    /// file_index -> (File handle, filename)
    pub(super) files: HashMap<i32, (std::fs::File, String)>,
}

impl OutputFiles {
    /// Create OutputFiles by pre-creating empty files for each download file entry.
    pub(super) fn create(
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
pub(super) fn is_missing_article_error(err: &nntp_rs::NntpError) -> bool {
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
pub(super) type BatchResultVec = Vec<std::result::Result<Vec<(i32, u64)>, (String, usize)>>;

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
    pub(super) async fn remove_from_active(&self) {
        let mut active = self.active_downloads.lock().await;
        active.remove(&self.id);
    }

    /// Mark the download as failed with an error message and emit the failure event.
    pub(super) async fn mark_failed(&self, error: &str) {
        self.mark_failed_with_stats(error, None, None, None).await;
    }

    /// Mark the download as failed with an error message and optional article stats.
    pub(super) async fn mark_failed_with_stats(
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
    pub(super) fn spawn_post_processing(self) {
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
