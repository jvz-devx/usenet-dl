use super::*;

#[tokio::test]
async fn test_queue_adds_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add download
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Verify it's in the queue
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 1);

    // Verify we can get it from the queue
    let next_id = {
        let queue = downloader.queue_state.queue.lock().await;
        queue.peek().map(|item| item.id)
    };
    assert_eq!(next_id, Some(id));
}

#[tokio::test]
async fn test_queue_priority_ordering() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add downloads with different priorities
    let low_id = downloader
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

    let high_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "high",
            DownloadOptions {
                priority: Priority::High,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let normal_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "normal",
            DownloadOptions {
                priority: Priority::Normal,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Queue should have 3 items
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 3);

    // Should return highest priority first (High > Normal > Low)
    let high_result = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(high_result, Some(high_id));

    let normal_result = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(normal_result, Some(normal_id));

    let low_result = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(low_result, Some(low_id));

    let empty_result = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(empty_result, None);
}

#[tokio::test]
async fn test_queue_fifo_for_same_priority() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads with same priority
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "first", DownloadOptions::default())
        .await
        .unwrap();

    // Small delay to ensure different timestamps
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "second", DownloadOptions::default())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "third", DownloadOptions::default())
        .await
        .unwrap();

    // Should return in FIFO order for same priority
    let result1 = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(result1, Some(id1));

    let result2 = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(result2, Some(id2));

    let result3 = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(result3, Some(id3));
}

#[tokio::test]
async fn test_queue_remove_download() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add downloads
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "first", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "second", DownloadOptions::default())
        .await
        .unwrap();

    let id3 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "third", DownloadOptions::default())
        .await
        .unwrap();

    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 3);

    // Remove middle download
    let removed = downloader.remove_from_queue(id2).await;
    assert!(removed);

    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 2);

    // Should still get id1 and id3
    let result1 = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(result1, Some(id1));

    let result3 = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(result3, Some(id3));

    let empty_result = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(empty_result, None);
}

#[tokio::test]
async fn test_queue_remove_nonexistent() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Try to remove download that doesn't exist
    let removed = downloader.remove_from_queue(DownloadId(999)).await;
    assert!(!removed);
}

#[tokio::test]
async fn test_queue_force_priority() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add normal priority download
    let normal_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "normal", DownloadOptions::default())
        .await
        .unwrap();

    // Add force priority download (should jump to front)
    let force_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "force",
            DownloadOptions {
                priority: Priority::Force,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Force should come first even though added second
    let force_result = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(force_result, Some(force_id));

    let normal_result = {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.pop().map(|item| item.id)
    };
    assert_eq!(normal_result, Some(normal_id));
}

#[tokio::test]
async fn test_queue_state_persisted_to_database() {
    // Test: Queue state is persisted to SQLite on every change
    let (downloader, _temp_dir) = create_test_downloader().await;

    // 1. Add download - should persist Status::Queued
    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Verify Status::Queued persisted to database
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        download.status,
        Status::Queued.to_i32(),
        "Status should be Queued in DB"
    );
    assert_eq!(download.priority, 0, "Priority should be Normal (0)");

    // 2. Pause download - should persist Status::Paused
    downloader.pause(id).await.unwrap();

    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        download.status,
        Status::Paused.to_i32(),
        "Status should be Paused in DB"
    );

    // 3. Resume download - should persist Status::Queued again
    downloader.resume(id).await.unwrap();

    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        download.status,
        Status::Queued.to_i32(),
        "Status should be Queued in DB after resume"
    );

    // 4. Verify in-memory queue and database are synchronized
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 1, "In-memory queue should have 1 download");

    // Query incomplete downloads from DB (should include our Queued download)
    let incomplete = downloader.db.get_incomplete_downloads().await.unwrap();
    assert_eq!(incomplete.len(), 1, "DB should have 1 incomplete download");
    assert_eq!(incomplete[0].id, id, "Incomplete download ID should match");

    // 5. Cancel download - should remove from database
    downloader.cancel(id).await.unwrap();

    let download = downloader.db.get_download(id).await.unwrap();
    assert!(download.is_none(), "Download should be deleted from DB");

    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 0, "In-memory queue should be empty");
}

#[tokio::test]
async fn test_queue_ordering_persisted_correctly() {
    // Test that queue ordering (priority + created_at) is persisted and queryable
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add downloads with different priorities
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

    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await; // Ensure different timestamps

    let id_normal = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "normal",
            DownloadOptions {
                priority: Priority::Normal,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    let id_high = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "high",
            DownloadOptions {
                priority: Priority::High,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Query database with priority ordering (as restore_queue() would do)
    let all_downloads = downloader.db.list_downloads().await.unwrap();

    // Should be ordered: High, Normal, Low (priority DESC)
    assert_eq!(all_downloads.len(), 3, "Should have 3 downloads");
    assert_eq!(
        all_downloads[0].id, id_high,
        "First should be High priority"
    );
    assert_eq!(
        all_downloads[1].id, id_normal,
        "Second should be Normal priority"
    );
    assert_eq!(all_downloads[2].id, id_low, "Third should be Low priority");

    // Verify priorities are correct in database
    assert_eq!(all_downloads[0].priority, Priority::High as i32);
    assert_eq!(all_downloads[1].priority, Priority::Normal as i32);
    assert_eq!(all_downloads[2].priority, Priority::Low as i32);
}
