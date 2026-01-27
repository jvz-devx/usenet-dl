//! Background tasks for progress reporting and database batch updates.

use crate::types::{DownloadId, Event};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Buffer size for the article status update channel
pub(crate) const ARTICLE_CHANNEL_BUFFER: usize = 500;

/// Interval between progress update emissions
const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

/// Batch size threshold for flushing article status updates to the database
const ARTICLE_BATCH_SIZE: usize = 100;

/// Parameters for spawning a progress reporter background task
pub(crate) struct ProgressReporterParams {
    /// Download ID
    pub id: DownloadId,
    /// Total number of articles
    pub total_articles: usize,
    /// Total size in bytes
    pub total_size_bytes: u64,
    /// Download start time
    pub download_start: std::time::Instant,
    /// Atomic counter for downloaded articles
    pub downloaded_articles: Arc<AtomicU64>,
    /// Atomic counter for downloaded bytes
    pub downloaded_bytes: Arc<AtomicU64>,
    /// Event broadcast sender
    pub event_tx: tokio::sync::broadcast::Sender<Event>,
    /// Database handle
    pub db: Arc<crate::db::Database>,
    /// Cancellation token
    pub cancel_token: tokio_util::sync::CancellationToken,
}

/// Spawn a background task that periodically reports download progress.
pub(crate) fn spawn_progress_reporter(params: ProgressReporterParams) -> tokio::task::JoinHandle<()> {
    let ProgressReporterParams {
        id,
        total_articles,
        total_size_bytes,
        download_start,
        downloaded_articles,
        downloaded_bytes,
        event_tx,
        db,
        cancel_token,
    } = params;
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
                        tracing::error!(download_id = id.0, error = %e, "Failed to update progress");
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
pub(crate) fn spawn_batch_updater(
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
                            tracing::error!(download_id = id.0, batch_size = buffer.len(), error = %e, "Failed to batch update article statuses");
                        }
                        buffer.clear();
                    }
                }
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        if let Err(e) = db.update_articles_status_batch(&buffer).await {
                            tracing::error!(download_id = id.0, batch_size = buffer.len(), error = %e, "Failed to batch update article statuses");
                        }
                        buffer.clear();
                    }
                }
                _ = cancel_token.cancelled() => {
                    if !buffer.is_empty() {
                        if let Err(e) = db.update_articles_status_batch(&buffer).await {
                            tracing::error!(download_id = id.0, batch_size = buffer.len(), error = %e, "Failed to batch update article statuses on cancellation");
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
                tracing::error!(download_id = id.0, batch_size = buffer.len(), error = %e, "Failed to flush remaining article statuses");
            }
        }
    })
}
