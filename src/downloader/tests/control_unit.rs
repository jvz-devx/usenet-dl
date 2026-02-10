use crate::downloader::test_helpers::{SAMPLE_NZB, create_test_downloader};
use crate::error::{DownloadError, Error};
use crate::types::{DownloadId, DownloadOptions, Event, Priority, Status};

// --- pause() tests ---

#[tokio::test]
async fn test_pause_queued_download_sets_status_to_paused() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Verify initial status is Queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());

    downloader.pause(id).await.unwrap();

    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        download.status,
        Status::Paused.to_i32(),
        "pausing a Queued download should set DB status to Paused"
    );
}

#[tokio::test]
async fn test_pause_downloading_triggers_cancellation_token() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Simulate an active download by inserting a cancellation token
    let cancel_token = tokio_util::sync::CancellationToken::new();
    let token_clone = cancel_token.clone();
    downloader
        .queue_state
        .active_downloads
        .lock()
        .await
        .insert(id, cancel_token);

    // Set status to Downloading in DB
    downloader
        .db
        .update_status(id, Status::Downloading.to_i32())
        .await
        .unwrap();

    downloader.pause(id).await.unwrap();

    assert!(
        token_clone.is_cancelled(),
        "cancellation token should be triggered when pausing an active download"
    );

    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        download.status,
        Status::Paused.to_i32(),
        "status should be Paused after pausing an active download"
    );

    // Active downloads map should no longer contain this download
    let active = downloader.queue_state.active_downloads.lock().await;
    assert!(
        !active.contains_key(&id),
        "download should be removed from active_downloads map"
    );
}

#[tokio::test]
async fn test_pause_already_paused_is_idempotent() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader.pause(id).await.unwrap();

    // Pausing again should succeed silently
    let result = downloader.pause(id).await;
    assert!(
        result.is_ok(),
        "pausing an already-paused download should be idempotent"
    );

    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        download.status,
        Status::Paused.to_i32(),
        "status should remain Paused"
    );
}

#[tokio::test]
async fn test_pause_complete_download_returns_invalid_state() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader
        .db
        .update_status(id, Status::Complete.to_i32())
        .await
        .unwrap();

    let result = downloader.pause(id).await;

    match result {
        Err(Error::Download(DownloadError::InvalidState {
            id: err_id,
            operation,
            current_state,
        })) => {
            assert_eq!(
                err_id, id.0,
                "error should reference the correct download ID"
            );
            assert_eq!(
                operation, "pause",
                "error should specify 'pause' as the operation"
            );
            assert!(
                current_state.contains("Complete"),
                "error should report current state as Complete, got: {}",
                current_state
            );
        }
        other => panic!(
            "expected InvalidState error for pausing Complete download, got: {:?}",
            other
        ),
    }

    // Status should be unchanged
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        download.status,
        Status::Complete.to_i32(),
        "status should remain Complete after failed pause"
    );
}

#[tokio::test]
async fn test_pause_failed_download_returns_invalid_state() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader
        .db
        .update_status(id, Status::Failed.to_i32())
        .await
        .unwrap();

    let result = downloader.pause(id).await;

    match result {
        Err(Error::Download(DownloadError::InvalidState {
            id: err_id,
            operation,
            current_state,
        })) => {
            assert_eq!(err_id, id.0);
            assert_eq!(operation, "pause");
            assert!(
                current_state.contains("Failed"),
                "error should report current state as Failed, got: {}",
                current_state
            );
        }
        other => panic!(
            "expected InvalidState error for pausing Failed download, got: {:?}",
            other
        ),
    }
}

// --- resume() tests ---

#[tokio::test]
async fn test_resume_paused_download_sets_queued_and_adds_to_queue() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader.pause(id).await.unwrap();

    // Queue should be empty after pause
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 0, "queue should be empty after pause");

    downloader.resume(id).await.unwrap();

    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        download.status,
        Status::Queued.to_i32(),
        "resumed download should have status Queued"
    );

    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(
        queue_size, 1,
        "resumed download should be added back to the queue"
    );
}

#[tokio::test]
async fn test_resume_queued_download_is_idempotent() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Download starts as Queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());

    // Resuming a Queued download should return Ok
    let result = downloader.resume(id).await;
    assert!(
        result.is_ok(),
        "resuming a Queued download should be idempotent"
    );

    // Status should still be Queued, not double-queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());
}

#[tokio::test]
async fn test_resume_complete_download_returns_invalid_state() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader
        .db
        .update_status(id, Status::Complete.to_i32())
        .await
        .unwrap();

    let result = downloader.resume(id).await;

    match result {
        Err(Error::Download(DownloadError::InvalidState {
            id: err_id,
            operation,
            current_state,
        })) => {
            assert_eq!(err_id, id.0);
            assert_eq!(operation, "resume");
            assert!(
                current_state.contains("Complete"),
                "error should report Complete state, got: {}",
                current_state
            );
        }
        other => panic!(
            "expected InvalidState for resuming Complete download, got: {:?}",
            other
        ),
    }
}

// --- cancel() tests ---

#[tokio::test]
async fn test_cancel_removes_from_db_and_emits_removed_event() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    let mut events = downloader.subscribe();

    downloader.cancel(id).await.unwrap();

    // Download should be gone from DB
    let download = downloader.db.get_download(id).await.unwrap();
    assert!(
        download.is_none(),
        "cancelled download should be removed from DB"
    );

    // Queue should be empty
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 0, "queue should be empty after cancel");

    // Should receive Removed event
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            match events.recv().await {
                Ok(Event::Removed { id: event_id }) => return event_id,
                Ok(_) => continue,
                Err(_) => panic!("event channel closed without Removed event"),
            }
        }
    })
    .await
    .expect("timed out waiting for Removed event");

    assert_eq!(
        event, id,
        "Removed event should carry the cancelled download's ID"
    );
}

