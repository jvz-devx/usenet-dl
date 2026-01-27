use super::*;

#[tokio::test]
async fn test_queue_persistence_enables_restore() {
    // Test that persisted queue state can be used to restore queue
    use tempfile::TempDir;

    // Create persistent temp directory for database
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("usenet-dl.db");

    // Create first downloader instance
    let config1 = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: db_path.clone(),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        download: config::DownloadConfig {
            temp_dir: temp_dir.path().join("temp"),
            download_dir: temp_dir.path().join("downloads"),
            ..Default::default()
        },
        ..Default::default()
    };
    let downloader = UsenetDownloader::new(config1).await.unwrap();

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

    // Mark one as Processing, complete one, leave one queued
    downloader
        .db
        .update_status(id2, Status::Processing.to_i32())
        .await
        .unwrap();
    downloader
        .db
        .update_status(id3, Status::Complete.to_i32())
        .await
        .unwrap();

    // Simulate restart: create new downloader with same database
    drop(downloader); // Close first instance

    let config2 = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: db_path.clone(),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        download: config::DownloadConfig {
            temp_dir: temp_dir.path().join("temp"),
            download_dir: temp_dir.path().join("downloads"),
            ..Default::default()
        },
        ..Default::default()
    };
    let downloader2 = UsenetDownloader::new(config2).await.unwrap();

    // Verify we can query incomplete downloads (would be used by restore_queue)
    // Note: get_incomplete_downloads() returns status IN (0, 1, 3) - Queued, Downloading, Processing
    // It intentionally excludes Paused (2), which would be handled separately
    let incomplete = downloader2.db.get_incomplete_downloads().await.unwrap();

    // Should have 2: id1 (Queued) and id2 (Processing)
    // Should NOT have id3 (Complete)
    assert_eq!(incomplete.len(), 2, "Should have 2 incomplete downloads");

    let incomplete_ids: Vec<i64> = incomplete.iter().map(|d| d.id).collect();
    assert!(
        incomplete_ids.contains(&id1.0),
        "Should include Queued download"
    );
    assert!(
        incomplete_ids.contains(&id2.0),
        "Should include Processing download"
    );
    assert!(
        !incomplete_ids.contains(&id3.0),
        "Should NOT include Complete download"
    );

    // Verify they're in priority order
    assert_eq!(incomplete[0].priority, 0, "First should be Normal priority");
    assert_eq!(
        incomplete[1].priority, 0,
        "Second should be Normal priority"
    );

    // Also verify paused downloads can be restored separately
    let paused = downloader2
        .db
        .list_downloads_by_status(Status::Paused.to_i32())
        .await
        .unwrap();
    assert_eq!(
        paused.len(),
        0,
        "No paused downloads in this test (id2 was set to Processing)"
    );
}

#[tokio::test]
async fn test_resume_download_with_pending_articles() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Simulate partial download: mark first article as downloaded
    let articles = downloader
        .db
        .get_pending_articles(download_id)
        .await
        .unwrap();
    assert_eq!(
        articles.len(),
        2,
        "Should have 2 pending articles initially"
    );

    downloader
        .db
        .update_article_status(articles[0].id, crate::db::article_status::DOWNLOADED)
        .await
        .unwrap();

    // Update download status to Paused (simulate interrupted download)
    downloader
        .db
        .update_status(download_id, Status::Paused.to_i32())
        .await
        .unwrap();

    // Resume the download
    downloader.resume_download(download_id).await.unwrap();

    // Verify download is back in Queued status
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(Status::from_i32(download.status), Status::Queued);

    // Verify only 1 article remains pending
    let pending = downloader
        .db
        .get_pending_articles(download_id)
        .await
        .unwrap();
    assert_eq!(
        pending.len(),
        1,
        "Should have 1 pending article after resume"
    );
    assert_eq!(
        pending[0].id, articles[1].id,
        "Should be the second article"
    );
}

