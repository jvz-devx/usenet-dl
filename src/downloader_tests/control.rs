use super::*;

#[tokio::test]
async fn test_pause_queued_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add download
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Download should be queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());

    // Pause it
    downloader.pause(id).await.unwrap();

    // Status should be updated to Paused
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32());
}

#[tokio::test]
async fn test_pause_already_paused() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Pause it once
    downloader.pause(id).await.unwrap();

    // Pause it again (should be idempotent)
    let result = downloader.pause(id).await;
    assert!(result.is_ok());

    // Status should still be Paused
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32());
}

#[tokio::test]
async fn test_pause_completed_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark as complete
    downloader
        .db
        .update_status(id, Status::Complete.to_i32())
        .await
        .unwrap();

    // Try to pause (should fail)
    let result = downloader.pause(id).await;
    assert!(result.is_err());

    // Status should still be Complete
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Complete.to_i32());
}

#[tokio::test]
async fn test_pause_nonexistent_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Try to pause download that doesn't exist
    let result = downloader.pause(DownloadId(999)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_resume_paused_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add download
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Pause it
    downloader.pause(id).await.unwrap();
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32());

    // Resume it
    downloader.resume(id).await.unwrap();

    // Status should be updated to Queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());

    // Should be back in the queue
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert!(queue_size > 0);
}

#[tokio::test]
async fn test_resume_already_queued() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Download is already queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());

    // Try to resume (should be idempotent)
    let result = downloader.resume(id).await;
    assert!(result.is_ok());

    // Status should still be Queued
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());
}

#[tokio::test]
async fn test_resume_completed_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark as complete
    downloader
        .db
        .update_status(id, Status::Complete.to_i32())
        .await
        .unwrap();

    // Try to resume (should fail)
    let result = downloader.resume(id).await;
    assert!(result.is_err());

    // Status should still be Complete
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Complete.to_i32());
}

#[tokio::test]
async fn test_resume_failed_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark as failed
    downloader
        .db
        .update_status(id, Status::Failed.to_i32())
        .await
        .unwrap();

    // Try to resume (should fail - use reprocess() instead for failed downloads)
    let result = downloader.resume(id).await;
    assert!(result.is_err());

    // Status should still be Failed
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Failed.to_i32());
}

#[tokio::test]
async fn test_resume_nonexistent_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Try to resume download that doesn't exist
    let result = downloader.resume(DownloadId(999)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_pause_resume_cycle() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add download
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    let initial_queue_size = downloader.queue_state.queue.lock().await.len();

    // Pause
    downloader.pause(id).await.unwrap();
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32());

    // Resume
    downloader.resume(id).await.unwrap();
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Queued.to_i32());

    // Queue size should be restored
    let final_queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(final_queue_size, initial_queue_size);
}

#[tokio::test]
async fn test_resume_preserves_priority() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add high priority download
    let id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test",
            DownloadOptions {
                priority: Priority::High,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Add normal priority download
    let normal_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "normal", DownloadOptions::default())
        .await
        .unwrap();

    // Pause high priority download
    downloader.pause(id).await.unwrap();

    // Resume high priority download
    downloader.resume(id).await.unwrap();

    // High priority download should still come first
    let first = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(first, Some(id));

    let second = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(second, Some(normal_id));
}

#[tokio::test]
async fn test_cancel_queued_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Verify download exists in database
    assert!(downloader.db.get_download(id).await.unwrap().is_some());

    // Verify download is in queue
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 1);

    // Cancel the download
    downloader.cancel(id).await.unwrap();

    // Download should be removed from database
    assert!(downloader.db.get_download(id).await.unwrap().is_none());

    // Download should be removed from queue
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 0);
}

#[tokio::test]
async fn test_cancel_paused_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Pause the download
    downloader.pause(id).await.unwrap();

    // Verify it's paused
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, Status::Paused.to_i32());

    // Cancel the paused download
    downloader.cancel(id).await.unwrap();

    // Download should be removed from database
    assert!(downloader.db.get_download(id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_cancel_deletes_temp_files() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Create temp directory and some files (simulating partially downloaded)
    let download_temp_dir = downloader
        .config
        .download
        .temp_dir
        .join(format!("download_{}", id));
    tokio::fs::create_dir_all(&download_temp_dir).await.unwrap();

    let test_file = download_temp_dir.join("article_1.dat");
    tokio::fs::write(&test_file, b"test data").await.unwrap();

    // Verify temp directory exists
    assert!(download_temp_dir.exists());
    assert!(test_file.exists());

    // Cancel the download
    downloader.cancel(id).await.unwrap();

    // Temp directory should be deleted
    assert!(!download_temp_dir.exists());
    assert!(!test_file.exists());
}

#[tokio::test]
async fn test_cancel_nonexistent_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Try to cancel download that doesn't exist
    let result = downloader.cancel(DownloadId(999)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_cancel_completed_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark as completed
    downloader
        .db
        .update_status(id, Status::Complete.to_i32())
        .await
        .unwrap();

    // Cancel completed download (should succeed - removes from history)
    downloader.cancel(id).await.unwrap();

    // Download should be removed from database
    assert!(downloader.db.get_download(id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_cancel_removes_from_queue() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
        .await
        .unwrap();

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
        .await
        .unwrap();

    // Verify all are queued
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 3);

    // Cancel middle download
    downloader.cancel(id2).await.unwrap();

    // Queue should have 2 items
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 2);

    // Get downloads from queue - should only be id1 and id3
    let next = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert!(next == Some(id1) || next == Some(id3));

    let next2 = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert!(next2 == Some(id1) || next2 == Some(id3));
    assert_ne!(next, next2);

    // Queue should now be empty
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 0);
}

