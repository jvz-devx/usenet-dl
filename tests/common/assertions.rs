//! Custom test assertions for E2E tests

use std::path::Path;
use std::time::Duration;
use usenet_dl::{DownloadId, Event, Status, UsenetDownloader};

/// Result of waiting for download completion
#[derive(Debug)]
pub enum WaitResult {
    /// Download completed successfully
    Completed,
    /// Download failed with error
    Failed(String),
    /// Timeout waiting for completion
    Timeout,
    /// Channel closed unexpectedly
    ChannelClosed,
}

/// Wait for a download to reach a terminal state (Complete or Failed)
///
/// # Arguments
/// * `downloader` - The downloader instance
/// * `id` - Download ID to wait for
/// * `timeout` - Maximum time to wait
///
/// # Returns
/// `WaitResult` indicating the outcome
pub async fn wait_for_completion(
    downloader: &UsenetDownloader,
    id: DownloadId,
    timeout: Duration,
) -> WaitResult {
    let mut events = downloader.subscribe();

    let result = tokio::time::timeout(timeout, async {
        loop {
            match events.recv().await {
                Ok(Event::Complete { id: event_id, .. }) if event_id == id => {
                    return WaitResult::Completed;
                }
                Ok(Event::Failed {
                    id: event_id,
                    error,
                    ..
                }) if event_id == id => {
                    return WaitResult::Failed(error);
                }
                Ok(_) => {
                    // Other events, continue waiting
                    continue;
                }
                Err(_) => {
                    return WaitResult::ChannelClosed;
                }
            }
        }
    })
    .await;

    match result {
        Ok(wait_result) => wait_result,
        Err(_) => WaitResult::Timeout,
    }
}

/// Wait for download to start (status changes to Downloading)
pub async fn wait_for_downloading(
    downloader: &UsenetDownloader,
    id: DownloadId,
    timeout: Duration,
) -> bool {
    let mut events = downloader.subscribe();

    let result = tokio::time::timeout(timeout, async {
        loop {
            match events.recv().await {
                Ok(Event::Downloading { id: event_id, .. }) if event_id == id => {
                    return true;
                }
                Ok(Event::Failed { id: event_id, .. }) if event_id == id => {
                    return false;
                }
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await;

    result.unwrap_or(false)
}

/// Wait for a specific event type
pub async fn wait_for_event<F>(
    downloader: &UsenetDownloader,
    timeout: Duration,
    predicate: F,
) -> Option<Event>
where
    F: Fn(&Event) -> bool,
{
    let mut events = downloader.subscribe();

    let result = tokio::time::timeout(timeout, async {
        loop {
            match events.recv().await {
                Ok(event) if predicate(&event) => {
                    return Some(event);
                }
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    })
    .await;

    result.ok().flatten()
}

/// Collect all events until timeout or predicate is satisfied
pub async fn collect_events_until<F>(
    downloader: &UsenetDownloader,
    timeout: Duration,
    stop_predicate: F,
) -> Vec<Event>
where
    F: Fn(&Event) -> bool,
{
    let mut events = downloader.subscribe();
    let mut collected = Vec::new();

    let _ = tokio::time::timeout(timeout, async {
        while let Ok(event) = events.recv().await {
            let should_stop = stop_predicate(&event);
            collected.push(event);
            if should_stop {
                break;
            }
        }
    })
    .await;

    collected
}

/// Assert that a download completed successfully
pub async fn assert_download_completed(
    downloader: &UsenetDownloader,
    id: DownloadId,
    timeout: Duration,
) {
    match wait_for_completion(downloader, id, timeout).await {
        WaitResult::Completed => {}
        WaitResult::Failed(error) => {
            panic!("Download {} failed with error: {}", id, error);
        }
        WaitResult::Timeout => {
            panic!("Timeout waiting for download {} to complete", id);
        }
        WaitResult::ChannelClosed => {
            panic!("Event channel closed while waiting for download {}", id);
        }
    }
}

/// Assert that a download failed with expected error
pub async fn assert_download_failed(
    downloader: &UsenetDownloader,
    id: DownloadId,
    timeout: Duration,
    expected_error_contains: Option<&str>,
) {
    match wait_for_completion(downloader, id, timeout).await {
        WaitResult::Failed(error) => {
            if let Some(expected) = expected_error_contains {
                assert!(
                    error.contains(expected),
                    "Expected error to contain '{}', got: {}",
                    expected,
                    error
                );
            }
        }
        WaitResult::Completed => {
            panic!("Expected download {} to fail, but it completed", id);
        }
        WaitResult::Timeout => {
            panic!("Timeout waiting for download {} to fail", id);
        }
        WaitResult::ChannelClosed => {
            panic!("Event channel closed while waiting for download {}", id);
        }
    }
}

/// Assert that files exist in the download directory
pub fn assert_files_exist(dir: &Path, expected_files: &[&str]) {
    for filename in expected_files {
        let path = dir.join(filename);
        assert!(
            path.exists(),
            "Expected file '{}' to exist in {:?}",
            filename,
            dir
        );
    }
}

/// Assert that a directory is not empty
pub fn assert_dir_not_empty(dir: &Path) {
    assert!(dir.exists(), "Directory {:?} does not exist", dir);
    let entries: Vec<_> = std::fs::read_dir(dir)
        .expect("Failed to read directory")
        .collect();
    assert!(
        !entries.is_empty(),
        "Expected directory {:?} to contain files, but it's empty",
        dir
    );
}

/// Assert download has expected status
pub async fn assert_download_status(
    downloader: &UsenetDownloader,
    id: DownloadId,
    expected_status: Status,
) {
    let downloads = downloader.db.list_downloads().await.unwrap_or_default();
    let download = downloads.iter().find(|d| d.id == id);

    match download {
        Some(d) => {
            assert_eq!(
                Status::from_i32(d.status),
                expected_status,
                "Expected download {} to have status {:?}, got {:?}",
                id,
                expected_status,
                Status::from_i32(d.status)
            );
        }
        None => {
            panic!("Download {} not found", id);
        }
    }
}

/// Assert download progress is within expected range
pub async fn assert_progress_in_range(
    downloader: &UsenetDownloader,
    id: DownloadId,
    min_percent: f32,
    max_percent: f32,
) {
    let downloads = downloader.db.list_downloads().await.unwrap_or_default();
    let download = downloads.iter().find(|d| d.id == id);

    match download {
        Some(d) => {
            assert!(
                d.progress >= min_percent && d.progress <= max_percent,
                "Expected download {} progress to be between {}% and {}%, got {}%",
                id,
                min_percent,
                max_percent,
                d.progress
            );
        }
        None => {
            panic!("Download {} not found", id);
        }
    }
}