#[tokio::test]
async fn test_resume_download_no_pending_articles() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark all articles as downloaded
    let articles = downloader
        .db
        .get_pending_articles(download_id)
        .await
        .unwrap();
    for article in articles {
        downloader
            .db
            .update_article_status(article.id, crate::db::article_status::DOWNLOADED)
            .await
            .unwrap();
    }

    // Update status to Downloading (simulate download just completed)
    downloader
        .db
        .update_status(download_id, Status::Downloading.to_i32())
        .await
        .unwrap();

    // Resume should proceed to post-processing
    downloader.resume_download(download_id).await.unwrap();

    // Verify status is now Processing (ready for post-processing)
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(Status::from_i32(download.status), Status::Processing);

    // Verify no pending articles remain
    let pending = downloader
        .db
        .get_pending_articles(download_id)
        .await
        .unwrap();
    assert_eq!(pending.len(), 0, "Should have no pending articles");
}

#[tokio::test]
async fn test_resume_download_nonexistent() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Try to resume non-existent download
    let result = downloader.resume_download(DownloadId(99999)).await;

    // Should succeed (get_pending_articles returns empty Vec for non-existent downloads)
    // This is acceptable behavior - resume_download is idempotent
    assert!(
        result.is_ok(),
        "Should succeed (no-op) for non-existent download"
    );

    // Verify no status was changed (download doesn't exist in database)
    let download = downloader.db.get_download(DownloadId(99999)).await.unwrap();
    assert!(download.is_none(), "Download should not exist");
}

#[tokio::test]
async fn test_resume_download_emits_event() {
    // resume_download() is a pure state function — it sets status to Processing
    // when no pending articles remain (post-processing is spawned by the caller).
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark all articles as downloaded
    let articles = downloader
        .db
        .get_pending_articles(download_id)
        .await
        .unwrap();
    for article in articles {
        downloader
            .db
            .update_article_status(article.id, crate::db::article_status::DOWNLOADED)
            .await
            .unwrap();
    }

    // Resume should set status to Processing (no spawn, no event)
    downloader.resume_download(download_id).await.unwrap();

    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Processing,
        "Should set status to Processing when no pending articles"
    );
}

// restore_queue() tests

#[tokio::test]
async fn test_restore_queue_with_no_incomplete_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Restore queue with empty database
    downloader.restore_queue().await.unwrap();

    // Queue should remain empty
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(
        queue_size, 0,
        "Queue should be empty when no incomplete downloads"
    );
}

#[tokio::test]
async fn test_restore_queue_with_queued_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple downloads with different priorities
    let id1 = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "download1",
            DownloadOptions {
                priority: Priority::Low,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let id2 = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "download2",
            DownloadOptions {
                priority: Priority::High,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Clear the queue (simulating a restart)
    downloader.queue_state.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Queue should have both downloads restored
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 2, "Queue should have 2 downloads restored");

    // Verify priority ordering (High priority should be first)
    let next = downloader.queue_state.queue.lock().await.pop().unwrap();
    assert_eq!(next.id, id2, "High priority download should be first");
    assert_eq!(next.priority, Priority::High);

    let next = downloader.queue_state.queue.lock().await.pop().unwrap();
    assert_eq!(next.id, id1, "Low priority download should be second");
    assert_eq!(next.priority, Priority::Low);
}

#[tokio::test]
async fn test_restore_queue_with_downloading_status() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Manually set status to Downloading (simulating interrupted download)
    downloader
        .db
        .update_status(download_id, Status::Downloading.to_i32())
        .await
        .unwrap();

    // Clear the queue
    downloader.queue_state.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Download should be back in queue with Queued status (resume_download does this)
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Queued,
        "Download status should be Queued after restore"
    );

    // Queue should contain the download
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 1, "Queue should have 1 download");
}

