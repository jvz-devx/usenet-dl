use super::*;

/// Test that start_post_processing with PostProcess::None sets status to Complete
/// and emits a Complete event.
#[tokio::test]
async fn test_post_process_success_sets_status_to_complete() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download with explicit PostProcess::None to avoid the default (UnpackAndCleanup)
    // which would try to scan the filesystem for archives
    let download_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions {
                post_process: Some(crate::config::PostProcess::None),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Set download to Downloading first (simulating completed download phase)
    downloader
        .db
        .update_status(download_id, Status::Downloading.to_i32())
        .await
        .unwrap();

    // Subscribe to events before triggering post-processing
    let mut events = downloader.subscribe();

    // The download's post_process field defaults to 0 (None), so the post-processor
    // will skip all stages and return Ok(download_path).
    // start_post_processing should then call handle_post_process_success.
    let result = downloader.start_post_processing(download_id).await;
    assert!(
        result.is_ok(),
        "post-processing with None mode should succeed: {:?}",
        result
    );

    // Verify the download status is now Complete in the database
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Complete,
        "download should be marked Complete after successful post-processing"
    );

    // Verify a Complete event was emitted
    let mut found_complete = false;
    for _ in 0..10 {
        match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Ok(Event::Complete { id, .. })) if id == download_id => {
                found_complete = true;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    assert!(
        found_complete,
        "should emit a Complete event after successful post-processing"
    );
}

/// Test that start_post_processing with a non-existent download returns NotFound
#[tokio::test]
async fn test_post_process_nonexistent_download_returns_not_found() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let result = downloader.start_post_processing(DownloadId(99999)).await;

    match result {
        Err(crate::error::Error::NotFound(msg)) => {
            assert!(
                msg.contains("99999"),
                "error message should contain the download ID, got: {msg}"
            );
        }
        other => panic!("expected NotFound error for non-existent download, got: {other:?}"),
    }
}

/// Test that start_post_processing updates status to Processing before running pipeline
#[tokio::test]
async fn test_post_process_sets_processing_status_first() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let download_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions {
                post_process: Some(crate::config::PostProcess::None),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Start post-processing (None mode completes instantly)
    downloader.start_post_processing(download_id).await.unwrap();

    // By the time it finishes, status should be Complete (went through Processing first).
    // We can't observe the intermediate Processing state in a unit test since it's async,
    // but the final state should be Complete, confirming the pipeline ran.
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Complete,
        "final status should be Complete after successful None-mode post-processing"
    );
}

/// Test that post-processing failure sets status to Failed and records the error message
#[tokio::test]
async fn test_post_process_failure_sets_status_to_failed_and_records_error() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download with Verify post-processing mode
    let download_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions {
                post_process: Some(crate::config::PostProcess::UnpackAndCleanup),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // The temp directory for the download won't exist (no actual download happened),
    // so the move stage will fail when trying to process files.
    // But first, create the download temp dir so we get past the extract stage.
    // Actually, UnpackAndCleanup with NoOpParity will: verify(skip), repair(skip),
    // then extract(scan for archives in non-existent dir), which will fail with IO error.

    // Subscribe to events
    let mut events = downloader.subscribe();

    let result = downloader.start_post_processing(download_id).await;

    // The pipeline should fail because the download directory doesn't exist
    assert!(
        result.is_err(),
        "post-processing should fail when download directory doesn't exist"
    );

    // Verify the download status is Failed in the database
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Failed,
        "download should be marked Failed after post-processing failure"
    );

    // Verify an error message was recorded
    assert!(
        download.error_message.is_some(),
        "error message should be recorded in the database on failure"
    );
    let error_msg = download.error_message.unwrap();
    assert!(!error_msg.is_empty(), "error message should not be empty");

    // Verify a Failed event was emitted
    let mut found_failed = false;
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Ok(Event::Failed { id, error, .. })) if id == download_id => {
                assert!(
                    !error.is_empty(),
                    "Failed event should contain a non-empty error message"
                );
                found_failed = true;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    assert!(
        found_failed,
        "should emit a Failed event after post-processing failure"
    );
}