#[tokio::test]
async fn test_cancel_emits_removed_event() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Cancel the download (in background to avoid blocking)
    let downloader_clone = downloader.clone();
    tokio::spawn(async move {
        downloader_clone.cancel(id).await.unwrap();
    });

    // Wait for Removed event
    let mut received_removed = false;
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await {
            Ok(Ok(crate::types::Event::Removed { id: event_id })) => {
                assert_eq!(event_id, id);
                received_removed = true;
                break;
            }
            Ok(Ok(_)) => continue, // Other events, keep checking
            Ok(Err(_)) => break,   // Channel closed
            Err(_) => break,       // Timeout
        }
    }

    assert!(received_removed, "Should have received Removed event");
}

// Queue-wide pause/resume tests

#[tokio::test]
async fn test_pause_all_pauses_active_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads with different statuses
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
        .await
        .unwrap();

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
        .await
        .unwrap();

    // Mark id2 as already paused
    downloader.pause(id2).await.unwrap();

    // Mark id3 as complete (should not be paused)
    downloader
        .db
        .update_status(id3, Status::Complete.to_i32())
        .await
        .unwrap();

    // Pause all
    downloader.pause_all().await.unwrap();

    // Check statuses
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    let d3 = downloader.db.get_download(id3).await.unwrap().unwrap();

    assert_eq!(d1.status, Status::Paused.to_i32(), "id1 should be paused");
    assert_eq!(
        d2.status,
        Status::Paused.to_i32(),
        "id2 should still be paused"
    );
    assert_eq!(
        d3.status,
        Status::Complete.to_i32(),
        "id3 should still be complete"
    );
}

#[tokio::test]
async fn test_pause_all_emits_queue_paused_event() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Pause all (in background to avoid blocking)
    let downloader_clone = downloader.clone();
    tokio::spawn(async move {
        downloader_clone.pause_all().await.unwrap();
    });

    // Wait for QueuePaused event
    let mut received_queue_paused = false;
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await {
            Ok(Ok(crate::types::Event::QueuePaused)) => {
                received_queue_paused = true;
                break;
            }
            Ok(Ok(_)) => continue, // Other events, keep checking
            Ok(Err(_)) => break,   // Channel closed
            Err(_) => break,       // Timeout
        }
    }

    assert!(
        received_queue_paused,
        "Should have received QueuePaused event"
    );
}

#[tokio::test]
async fn test_pause_all_with_empty_queue() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Pause all with no downloads (should not error)
    let result = downloader.pause_all().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_resume_all_resumes_paused_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
        .await
        .unwrap();

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test3", DownloadOptions::default())
        .await
        .unwrap();

    // Pause all downloads
    downloader.pause(id1).await.unwrap();
    downloader.pause(id2).await.unwrap();

    // Mark id3 as complete (should not be resumed)
    downloader
        .db
        .update_status(id3, Status::Complete.to_i32())
        .await
        .unwrap();

    // Resume all
    downloader.resume_all().await.unwrap();

    // Check statuses
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    let d3 = downloader.db.get_download(id3).await.unwrap().unwrap();

    assert_eq!(d1.status, Status::Queued.to_i32(), "id1 should be queued");
    assert_eq!(d2.status, Status::Queued.to_i32(), "id2 should be queued");
    assert_eq!(
        d3.status,
        Status::Complete.to_i32(),
        "id3 should still be complete"
    );
}

#[tokio::test]
async fn test_resume_all_emits_queue_resumed_event() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add and pause a download
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader.pause(id).await.unwrap();

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Resume all (in background to avoid blocking)
    let downloader_clone = downloader.clone();
    tokio::spawn(async move {
        downloader_clone.resume_all().await.unwrap();
    });

    // Wait for QueueResumed event
    let mut received_queue_resumed = false;
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await {
            Ok(Ok(crate::types::Event::QueueResumed)) => {
                received_queue_resumed = true;
                break;
            }
            Ok(Ok(_)) => continue, // Other events, keep checking
            Ok(Err(_)) => break,   // Channel closed
            Err(_) => break,       // Timeout
        }
    }

    assert!(
        received_queue_resumed,
        "Should have received QueueResumed event"
    );
}

#[tokio::test]
async fn test_resume_all_with_no_paused_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a queued download (not paused)
    downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Resume all (should not error even though nothing is paused)
    let result = downloader.resume_all().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_pause_all_resume_all_cycle() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
        .await
        .unwrap();

    // Initial state: both queued
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(d1.status, Status::Queued.to_i32());
    assert_eq!(d2.status, Status::Queued.to_i32());

    // Pause all
    downloader.pause_all().await.unwrap();

    // After pause: both paused
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(d1.status, Status::Paused.to_i32());
    assert_eq!(d2.status, Status::Paused.to_i32());

    // Resume all
    downloader.resume_all().await.unwrap();

    // After resume: both queued again
    let d1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    let d2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(d1.status, Status::Queued.to_i32());
    assert_eq!(d2.status, Status::Queued.to_i32());
}