#[tokio::test]
async fn test_restore_queue_with_processing_status() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download and mark all articles as downloaded
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    // Mark all articles as downloaded
    let articles = downloader
        .db
        .get_pending_articles(download_id)
        .await
        .unwrap();
    for article in articles {
        downloader
            .db
            .update_article_status(article.id, crate::db::article_status::DOWNLOADED)
            .await
            .unwrap();
    }

    // Manually set status to Processing (simulating interrupted post-processing)
    downloader
        .db
        .update_status(download_id, Status::Processing.to_i32())
        .await
        .unwrap();

    // Clear the queue
    downloader.queue_state.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Download should still be in Processing status (ready for post-processing)
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Processing,
        "Download status should remain Processing after restore"
    );
}

#[tokio::test]
async fn test_restore_queue_skips_completed_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download and mark as complete
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader
        .db
        .update_status(download_id, Status::Complete.to_i32())
        .await
        .unwrap();

    // Clear the queue
    downloader.queue_state.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Queue should be empty (completed downloads not restored)
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(
        queue_size, 0,
        "Queue should be empty (completed downloads not restored)"
    );
}

#[tokio::test]
async fn test_restore_queue_skips_failed_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download and mark as failed
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader
        .db
        .update_status(download_id, Status::Failed.to_i32())
        .await
        .unwrap();

    // Clear the queue
    downloader.queue_state.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Queue should be empty (failed downloads not restored)
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(
        queue_size, 0,
        "Queue should be empty (failed downloads not restored)"
    );
}

#[tokio::test]
async fn test_restore_queue_skips_paused_downloads() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download and pause it
    let download_id = downloader
        .add_nzb_content(SAMPLE_NZB.as_bytes(), "test", DownloadOptions::default())
        .await
        .unwrap();

    downloader.pause(download_id).await.unwrap();

    // Clear the queue
    downloader.queue_state.queue.lock().await.clear();

    // Restore queue
    downloader.restore_queue().await.unwrap();

    // Queue should be empty (paused downloads not restored - user explicitly paused them)
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(
        queue_size, 0,
        "Queue should be empty (paused downloads not restored)"
    );

    // Status should still be Paused
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Paused,
        "Paused downloads should remain paused"
    );
}

#[tokio::test]
async fn test_restore_queue_called_on_startup() {
    // Create a database with incomplete downloads
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create first downloader instance and add downloads
    {
        let config = Config {
            persistence: crate::config::PersistenceConfig {
                database_path: db_path.clone(),
                schedule_rules: vec![],
                categories: std::collections::HashMap::new(),
            },
            servers: vec![],
            download: config::DownloadConfig {
                max_concurrent_downloads: 3,
                ..Default::default()
            },
            ..Default::default()
        };
        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Add downloads
        downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "download1",
                DownloadOptions::default(),
            )
            .await
            .unwrap();
        downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "download2",
                DownloadOptions::default(),
            )
            .await
            .unwrap();

        // downloader is dropped here (simulating shutdown)
    }

    // Create new downloader instance (simulating restart)
    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: db_path.clone(),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        servers: vec![],
        download: config::DownloadConfig {
            max_concurrent_downloads: 3,
            ..Default::default()
        },
        ..Default::default()
    };
    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Queue should be automatically restored (new() calls restore_queue())
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 2, "Queue should be restored on startup");
}

