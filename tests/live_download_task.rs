#![cfg(feature = "live-tests")]

//! Live integration tests for the download pipeline.
//!
//! These tests connect to a real NNTP provider and exercise the full download
//! flow: NZB ingestion → queue processing → article fetching → failure handling.
//!
//! Gated behind the `live-tests` feature flag. Requires NNTP credentials in `.env`.
//!
//! ```bash
//! cargo test --features live-tests --test live_download_task -- --nocapture
//! ```

mod common;

use std::time::Duration;
use usenet_dl::{DownloadOptions, Event, Priority};

/// Test the full download pipeline with a small NZB containing fake message IDs.
///
/// With fake IDs, the download will fail at the article-fetch stage — which is
/// exactly what we want to verify: that the pipeline runs end-to-end and emits
/// the expected failure events rather than hanging or panicking.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_download_pipeline_small_nzb() {
    skip_if_no_credentials!();

    let (downloader, _temp_dir) = common::create_live_downloader()
        .await
        .expect("Failed to create live downloader");

    let mut events = downloader.subscribe();

    // Ingest a minimal NZB (fake message IDs → will fail on fetch)
    let id = downloader
        .add_nzb_content(
            common::MINIMAL_NZB.as_bytes(),
            "live_pipeline_test",
            DownloadOptions {
                priority: Priority::Normal,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to add NZB");

    // Start the queue processor so the download is picked up
    let processor = downloader.start_queue_processor();

    // Collect events until we see a terminal state for our download
    let mut saw_downloading = false;
    let mut terminal_event = None;

    let _ = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match events.recv().await {
                Ok(Event::Downloading { id: eid, .. }) if eid == id => {
                    saw_downloading = true;
                }
                Ok(Event::DownloadFailed { id: eid, error, .. }) if eid == id => {
                    terminal_event = Some(format!("DownloadFailed: {}", error));
                    return;
                }
                Ok(Event::DownloadComplete { id: eid, .. }) if eid == id => {
                    terminal_event = Some("DownloadComplete".to_string());
                    return;
                }
                Ok(_) => continue,
                Err(_) => return,
            }
        }
    })
    .await;

    // The pipeline should have reached the Downloading state
    assert!(
        saw_downloading,
        "Expected a Downloading event for download {}",
        id
    );

    // With fake message IDs, we expect failure (article not found on server)
    assert!(
        terminal_event.is_some(),
        "Expected a terminal event (DownloadFailed or DownloadComplete) for download {}",
        id
    );

    processor.abort();
    downloader.shutdown().await.ok();
}

/// Test that multiple downloads with different priorities are processed in order
/// through the live NNTP pipeline.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_priority_ordering_through_pipeline() {
    skip_if_no_credentials!();

    let (downloader, _temp_dir) = common::create_live_downloader()
        .await
        .expect("Failed to create live downloader");

    let mut events = downloader.subscribe();

    // Add two downloads: low priority first, then force priority
    let low_id = downloader
        .add_nzb_content(
            common::MINIMAL_NZB.as_bytes(),
            "low_priority_test",
            DownloadOptions {
                priority: Priority::Low,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to add low-priority NZB");

    let force_id = downloader
        .add_nzb_content(
            common::MULTI_SEGMENT_NZB.as_bytes(),
            "force_priority_test",
            DownloadOptions {
                priority: Priority::Force,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to add force-priority NZB");

    let processor = downloader.start_queue_processor();

    // Collect the first Downloading event per download ID to observe ordering
    let mut first_downloading = Vec::new();

    let _ = tokio::time::timeout(Duration::from_secs(30), async {
        while first_downloading.len() < 2 {
            if let Ok(Event::Downloading { id, .. }) = events.recv().await {
                if !first_downloading.contains(&id) {
                    first_downloading.push(id);
                }
            }
        }
    })
    .await;

    // Force priority should be processed before low priority
    if first_downloading.len() >= 2 {
        assert_eq!(
            first_downloading[0], force_id,
            "Force-priority download should start first, got order: {:?}",
            first_downloading
        );
        assert_eq!(
            first_downloading[1], low_id,
            "Low-priority download should start second"
        );
    } else {
        // At minimum, at least one download should have started
        assert!(
            !first_downloading.is_empty(),
            "Expected at least one Downloading event"
        );
    }

    processor.abort();
    downloader.shutdown().await.ok();
}
