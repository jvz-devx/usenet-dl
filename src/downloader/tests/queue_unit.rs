use crate::downloader::test_helpers::{SAMPLE_NZB, create_test_downloader};
use crate::error::{DatabaseError, Error};
use crate::types::{DownloadId, DownloadOptions, Priority, Status};

// --- add_to_queue() tests ---

#[tokio::test]
async fn test_add_to_queue_download_appears_in_queue() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // add_nzb_content already calls add_to_queue internally,
    // verify it's there
    let queue = downloader.queue_state.queue.lock().await;
    assert_eq!(queue.len(), 1, "queue should contain the added download");

    let peeked = queue.peek().unwrap();
    assert_eq!(peeked.id, id, "queued download ID should match");
    assert_eq!(
        peeked.priority,
        Priority::Normal,
        "default priority should be Normal"
    );
}

#[tokio::test]
async fn test_add_to_queue_nonexistent_download_returns_not_found() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let result = downloader.add_to_queue(DownloadId(99999)).await;

    match result {
        Err(Error::Database(DatabaseError::NotFound(msg))) => {
            assert!(
                msg.contains("99999"),
                "error should mention the nonexistent ID, got: {}",
                msg
            );
        }
        other => panic!("expected NotFound error, got: {:?}", other),
    }
}

// --- remove_from_queue() tests ---

#[tokio::test]
async fn test_remove_from_queue_returns_true_and_removes() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    assert_eq!(downloader.queue_state.queue.lock().await.len(), 1);

    let removed = downloader.remove_from_queue(id).await;
    assert!(
        removed,
        "remove_from_queue should return true for existing download"
    );

    assert_eq!(
        downloader.queue_state.queue.lock().await.len(),
        0,
        "queue should be empty after removal"
    );
}

#[tokio::test]
async fn test_remove_from_queue_nonexistent_returns_false() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let removed = downloader.remove_from_queue(DownloadId(99999)).await;
    assert!(
        !removed,
        "remove_from_queue should return false for nonexistent ID"
    );
}

// --- priority ordering tests ---

#[tokio::test]
async fn test_priority_ordering_high_before_normal_before_low() {
    let (downloader, _temp_dir) = create_test_downloader().await;

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

    let mut queue = downloader.queue_state.queue.lock().await;

    let first = queue.pop().unwrap();
    assert_eq!(first.id, id_high, "High priority should be dequeued first");

    let second = queue.pop().unwrap();
    assert_eq!(
        second.id, id_normal,
        "Normal priority should be dequeued second"
    );

    let third = queue.pop().unwrap();
    assert_eq!(third.id, id_low, "Low priority should be dequeued third");

    assert!(queue.pop().is_none(), "queue should be empty");
}

// --- restore_queue() tests ---

#[tokio::test]
async fn test_restore_queue_with_queued_downloads_readds_to_queue() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add downloads
    let id1 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test1", DownloadOptions::default())
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test2", DownloadOptions::default())
        .await
        .unwrap();

    // Clear the in-memory queue to simulate restart
    {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.clear();
    }

    // Queue should be empty now
    assert_eq!(downloader.queue_state.queue.lock().await.len(), 0);

    // Restore queue from DB
    let needs_pp = downloader.restore_queue().await.unwrap();
    assert!(
        needs_pp.is_empty(),
        "Queued downloads should not need post-processing"
    );

    // Queue should be repopulated
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(
        queue_size, 2,
        "restore_queue should re-add all Queued downloads"
    );

    // Verify both downloads are in the queue
    let mut queue = downloader.queue_state.queue.lock().await;
    let ids: Vec<DownloadId> = std::iter::from_fn(|| queue.pop().map(|q| q.id)).collect();
    assert!(ids.contains(&id1), "queue should contain id1");
    assert!(ids.contains(&id2), "queue should contain id2");
}