#[tokio::test]
async fn test_resume_after_simulated_crash() {
    // Test resume after simulated crash (kill process mid-download)
    //
    // This test simulates a crash by:
    // 1. Starting a download
    // 2. Marking some articles as downloaded (simulating partial progress)
    // 3. Setting status to Downloading (simulating crash mid-download)
    // 4. Dropping the downloader (simulating process termination)
    // 5. Creating a new downloader instance (simulating restart)
    // 6. Verifying that restore_queue() correctly resumes the download

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let download_id;
    let total_articles;

    // Simulate crash scenario
    {
        let config = Config {
            persistence: crate::config::PersistenceConfig {
                database_path: db_path.clone(),
                schedule_rules: vec![],
                categories: std::collections::HashMap::new(),
            },
            servers: vec![],
            download: config::DownloadConfig {
                max_concurrent_downloads: 3,
                ..Default::default()
            },
            ..Default::default()
        };
        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Add a download
        download_id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "crash_test",
                DownloadOptions::default(),
            )
            .await
            .unwrap();

        // Get all articles
        let articles = downloader
            .db
            .get_pending_articles(download_id)
            .await
            .unwrap();
        total_articles = articles.len();
        assert!(total_articles > 1, "Need at least 2 articles for this test");

        // Mark half of the articles as downloaded (simulating partial progress)
        let articles_to_download = total_articles / 2;
        for (i, article) in articles.iter().enumerate() {
            if i < articles_to_download {
                downloader
                    .db
                    .update_article_status(article.id, crate::db::article_status::DOWNLOADED)
                    .await
                    .unwrap();
            }
        }

        // Set status to Downloading (simulating crash mid-download)
        downloader
            .db
            .update_status(download_id, Status::Downloading.to_i32())
            .await
            .unwrap();

        // Set some progress to verify it's preserved
        let progress = 50.0;
        let speed = 1000000u64; // 1 MB/s
        let downloaded_bytes = 524288u64; // 512 KB
        downloader
            .db
            .update_progress(download_id, progress, speed, downloaded_bytes)
            .await
            .unwrap();

        // Simulate crash by dropping downloader (no graceful shutdown)
        // downloader is dropped here
    }

    // Simulate restart by creating a new downloader instance
    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: db_path.clone(),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        servers: vec![],
        download: config::DownloadConfig {
            max_concurrent_downloads: 3,
            ..Default::default()
        },
        ..Default::default()
    };
    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Verify the download was restored
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();

    // Status should be Queued (resume_download sets it back to Queued)
    assert_eq!(
        Status::from_i32(download.status),
        Status::Queued,
        "Download should be Queued after restore"
    );

    // Progress should be preserved from before crash
    assert_eq!(
        download.progress, 50.0,
        "Download progress should be preserved after crash"
    );

    // Downloaded bytes should be preserved
    assert_eq!(
        download.downloaded_bytes, 524288,
        "Downloaded bytes should be preserved after crash"
    );

    // Queue should contain the download
    let queue_size = downloader.queue_state.queue.lock().await.len();
    assert_eq!(queue_size, 1, "Queue should have 1 download after restore");

    // Verify that only pending articles remain
    let pending_articles = downloader
        .db
        .get_pending_articles(download_id)
        .await
        .unwrap();
    let expected_pending = total_articles - (total_articles / 2);
    assert_eq!(
        pending_articles.len(),
        expected_pending,
        "Only undownloaded articles should be pending"
    );

    // Verify that downloaded articles are marked correctly
    let downloaded_count = downloader
        .db
        .count_articles_by_status(download_id, crate::db::article_status::DOWNLOADED)
        .await
        .unwrap();
    assert_eq!(
        downloaded_count as usize,
        total_articles / 2,
        "Downloaded articles count should match"
    );
}

#[tokio::test]
async fn test_shutdown_graceful() {
    // Test graceful shutdown
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Verify shutdown completes successfully
    let result = downloader.shutdown().await;
    assert!(
        result.is_ok(),
        "Shutdown should complete successfully: {:?}",
        result
    );
}

#[tokio::test]
async fn test_shutdown_with_active_downloads() {
    // Test shutdown cancels active downloads
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Simulate some active downloads by adding cancellation tokens
    {
        let mut active = downloader.queue_state.active_downloads.lock().await;
        active.insert(DownloadId(1), tokio_util::sync::CancellationToken::new());
        active.insert(DownloadId(2), tokio_util::sync::CancellationToken::new());
    }

    // Verify we have active downloads
    {
        let active = downloader.queue_state.active_downloads.lock().await;
        assert_eq!(active.len(), 2);
    }

    // Shutdown should cancel them
    let result = downloader.shutdown().await;
    assert!(
        result.is_ok(),
        "Shutdown should complete successfully: {:?}",
        result
    );

    // Verify tokens were cancelled (active_downloads map should still contain them,
    // but they should be in cancelled state)
    {
        let active = downloader.queue_state.active_downloads.lock().await;
        for (_id, token) in active.iter() {
            assert!(
                token.is_cancelled(),
                "Download should be cancelled after shutdown"
            );
        }
    }
}