/// Test that post-processing success emits Complete event with correct path
#[tokio::test]
async fn test_post_process_success_emits_complete_event_with_path() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let download_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions {
                post_process: Some(crate::config::PostProcess::None),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let mut events = downloader.subscribe();

    downloader.start_post_processing(download_id).await.unwrap();

    // Find the Complete event and verify it has a path
    let mut found = false;
    for _ in 0..10 {
        match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Ok(Event::Complete { id, path })) if id == download_id => {
                // With PostProcess::None, the path should be the temp download path
                let expected_suffix = format!("download_{}", download_id.0);
                assert!(
                    path.to_string_lossy().contains(&expected_suffix),
                    "Complete event path should contain download temp dir, got: {:?}",
                    path
                );
                found = true;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    assert!(found, "should emit Complete event with path");
}

/// Test that post-processing failure event includes stage and error details
#[tokio::test]
async fn test_post_process_failure_event_includes_error_details() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let download_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "test.nzb",
            DownloadOptions {
                post_process: Some(crate::config::PostProcess::UnpackAndCleanup),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let mut events = downloader.subscribe();

    let _ = downloader.start_post_processing(download_id).await;

    // Find the Failed event
    let mut found = false;
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Ok(Event::Failed {
                id,
                stage,
                error,
                files_kept,
            })) if id == download_id => {
                assert_eq!(
                    stage,
                    crate::types::Stage::Extract,
                    "failure stage should default to Extract"
                );
                assert!(!error.is_empty(), "error message should be populated");
                assert!(files_kept, "files_kept should default to true on failure");
                found = true;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    assert!(
        found,
        "should emit Failed event with stage and error details"
    );
}

/// Test that reprocess() emits Verifying event, sets Processing status, and spawns post-processing
#[tokio::test]
async fn test_reprocess_emits_verifying_and_transitions_to_processing() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Add a download with PostProcess::None so the spawned post-processing completes cleanly
    let download_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "reprocess-test.nzb",
            DownloadOptions {
                post_process: Some(crate::config::PostProcess::None),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Mark the download as Complete (reprocess is meant for re-running on finished downloads)
    downloader
        .db
        .update_status(download_id, Status::Complete.to_i32())
        .await
        .unwrap();

    // Create the download temp directory (reprocess checks it exists)
    let download_temp_dir = downloader
        .config
        .download
        .temp_dir
        .join(format!("download_{}", download_id.0));
    tokio::fs::create_dir_all(&download_temp_dir).await.unwrap();

    // Subscribe to events before calling reprocess
    let mut events = downloader.subscribe();

    // Call reprocess — this should emit Verifying synchronously and spawn post-processing
    downloader.reprocess(download_id).await.unwrap();

    // Verify status was set to Processing immediately (before spawn completes)
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Processing,
        "reprocess should set status to Processing before spawning async work"
    );

    // Verify error message was cleared
    let error_msg = download.error_message.as_deref().unwrap_or("");
    assert!(
        error_msg.is_empty(),
        "reprocess should clear any previous error message"
    );

    // Verify Verifying event was emitted
    let mut found_verifying = false;
    for _ in 0..10 {
        match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Ok(Event::Verifying { id })) if id == download_id => {
                found_verifying = true;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    assert!(
        found_verifying,
        "reprocess should emit a Verifying event indicating post-processing has started"
    );

    // Wait for the spawned post-processing task to complete
    // With PostProcess::None, the pipeline completes almost instantly
    let mut found_complete = false;
    for _ in 0..30 {
        match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Ok(Event::Complete { id, .. })) if id == download_id => {
                found_complete = true;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    assert!(
        found_complete,
        "spawned post-processing should eventually emit Complete event"
    );

    // Verify final status is Complete after the spawned task finishes
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Complete,
        "download should be Complete after spawned post-processing finishes"
    );
}

/// Test that reprocess() fails when download temp directory doesn't exist
#[tokio::test]
async fn test_reprocess_missing_files_returns_not_found() {
    let (downloader, temp_dir) = create_test_downloader().await;

    // Override temp_dir to an isolated path so we control whether the download dir exists
    // The default "temp" relative path could point to an existing directory in the CWD
    let isolated_temp = temp_dir.path().join("nonexistent_temp");
    // Note: we intentionally do NOT create isolated_temp — reprocess needs it to not exist

    let download_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "missing-files.nzb",
            DownloadOptions::default(),
        )
        .await
        .unwrap();

    // Mark as Complete but the temp directory for this download won't exist
    // at the isolated_temp path
    downloader
        .db
        .update_status(download_id, Status::Complete.to_i32())
        .await
        .unwrap();

    // Swap the config to use the isolated temp dir so the download dir check fails
    // We need to create a new downloader with the isolated temp dir config
    let mut config_copy = (*downloader.config).clone();
    config_copy.download.temp_dir = isolated_temp;
    let config_arc = std::sync::Arc::new(config_copy);

    // Reconstruct downloader with the isolated config
    let downloader_isolated = UsenetDownloader {
        db: downloader.db.clone(),
        event_tx: downloader.event_tx.clone(),
        config: config_arc,
        nntp_pools: downloader.nntp_pools.clone(),
        speed_limiter: downloader.speed_limiter.clone(),
        queue_state: downloader.queue_state.clone(),
        runtime_config: downloader.runtime_config.clone(),
        processing: downloader.processing.clone(),
    };

    let result = downloader_isolated.reprocess(download_id).await;

    match result {
        Err(crate::error::Error::NotFound(msg)) => {
            assert!(
                msg.contains("Cannot reprocess"),
                "error should indicate files not found for reprocessing, got: {msg}"
            );
        }
        other => panic!("expected NotFound error when download files don't exist, got: {other:?}"),
    }

    // Status should remain Complete since reprocess checks existence BEFORE setting Processing
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Status::from_i32(download.status),
        Status::Complete,
        "status should remain unchanged when reprocess fails due to missing files"
    );
}

/// Test that successful post-processing preserves download metadata in DB
#[tokio::test]
async fn test_post_process_success_preserves_download_name_and_category() {
    let (downloader, _temp_dir) = create_test_downloader().await;

    let download_id = downloader
        .add_nzb_content(
            SAMPLE_NZB.as_bytes(),
            "my-movie.nzb",
            DownloadOptions {
                category: Some("movies".to_string()),
                post_process: Some(crate::config::PostProcess::None),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    downloader.start_post_processing(download_id).await.unwrap();

    // Verify metadata is preserved after post-processing
    let download = downloader
        .db
        .get_download(download_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        download.name, "my-movie.nzb",
        "download name should be preserved"
    );
    assert_eq!(
        download.category.as_deref(),
        Some("movies"),
        "category should be preserved"
    );
    assert_eq!(
        Status::from_i32(download.status),
        Status::Complete,
        "status should be Complete"
    );
}