#[tokio::test]
async fn test_cancel_deletes_temp_directory() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Create simulated temp directory with a file
    let download_temp_dir = downloader
        .config
        .download
        .temp_dir
        .join(format!("download_{}", id));
    tokio::fs::create_dir_all(&download_temp_dir).await.unwrap();
    let test_file = download_temp_dir.join("article_1.dat");
    tokio::fs::write(&test_file, b"test data").await.unwrap();
    assert!(download_temp_dir.exists());

    downloader.cancel(id).await.unwrap();

    assert!(
        !download_temp_dir.exists(),
        "temp directory should be deleted after cancel"
    );
}

#[tokio::test]
async fn test_cancel_nonexistent_download_returns_not_found() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let result = downloader.cancel(DownloadId(99999)).await;

    match result {
        Err(Error::Database(crate::error::DatabaseError::NotFound(msg))) => {
            assert!(
                msg.contains("99999"),
                "error message should contain the ID, got: {}",
                msg
            );
        }
        other => panic!("expected NotFound error, got: {:?}", other),
    }
}

// --- set_priority() tests ---

#[tokio::test]
async fn test_set_priority_on_queued_download_reorders_queue() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id_normal = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "normal", DownloadOptions::default())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let id_low = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "low",
            DownloadOptions {
                priority: Priority::Low,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Before priority change, normal should come first
    {
        let queue = downloader.queue_state.queue.lock().await;
        let first = queue.peek().unwrap();
        assert_eq!(
            first.id, id_normal,
            "normal should be first before priority change"
        );
    }

    // Upgrade the low download to High
    downloader
        .set_priority(id_low, Priority::High)
        .await
        .unwrap();

    // After priority change, the formerly-low download should be first
    {
        let mut queue = downloader.queue_state.queue.lock().await;
        let first = queue.pop().unwrap();
        assert_eq!(
            first.id, id_low,
            "download upgraded to High should now be first in queue"
        );
        assert_eq!(
            first.priority,
            Priority::High,
            "queue entry should reflect the new priority"
        );
    }

    // Verify DB was updated
    let download = downloader.db.get_download(id_low).await.unwrap().unwrap();
    assert_eq!(
        download.priority,
        Priority::High as i32,
        "priority should be updated in DB"
    );
}

// --- pause_all() tests ---

#[tokio::test]
async fn test_pause_all_pauses_queued_skips_paused_complete_failed() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create downloads in various states
    let id_queued = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "queued", DownloadOptions::default())
        .await
        .unwrap();

    let id_paused = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "paused", DownloadOptions::default())
        .await
        .unwrap();
    downloader.pause(id_paused).await.unwrap();

    let id_complete = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "complete",
            DownloadOptions::default(),
        )
        .await
        .unwrap();
    downloader
        .db
        .update_status(id_complete, Status::Complete.to_i32())
        .await
        .unwrap();

    let id_failed = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "failed", DownloadOptions::default())
        .await
        .unwrap();
    downloader
        .db
        .update_status(id_failed, Status::Failed.to_i32())
        .await
        .unwrap();

    downloader.pause_all().await.unwrap();

    let d_queued = downloader
        .db
        .get_download(id_queued)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        d_queued.status,
        Status::Paused.to_i32(),
        "Queued download should be paused by pause_all"
    );

    let d_paused = downloader
        .db
        .get_download(id_paused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        d_paused.status,
        Status::Paused.to_i32(),
        "already-Paused download should remain Paused"
    );

    let d_complete = downloader
        .db
        .get_download(id_complete)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        d_complete.status,
        Status::Complete.to_i32(),
        "Complete download should not be affected by pause_all"
    );

    let d_failed = downloader
        .db
        .get_download(id_failed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        d_failed.status,
        Status::Failed.to_i32(),
        "Failed download should not be affected by pause_all"
    );
}

// --- resume_all() tests ---

#[tokio::test]
async fn test_resume_all_resumes_only_paused_and_emits_queue_resumed() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "paused1", DownloadOptions::default())
        .await
        .unwrap();
    downloader.pause(id1).await.unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "paused2", DownloadOptions::default())
        .await
        .unwrap();
    downloader.pause(id2).await.unwrap();

    let id_complete = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "complete",
            DownloadOptions::default(),
        )
        .await
        .unwrap();
    downloader
        .db
        .update_status(id_complete, Status::Complete.to_i32())
        .await
        .unwrap();

    let mut events = downloader.subscribe();

    downloader.resume_all().await.unwrap();

    // Paused downloads should now be Queued
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    assert_eq!(
        d1.status,
        Status::Queued.to_i32(),
        "paused download should be resumed to Queued"
    );

    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(
        d2.status,
        Status::Queued.to_i32(),
        "paused download should be resumed to Queued"
    );

    // Complete download should be untouched
    let d_complete = downloader
        .db
        .get_download(id_complete)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        d_complete.status,
        Status::Complete.to_i32(),
        "Complete download should not be affected by resume_all"
    );

    // Should emit QueueResumed event
    let received = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            match events.recv().await {
                Ok(Event::QueueResumed) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await
    .expect("timed out waiting for QueueResumed event");

    assert!(received, "resume_all should emit QueueResumed event");
}