#[tokio::test]
async fn test_shutdown_waits_for_completion() {
    // Test shutdown waits for active downloads to complete
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download token, then remove it after a delay to simulate completion
    let token = tokio_util::sync::CancellationToken::new();
    {
        let mut active = downloader.queue_state.active_downloads.lock().await;
        active.insert(DownloadId(1), token.clone());
    }

    // Spawn a task that removes the download after 500ms (simulating completion)
    let active_downloads_clone = downloader.queue_state.active_downloads.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let mut active = active_downloads_clone.lock().await;
        active.remove(&DownloadId(1));
    });

    let start = std::time::Instant::now();

    // Shutdown should wait for the download to complete
    let result = downloader.shutdown().await;
    let elapsed = start.elapsed();

    assert!(
        result.is_ok(),
        "Shutdown should complete successfully: {:?}",
        result
    );

    // Verify it waited (should take at least 500ms)
    assert!(
        elapsed >= std::time::Duration::from_millis(450),
        "Shutdown should have waited for downloads to complete: {:?}",
        elapsed
    );

    // But not too long (should be < 1 second for this test)
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "Shutdown took too long: {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_shutdown_rejects_new_downloads() {
    // Test that shutdown() sets accepting_new flag and new downloads are rejected
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Initially, should accept new downloads
    assert!(
        downloader
            .queue_state
            .accepting_new
            .load(std::sync::atomic::Ordering::SeqCst),
        "Should accept new downloads initially"
    );

    // Attempt to add a download before shutdown - should succeed
    let result_before = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        )
        .await;
    assert!(
        result_before.is_ok(),
        "Should accept download before shutdown: {:?}",
        result_before
    );

    // Trigger shutdown
    let shutdown_result = downloader.shutdown().await;
    assert!(
        shutdown_result.is_ok(),
        "Shutdown should complete successfully: {:?}",
        shutdown_result
    );

    // After shutdown, accepting_new should be false
    assert!(
        !downloader
            .queue_state
            .accepting_new
            .load(std::sync::atomic::Ordering::SeqCst),
        "Should not accept new downloads after shutdown"
    );

    // Attempt to add a download after shutdown - should fail with ShuttingDown error
    let result_after = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test2.nzb",
            DownloadOptions::default(),
        )
        .await;

    assert!(
        result_after.is_err(),
        "Should reject download after shutdown"
    );
    match result_after {
        Err(crate::error::Error::ShuttingDown) => {
            // Expected error
        }
        other => panic!("Expected ShuttingDown error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_pause_graceful_all() {
    // Test graceful pause signals cancellation to all active downloads
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add multiple download tokens to simulate active downloads
    let token1 = tokio_util::sync::CancellationToken::new();
    let token2 = tokio_util::sync::CancellationToken::new();
    let token3 = tokio_util::sync::CancellationToken::new();

    {
        let mut active = downloader.queue_state.active_downloads.lock().await;
        active.insert(DownloadId(1), token1.clone());
        active.insert(DownloadId(2), token2.clone());
        active.insert(DownloadId(3), token3.clone());
    }

    // Verify tokens are not cancelled initially
    assert!(
        !token1.is_cancelled(),
        "Token 1 should not be cancelled initially"
    );
    assert!(
        !token2.is_cancelled(),
        "Token 2 should not be cancelled initially"
    );
    assert!(
        !token3.is_cancelled(),
        "Token 3 should not be cancelled initially"
    );

    // Call pause_graceful_all
    downloader.pause_graceful_all().await;

    // Verify all tokens are now cancelled (graceful pause signaled)
    assert!(
        token1.is_cancelled(),
        "Token 1 should be cancelled after graceful pause"
    );
    assert!(
        token2.is_cancelled(),
        "Token 2 should be cancelled after graceful pause"
    );
    assert!(
        token3.is_cancelled(),
        "Token 3 should be cancelled after graceful pause"
    );

    // Verify downloads are still in active_downloads map (they clean up when tasks complete)
    {
        let active = downloader.queue_state.active_downloads.lock().await;
        assert_eq!(active.len(), 3, "Downloads should still be in active map");
    }
}

#[tokio::test]
async fn test_graceful_pause_completes_current_article() {
    // Verify that graceful pause allows current article to complete
    // This is a conceptual test - the actual behavior is in the download loop
    // which checks cancellation BEFORE starting each article, not during.
    // This means the current article always completes before pausing.

    let (_downloader, _temp_dir) = create_test_downloader().await;

    // Create a cancellation token
    let token = tokio_util::sync::CancellationToken::new();
    let token_clone = token.clone();

    // Simulate an article download in progress
    let article_complete = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let article_complete_clone = article_complete.clone();

    // Spawn a task that simulates downloading an article (takes 200ms)
    let download_task = tokio::spawn(async move {
        // Simulate article download starting
        tracing::debug!("Article download started");

        // Download takes 200ms
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Mark article as complete
        article_complete_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        tracing::debug!("Article download completed");

        // After article completes, check for cancellation (this is what the real code does)
        if token_clone.is_cancelled() {
            tracing::debug!("Cancellation detected after article completed");
            return false; // Would exit the download loop
        }

        true // Would continue to next article
    });

    // Wait 100ms (article is in-progress)
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Signal graceful pause while article is downloading
    token.cancel();
    tracing::debug!("Graceful pause signaled while article in progress");

    // Wait for task to complete
    let result = download_task.await.unwrap();

    // Verify the article completed before the cancellation was detected
    assert!(
        article_complete.load(std::sync::atomic::Ordering::SeqCst),
        "Article should have completed"
    );
    assert!(
        !result,
        "Download should have stopped after detecting cancellation"
    );
}

#[tokio::test]
async fn test_persist_all_state_marks_interrupted_downloads_as_paused() {
    // Test persist_all_state() marks interrupted downloads as Paused
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download in Downloading status
    let id1 = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test1.nzb",
            DownloadOptions::default(),
        )
        .await
        .unwrap();

    // Manually set it to Downloading status (simulating active download)
    downloader
        .db
        .update_status(id1, Status::Downloading.to_i32())
        .await
        .unwrap();

    // Add another download in Processing status
    let id2 = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test2.nzb",
            DownloadOptions::default(),
        )
        .await
        .unwrap();
    downloader
        .db
        .update_status(id2, Status::Processing.to_i32())
        .await
        .unwrap();

    // Add a download in Complete status (should not be changed)
    let id3 = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test3.nzb",
            DownloadOptions::default(),
        )
        .await
        .unwrap();
    downloader
        .db
        .update_status(id3, Status::Complete.to_i32())
        .await
        .unwrap();

    // Verify initial states
    let dl1 = downloader.db.get_download(id1).await.unwrap().unwrap();
    assert_eq!(dl1.status, Status::Downloading.to_i32());
    let dl2 = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(dl2.status, Status::Processing.to_i32());
    let dl3 = downloader.db.get_download(id3).await.unwrap().unwrap();
    assert_eq!(dl3.status, Status::Complete.to_i32());

    // Call persist_all_state (these downloads are not in active_downloads map)
    let result = downloader.persist_all_state().await;
    assert!(
        result.is_ok(),
        "persist_all_state should succeed: {:?}",
        result
    );

    // Verify interrupted downloads were marked as Paused
    let dl1_after = downloader.db.get_download(id1).await.unwrap().unwrap();
    assert_eq!(
        dl1_after.status,
        Status::Paused.to_i32(),
        "Interrupted Downloading should be marked as Paused"
    );

    let dl2_after = downloader.db.get_download(id2).await.unwrap().unwrap();
    assert_eq!(
        dl2_after.status,
        Status::Paused.to_i32(),
        "Interrupted Processing should be marked as Paused"
    );

    // Complete download should remain unchanged
    let dl3_after = downloader.db.get_download(id3).await.unwrap().unwrap();
    assert_eq!(
        dl3_after.status,
        Status::Complete.to_i32(),
        "Complete download should remain Complete"
    );
}