#[tokio::test]
async fn test_restore_queue_downloading_with_pending_articles_requeues() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download (creates articles with status=PENDING)
    let id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "downloading-test",
            DownloadOptions::default(),
        )
        .await
        .unwrap();

    // Simulate that download was actively downloading when the app crashed
    downloader
        .db
        .update_status(id, Status::Downloading.to_i32())
        .await
        .unwrap();

    // Clear in-memory queue to simulate restart
    {
        downloader.queue_state.queue.lock().await.clear();
    }

    let needs_pp = downloader.restore_queue().await.unwrap();

    // Articles are still pending, so it should be re-queued, not post-processed
    assert!(
        needs_pp.is_empty(),
        "download with pending articles should not need post-processing"
    );

    // Verify status was changed back to Queued for re-download
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Queued,
        "downloading download with pending articles should be re-queued"
    );

    // Verify it's in the in-memory queue
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 1, "download should be added back to the queue");
}

#[tokio::test]
async fn test_restore_queue_downloading_all_articles_done_needs_post_processing() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "all-done-test",
            DownloadOptions::default(),
        )
        .await
        .unwrap();

    // Mark all articles as downloaded (simulating completed download phase)
    let articles = downloader.db.get_articles(id).await.unwrap();
    assert!(
        !articles.is_empty(),
        "NZB should have created articles in the DB"
    );
    for article in &articles {
        downloader
            .db
            .update_article_status(article.id, crate::db::article_status::DOWNLOADED)
            .await
            .unwrap();
    }

    // Set status to Downloading (app crashed after all articles finished but before post-processing)
    downloader
        .db
        .update_status(id, Status::Downloading.to_i32())
        .await
        .unwrap();

    // Clear queue to simulate restart
    {
        downloader.queue_state.queue.lock().await.clear();
    }

    let needs_pp = downloader.restore_queue().await.unwrap();

    // All articles downloaded → should be marked Processing and returned for post-processing
    assert_eq!(
        needs_pp.len(),
        1,
        "download with all articles done should need post-processing"
    );
    assert_eq!(needs_pp[0], id, "returned ID should match the download");

    // Verify status is Processing in DB
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Processing,
        "download with all articles done should be marked Processing"
    );

    // Should NOT be in the queue (it needs post-processing, not downloading)
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(
        queue_size, 0,
        "completed download should not be in the download queue"
    );
}

#[tokio::test]
async fn test_restore_queue_processing_status_needs_post_processing() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "processing-test",
            DownloadOptions::default(),
        )
        .await
        .unwrap();

    // Mark all articles as downloaded
    let articles = downloader.db.get_articles(id).await.unwrap();
    for article in &articles {
        downloader
            .db
            .update_article_status(article.id, crate::db::article_status::DOWNLOADED)
            .await
            .unwrap();
    }

    // Set status to Processing (app crashed during post-processing)
    downloader
        .db
        .update_status(id, Status::Processing.to_i32())
        .await
        .unwrap();

    // Clear queue to simulate restart
    {
        downloader.queue_state.queue.lock().await.clear();
    }

    let needs_pp = downloader.restore_queue().await.unwrap();

    // Was already Processing → should be returned for post-processing
    assert_eq!(
        needs_pp.len(),
        1,
        "Processing download should be returned for post-processing"
    );
    assert_eq!(needs_pp[0], id);

    // Status should remain Processing
    let download = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Processing,
        "Processing download should stay in Processing state"
    );
}

#[tokio::test]
async fn test_restore_queue_with_no_incomplete_downloads_returns_empty() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download and mark it complete
    let id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "complete",
            DownloadOptions::default(),
        )
        .await
        .unwrap();

    downloader
        .db
        .update_status(id, Status::Complete.to_i32())
        .await
        .unwrap();

    // Clear queue
    {
        let mut queue = downloader.queue_state.queue.lock().await;
        queue.clear();
    }

    let needs_pp = downloader.restore_queue().await.unwrap();

    assert!(
        needs_pp.is_empty(),
        "no incomplete downloads should mean no post-processing needed"
    );

    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(
        queue_size, 0,
        "queue should remain empty when no incomplete downloads exist"
    );
}
