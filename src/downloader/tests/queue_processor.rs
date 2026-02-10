use super::*;
use crate::db::NewDownload;
use crate::downloader::QueuedDownload;
use crate::downloader::test_helpers::create_test_downloader;
use crate::types::{DownloadId, Event, Priority, Status};

/// Insert a download record into the DB and return its ID.
async fn insert_test_download(
    downloader: &UsenetDownloader,
    name: &str,
    priority: Priority,
) -> DownloadId {
    downloader
        .db
        .insert_download(&NewDownload {
            name: name.to_string(),
            nzb_path: "/tmp/test.nzb".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/tmp/out".to_string(),
            post_process: 0,
            priority: priority as i32,
            status: Status::Queued.to_i32(),
            size_bytes: 1000,
        })
        .await
        .unwrap()
}

/// Push a QueuedDownload into the in-memory priority queue.
async fn push_to_queue(
    downloader: &UsenetDownloader,
    id: DownloadId,
    priority: Priority,
    created_at: i64,
) {
    let mut queue = downloader.queue_state.queue.lock().await;
    queue.push(QueuedDownload {
        id,
        priority,
        created_at,
    });
}

// -----------------------------------------------------------------------
// dequeues_in_priority_order
// -----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dequeues_in_priority_order() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Use max_concurrent=1 to serialize downloads so we can observe ordering
    let downloader = {
        let mut d = downloader;
        d.queue_state.concurrent_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        d
    };

    // Insert DB records for 3 downloads with different priorities
    let normal_id = insert_test_download(&downloader, "normal", Priority::Normal).await;
    let force_id = insert_test_download(&downloader, "force", Priority::Force).await;
    let high_id = insert_test_download(&downloader, "high", Priority::High).await;

    // Push to queue: Force should dequeue first, then High, then Normal
    push_to_queue(&downloader, normal_id, Priority::Normal, 1).await;
    push_to_queue(&downloader, force_id, Priority::Force, 2).await;
    push_to_queue(&downloader, high_id, Priority::High, 3).await;

    // Subscribe to events BEFORE starting the processor
    let mut events = downloader.subscribe();

    // Start queue processor — downloads will fail immediately (no NNTP pools)
    let handle = downloader.start_queue_processor();

    // Collect the first Downloading event per unique download ID.
    // With max_concurrent=1 and no NNTP pools, each download runs serially:
    // dequeue → Downloading → DownloadFailed → (next download)
    let mut event_ids = Vec::new();

    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while event_ids.len() < 3 {
            if let Ok(Event::Downloading { id, .. }) = events.recv().await
                && !event_ids.contains(&id)
            {
                event_ids.push(id);
            }
        }
    })
    .await;

    assert!(
        event_ids.len() >= 3,
        "Expected 3 Downloading events, got {}: {:?}",
        event_ids.len(),
        event_ids
    );

    // Verify ordering: Force > High > Normal
    assert_eq!(
        event_ids[0], force_id,
        "Force-priority download should be dequeued first"
    );
    assert_eq!(
        event_ids[1], high_id,
        "High-priority download should be dequeued second"
    );
    assert_eq!(
        event_ids[2], normal_id,
        "Normal-priority download should be dequeued third"
    );

    handle.abort();
}

// -----------------------------------------------------------------------
// respects_max_concurrent_downloads
// -----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn respects_max_concurrent_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Override to max 1 concurrent download
    let downloader = {
        let mut d = downloader;
        d.queue_state.concurrent_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        d
    };

    // Insert 2 download records
    let id1 = insert_test_download(&downloader, "first", Priority::Normal).await;
    let id2 = insert_test_download(&downloader, "second", Priority::Normal).await;

    // Push both, id1 with older timestamp so it dequeues first
    push_to_queue(&downloader, id1, Priority::Normal, 1).await;
    push_to_queue(&downloader, id2, Priority::Normal, 2).await;

    let mut events = downloader.subscribe();

    let handle = downloader.start_queue_processor();

    // Collect all events in order. With max_concurrent=1, the semaphore
    // ensures only one download task runs at a time. The first download
    // must fail (no NNTP pools) and release the permit before the second
    // can start. So we should see id1's Downloading before id2's Downloading.
    let mut event_order = Vec::new();

    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while event_order.len() < 4 {
            if let Ok(event) = events.recv().await {
                match &event {
                    Event::Downloading { id, .. } => {
                        event_order.push((*id, "downloading"));
                    }
                    Event::DownloadFailed { id, .. } => {
                        event_order.push((*id, "failed"));
                    }
                    _ => {}
                }
            }
        }
    })
    .await;

    // With max_concurrent=1, the ordering must be:
    // id1 downloading → id1 failed → id2 downloading → id2 failed
    // (id2 can't start downloading until id1's permit is released)
    assert!(
        event_order.len() >= 2,
        "Should have at least 2 events, got {:?}",
        event_order
    );

    // Find position of first Downloading for each ID
    let id1_start = event_order
        .iter()
        .position(|(id, ev)| *id == id1 && *ev == "downloading");
    let id2_start = event_order
        .iter()
        .position(|(id, ev)| *id == id2 && *ev == "downloading");
    let id1_end = event_order
        .iter()
        .position(|(id, ev)| *id == id1 && *ev == "failed");

    if let (Some(id1_s), Some(id1_e), Some(id2_s)) = (id1_start, id1_end, id2_start) {
        assert!(id1_s < id1_e, "id1 should start before it fails");
        assert!(
            id1_e < id2_s,
            "id1 should fail (releasing permit) before id2 starts downloading. \
             Events: {:?}",
            event_order
        );
    } else {
        // At minimum, verify the first download started
        assert!(
            id1_start.is_some(),
            "First download should have started. Events: {:?}",
            event_order
        );
    }

    handle.abort();
}

// -----------------------------------------------------------------------
// re_pushes_on_semaphore_close
// -----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn re_pushes_on_semaphore_close() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Close the semaphore BEFORE starting the processor
    downloader.queue_state.concurrent_limit.close();

    let id = insert_test_download(&downloader, "orphan", Priority::Normal).await;
    push_to_queue(&downloader, id, Priority::Normal, 1).await;

    // Start processor — it should pop the item, fail to acquire, re-push, and break
    let handle = downloader.start_queue_processor();

    // Wait for the processor task to finish (it should break out of the loop)
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "Queue processor should exit when semaphore is closed"
    );

    // Verify the item was re-pushed to the queue
    let queue_len = downloader.queue_state.queue.lock().await.len();
    assert_eq!(
        queue_len, 1,
        "Item should be re-pushed to queue when semaphore is closed"
    );

    // Verify it's the same item
    let item = downloader.queue_state.queue.lock().await.pop().unwrap();
    assert_eq!(item.id, id, "Re-pushed item should have the same ID");
}