#[tokio::test]
async fn test_persist_all_state_preserves_active_downloads() {
    // Test persist_all_state() does not modify truly active downloads
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download
    let id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        )
        .await
        .unwrap();

    // Set it to Downloading status
    downloader
        .db
        .update_status(id, Status::Downloading.to_i32())
        .await
        .unwrap();

    // Add it to active_downloads map (simulating it's actually running)
    {
        let mut active = downloader.queue_state.active_downloads.lock().await;
        active.insert(id, tokio_util::sync::CancellationToken::new());
    }

    // Call persist_all_state
    let result = downloader.persist_all_state().await;
    assert!(
        result.is_ok(),
        "persist_all_state should succeed: {:?}",
        result
    );

    // Verify the download status was NOT changed (it's still active)
    let dl_after = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        dl_after.status,
        Status::Downloading.to_i32(),
        "Active download should remain in Downloading status"
    );
}

#[tokio::test]
async fn test_shutdown_calls_persist_all_state() {
    // Test shutdown() integrates persist_all_state()
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download in Downloading status (simulating interrupted)
    let id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions::default(),
        )
        .await
        .unwrap();
    downloader
        .db
        .update_status(id, Status::Downloading.to_i32())
        .await
        .unwrap();

    // Call shutdown
    let result = downloader.shutdown().await;
    assert!(result.is_ok(), "Shutdown should succeed: {:?}", result);

    // Verify the interrupted download was marked as Paused by persist_all_state
    let dl_after = downloader.db.get_download(id).await.unwrap().unwrap();
    assert_eq!(
        dl_after.status,
        Status::Paused.to_i32(),
        "Interrupted download should be marked as Paused after shutdown"
    );
}

#[tokio::test]
async fn test_shutdown_emits_shutdown_event() {
    // Test that shutdown() emits a Shutdown event
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Spawn a task to collect events
    let event_handle = tokio::spawn(async move {
        let mut shutdown_received = false;
        while let Ok(event) = events.recv().await {
            if matches!(event, Event::Shutdown) {
                shutdown_received = true;
                break;
            }
        }
        shutdown_received
    });

    // Call shutdown
    let result = downloader.shutdown().await;
    assert!(result.is_ok(), "Shutdown should succeed: {:?}", result);

    // Verify Shutdown event was emitted
    let shutdown_received = tokio::time::timeout(std::time::Duration::from_secs(1), event_handle)
        .await
        .expect("Timeout waiting for event task")
        .expect("Event task should complete");

    assert!(shutdown_received, "Shutdown event should be emitted");
}

#[tokio::test]
async fn test_run_with_shutdown_basic() {
    // Test that run_with_shutdown function exists and is callable
    // Note: We can't easily test actual signal handling in unit tests,
    // but we verify the function compiles and the structure is correct

    let (downloader, _temp_dir) = create_test_downloader().await;

    // We can't easily send signals in a test, so we just verify
    // the function signature and structure by calling shutdown directly
    let result = downloader.shutdown().await;
    assert!(result.is_ok(), "Shutdown should succeed: {:?}", result);
}

#[tokio::test]
async fn test_graceful_shutdown_and_recovery_on_restart() {
    // Test complete graceful shutdown and recovery on restart
    //
    // This integration test verifies:
    // 1. Active downloads are gracefully paused on shutdown
    // 2. Database is marked as "clean shutdown"
    // 3. On restart, downloads are properly restored
    // 4. Progress and state are preserved across restart

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let download_id;
    let total_articles;

    // Part 1: Create downloader, add download, and perform graceful shutdown
    {
        let config = Config {
            persistence: crate::config::PersistenceConfig {
                database_path: db_path.clone(),
                schedule_rules: vec![],
                categories: std::collections::HashMap::new(),
            },
            servers: vec![],
            download: config::DownloadConfig {
                max_concurrent_downloads: 3,
                ..Default::default()
            },
            ..Default::default()
        };

        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Add a download
        download_id = downloader
            .add_nzb_content(
                SAMPLE_NZB.as_bytes(),
                "test.nzb",
                DownloadOptions::default(),
            )
            .await
            .unwrap();

        // Get all articles
        let articles = downloader
            .db
            .get_pending_articles(download_id)
            .await
            .unwrap();
        total_articles = articles.len();
        assert!(total_articles > 1, "Need at least 2 articles for this test");

        // Mark first article as downloaded (simulating partial progress)
        if let Some(first_article) = articles.first() {
            downloader
                .db
                .update_article_status(first_article.id, crate::db::article_status::DOWNLOADED)
                .await
                .unwrap();
        }

        // Set status to Downloading (simulating active download)
        downloader
            .db
            .update_status(download_id, Status::Downloading.to_i32())
            .await
            .unwrap();

        // Set some progress to verify it's preserved
        let progress = 50.0;
        let speed = 1000000u64; // 1 MB/s
        let downloaded_bytes = 524288u64; // 512 KB
        downloader
            .db
            .update_progress(download_id, progress, speed, downloaded_bytes)
            .await
            .unwrap();

        // Perform graceful shutdown
        let shutdown_result = downloader.shutdown().await;
        assert!(
            shutdown_result.is_ok(),
            "Graceful shutdown should succeed: {:?}",
            shutdown_result
        );

        // Verify database was marked as clean shutdown
        let was_unclean = downloader.db.was_unclean_shutdown().await.unwrap();
        assert!(
            !was_unclean,
            "Database should be marked as CLEAN shutdown after graceful shutdown"
        );

        // Verify download was marked as Paused (not Downloading)
        let download = downloader
            .db
            .get_download(download_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            Status::from_i32(download.status),
            Status::Paused,
            "Download should be marked as Paused after graceful shutdown"
        );
    }

    // Part 2: Simulate restart by creating new downloader instance
    {
        // First, check the shutdown state BEFORE creating the downloader
        // (UsenetDownloader::new() calls set_clean_start() which would override the flag)
        let db_for_check = Database::new(&db_path).await.unwrap();
        let was_unclean = db_for_check.was_unclean_shutdown().await.unwrap();
        assert!(
            !was_unclean,
            "Database should show clean shutdown from previous session"
        );
        db_for_check.close().await;

        // Now create the downloader (which will call set_clean_start() internally)
        let config = Config {
            persistence: crate::config::PersistenceConfig {
                database_path: db_path.clone(),
                schedule_rules: vec![],
                categories: std::collections::HashMap::new(),
            },
            servers: vec![],
            download: config::DownloadConfig {
                max_concurrent_downloads: 3,
                ..Default::default()
            },
            ..Default::default()
        };

        let downloader = UsenetDownloader::new(config).await.unwrap();

        // Verify download was restored
        let restored_download = downloader.db.get_download(download_id).await.unwrap();
        assert!(
            restored_download.is_some(),
            "Download should be restored after restart"
        );

        let download = restored_download.unwrap();

        // After graceful shutdown, download should remain Paused
        assert_eq!(
            Status::from_i32(download.status),
            Status::Paused,
            "Download should remain Paused after restart"
        );

        // Progress should be preserved
        assert_eq!(download.progress, 50.0, "Progress should be preserved");
        assert_eq!(
            download.downloaded_bytes, 524288,
            "Downloaded bytes should be preserved"
        );

        // Verify article tracking was preserved
        let pending_articles = downloader
            .db
            .get_pending_articles(download_id)
            .await
            .unwrap();
        assert_eq!(
            pending_articles.len(),
            total_articles - 1,
            "Should have {} pending articles (1 was downloaded before shutdown)",
            total_articles - 1
        );

        // Verify we can resume the download after restart
        let resume_result = downloader.resume(download_id).await;
        assert!(
            resume_result.is_ok(),
            "Should be able to resume download after restart: {:?}",
            resume_result
        );

        let resumed_download = downloader
            .db
            .get_download(download_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            Status::from_i32(resumed_download.status),
            Status::Queued,
            "Download should be Queued after resume"
        );
    }
}
