use super::*;
use crate::parity::NoOpParityHandler;
use tokio::sync::broadcast;

/// Helper to create a no-op parity handler for tests
fn test_parity_handler() -> Arc<dyn ParityHandler> {
    Arc::new(NoOpParityHandler)
}

async fn test_database() -> Arc<crate::db::Database> {
    let temp_file = tempfile::NamedTempFile::new().unwrap();
    let db = crate::db::Database::new(temp_file.path()).await.unwrap();
    Arc::new(db)
}

#[tokio::test]
async fn test_post_processing_none() {
    let (tx, _rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(tx, config, test_parity_handler(), test_database().await);

    let download_path = PathBuf::from("/tmp/download");
    let destination = PathBuf::from("/tmp/destination");

    let result = processor
        .start_post_processing(
            DownloadId(1),
            download_path.clone(),
            PostProcess::None,
            destination,
        )
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), download_path);
}

#[tokio::test]
async fn test_post_processing_verify() {
    use tempfile::TempDir;

    let (tx, mut rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(tx, config, test_parity_handler(), test_database().await);

    // Create temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    let destination = temp_dir.path().join("destination");
    tokio::fs::create_dir_all(&download_path).await.unwrap();

    let result = processor
        .start_post_processing(
            DownloadId(1),
            download_path.clone(),
            PostProcess::Verify,
            destination,
        )
        .await;

    assert!(result.is_ok());

    // Check that Verifying and VerifyComplete events were emitted
    let event1 = rx.recv().await.unwrap();
    assert!(matches!(event1, Event::Verifying { id } if id == DownloadId(1)));

    let event2 = rx.recv().await.unwrap();
    assert!(matches!(
        event2,
        Event::VerifyComplete {
            id,
            damaged: false
        } if id == DownloadId(1)
    ));
}

#[tokio::test]
async fn test_post_processing_unpack_and_cleanup() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, mut rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(tx, config, test_parity_handler(), test_database().await);

    // Create temporary directories and files for testing
    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    let destination = temp_dir.path().join("destination");

    // Create the download directory with a test file
    fs::create_dir_all(&download_path).await.unwrap();
    fs::write(download_path.join("test.txt"), b"test content")
        .await
        .unwrap();

    let result = processor
        .start_post_processing(
            DownloadId(1),
            download_path.clone(),
            PostProcess::UnpackAndCleanup,
            destination.clone(),
        )
        .await;

    assert!(result.is_ok());

    // Check that all stage events were emitted in order
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();

    assert!(!events.is_empty());

    // Should have: Verifying, VerifyComplete, Extracting, ExtractComplete, Moving, Cleaning
    assert!(events.iter().any(|e| matches!(e, Event::Verifying { .. })));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::VerifyComplete { .. }))
    );
    assert!(events.iter().any(|e| matches!(e, Event::Extracting { .. })));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::ExtractComplete { .. }))
    );
    assert!(events.iter().any(|e| matches!(e, Event::Moving { .. })));
    assert!(events.iter().any(|e| matches!(e, Event::Cleaning { .. })));

    // Verify file was moved to destination
    assert!(destination.join("test.txt").exists());
}

#[tokio::test]
async fn test_stage_executor_ordering() {
    use tempfile::TempDir;
    use tokio::fs;

    // Verify that stages execute in the correct order
    let (tx, mut rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(tx, config, test_parity_handler(), test_database().await);

    // Create temporary directories and files
    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    let destination = temp_dir.path().join("destination");

    fs::create_dir_all(&download_path).await.unwrap();
    fs::write(download_path.join("test.txt"), b"test content")
        .await
        .unwrap();

    processor
        .start_post_processing(
            DownloadId(1),
            download_path,
            PostProcess::UnpackAndCleanup,
            destination,
        )
        .await
        .unwrap();

    // Collect events
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    // Find indices of each stage
    let verifying_idx = events
        .iter()
        .position(|e| matches!(e, Event::Verifying { .. }));
    let extracting_idx = events
        .iter()
        .position(|e| matches!(e, Event::Extracting { .. }));
    let moving_idx = events
        .iter()
        .position(|e| matches!(e, Event::Moving { .. }));
    let cleaning_idx = events
        .iter()
        .position(|e| matches!(e, Event::Cleaning { .. }));

    // Verify ordering
    assert!(verifying_idx < extracting_idx);
    assert!(extracting_idx < moving_idx);
    assert!(moving_idx < cleaning_idx);
}

#[tokio::test]
async fn test_move_files_single_file_no_collision() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(tx, config, test_parity_handler(), test_database().await);

    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("source.txt");
    let dest = temp_dir.path().join("dest.txt");

    fs::write(&source, b"test content").await.unwrap();

    let result = processor.move_files(DownloadId(1), &source, &dest).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), dest);
    assert!(dest.exists());
    assert!(!source.exists());
}

#[tokio::test]
async fn test_move_files_collision_rename() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let mut config = Config::default();
    config.download.file_collision = crate::config::FileCollisionAction::Rename;
    let processor = PostProcessor::new(
        tx,
        Arc::new(config),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("source.txt");
    let dest = temp_dir.path().join("dest.txt");

    // Create both source and existing destination
    fs::write(&source, b"new content").await.unwrap();
    fs::write(&dest, b"existing content").await.unwrap();

    let result = processor.move_files(DownloadId(1), &source, &dest).await;
    assert!(result.is_ok());

    let final_dest = result.unwrap();
    assert_ne!(final_dest, dest); // Should have been renamed
    assert!(final_dest.to_str().unwrap().contains("dest (1).txt"));
    assert!(final_dest.exists());
    assert!(dest.exists()); // Original should still exist
    assert!(!source.exists()); // Source should be moved
}

#[tokio::test]
async fn test_move_files_collision_overwrite() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let mut config = Config::default();
    config.download.file_collision = crate::config::FileCollisionAction::Overwrite;
    let processor = PostProcessor::new(
        tx,
        Arc::new(config),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("source.txt");
    let dest = temp_dir.path().join("dest.txt");

    // Create both source and existing destination
    fs::write(&source, b"new content").await.unwrap();
    fs::write(&dest, b"existing content").await.unwrap();

    let result = processor.move_files(DownloadId(1), &source, &dest).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), dest);
    assert!(dest.exists());
    assert!(!source.exists());

    // Verify content was overwritten
    let content = fs::read_to_string(&dest).await.unwrap();
    assert_eq!(content, "new content");
}

#[tokio::test]
async fn test_move_files_collision_skip() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let mut config = Config::default();
    config.download.file_collision = crate::config::FileCollisionAction::Skip;
    let processor = PostProcessor::new(
        tx,
        Arc::new(config),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("source.txt");
    let dest = temp_dir.path().join("dest.txt");

    // Create both source and existing destination
    fs::write(&source, b"new content").await.unwrap();
    fs::write(&dest, b"existing content").await.unwrap();

    let result = processor.move_files(DownloadId(1), &source, &dest).await;
    assert!(result.is_err()); // Should fail with collision error
    assert!(source.exists()); // Source should still exist
    assert!(dest.exists()); // Destination should still exist

    // Verify original content preserved
    let content = fs::read_to_string(&dest).await.unwrap();
    assert_eq!(content, "existing content");
}

#[tokio::test]
async fn test_move_directory_contents() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(tx, config, test_parity_handler(), test_database().await);

    let temp_dir = TempDir::new().unwrap();
    let source_dir = temp_dir.path().join("source");
    let dest_dir = temp_dir.path().join("dest");

    // Create source directory with multiple files and subdirectories
    fs::create_dir_all(&source_dir).await.unwrap();
    fs::write(source_dir.join("file1.txt"), b"content1")
        .await
        .unwrap();
    fs::write(source_dir.join("file2.txt"), b"content2")
        .await
        .unwrap();

    let subdir = source_dir.join("subdir");
    fs::create_dir_all(&subdir).await.unwrap();
    fs::write(subdir.join("file3.txt"), b"content3")
        .await
        .unwrap();

    let result = processor
        .move_files(DownloadId(1), &source_dir, &dest_dir)
        .await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), dest_dir);

    // Verify all files were moved
    assert!(dest_dir.join("file1.txt").exists());
    assert!(dest_dir.join("file2.txt").exists());
    assert!(dest_dir.join("subdir/file3.txt").exists());

    // Verify source files no longer exist
    assert!(!source_dir.join("file1.txt").exists());
    assert!(!source_dir.join("file2.txt").exists());
    assert!(!source_dir.join("subdir/file3.txt").exists());
}

#[tokio::test]
async fn test_move_directory_with_collision_rename() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let mut config = Config::default();
    config.download.file_collision = crate::config::FileCollisionAction::Rename;
    let processor = PostProcessor::new(
        tx,
        Arc::new(config),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let source_dir = temp_dir.path().join("source");
    let dest_dir = temp_dir.path().join("dest");

    // Create source directory with files
    fs::create_dir_all(&source_dir).await.unwrap();
    fs::write(source_dir.join("file.txt"), b"new content")
        .await
        .unwrap();

    // Create destination directory with conflicting file
    fs::create_dir_all(&dest_dir).await.unwrap();
    fs::write(dest_dir.join("file.txt"), b"existing content")
        .await
        .unwrap();

    let result = processor
        .move_files(DownloadId(1), &source_dir, &dest_dir)
        .await;
    assert!(result.is_ok());

    // Both files should exist (one renamed)
    assert!(dest_dir.join("file.txt").exists());
    assert!(dest_dir.join("file (1).txt").exists());

    // Verify original content preserved
    let original = fs::read_to_string(dest_dir.join("file.txt")).await.unwrap();
    assert_eq!(original, "existing content");

    let renamed = fs::read_to_string(dest_dir.join("file (1).txt"))
        .await
        .unwrap();
    assert_eq!(renamed, "new content");
}

#[tokio::test]
async fn test_cleanup_removes_target_extensions() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(
        tx,
        config.clone(),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    fs::create_dir_all(&download_path).await.unwrap();

    // Create files with target extensions
    fs::write(download_path.join("file.par2"), b"par2 data")
        .await
        .unwrap();
    fs::write(download_path.join("file.nzb"), b"nzb data")
        .await
        .unwrap();
    fs::write(download_path.join("file.sfv"), b"sfv data")
        .await
        .unwrap();
    fs::write(download_path.join("file.srr"), b"srr data")
        .await
        .unwrap();
    fs::write(download_path.join("file.nfo"), b"nfo data")
        .await
        .unwrap();

    // Create files that should NOT be deleted
    fs::write(download_path.join("video.mkv"), b"video data")
        .await
        .unwrap();
    fs::write(download_path.join("readme.txt"), b"readme")
        .await
        .unwrap();

    // Call cleanup directly via the cleanup module
    crate::post_processing::cleanup::run_cleanup_stage(
        DownloadId(1),
        &download_path,
        &processor.event_tx,
        &config,
    )
    .await
    .unwrap();

    // Verify target files were deleted
    assert!(!download_path.join("file.par2").exists());
    assert!(!download_path.join("file.nzb").exists());
    assert!(!download_path.join("file.sfv").exists());
    assert!(!download_path.join("file.srr").exists());
    assert!(!download_path.join("file.nfo").exists());

    // Verify other files still exist
    assert!(download_path.join("video.mkv").exists());
    assert!(download_path.join("readme.txt").exists());
}

#[tokio::test]
async fn test_cleanup_removes_archive_files() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(
        tx,
        config.clone(),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    fs::create_dir_all(&download_path).await.unwrap();

    // Create archive files
    fs::write(download_path.join("file.rar"), b"rar data")
        .await
        .unwrap();
    fs::write(download_path.join("file.zip"), b"zip data")
        .await
        .unwrap();
    fs::write(download_path.join("file.7z"), b"7z data")
        .await
        .unwrap();

    // Create extracted files (should NOT be deleted)
    fs::write(download_path.join("video.mkv"), b"video data")
        .await
        .unwrap();

    crate::post_processing::cleanup::run_cleanup_stage(
        DownloadId(1),
        &download_path,
        &processor.event_tx,
        &config,
    )
    .await
    .unwrap();

    // Verify archive files were deleted
    assert!(!download_path.join("file.rar").exists());
    assert!(!download_path.join("file.zip").exists());
    assert!(!download_path.join("file.7z").exists());

    // Verify extracted files still exist
    assert!(download_path.join("video.mkv").exists());
}

#[tokio::test]
async fn test_cleanup_removes_sample_folders() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(
        tx,
        config.clone(),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    fs::create_dir_all(&download_path).await.unwrap();

    // Create sample folders with various case variations
    let sample_dir = download_path.join("sample");
    fs::create_dir_all(&sample_dir).await.unwrap();
    fs::write(sample_dir.join("sample.mkv"), b"sample video")
        .await
        .unwrap();

    let samples_dir = download_path.join("Samples");
    fs::create_dir_all(&samples_dir).await.unwrap();
    fs::write(samples_dir.join("sample.mkv"), b"sample video")
        .await
        .unwrap();

    // Create a normal folder (should NOT be deleted)
    let content_dir = download_path.join("content");
    fs::create_dir_all(&content_dir).await.unwrap();
    fs::write(content_dir.join("video.mkv"), b"real video")
        .await
        .unwrap();

    crate::post_processing::cleanup::run_cleanup_stage(
        DownloadId(1),
        &download_path,
        &processor.event_tx,
        &config,
    )
    .await
    .unwrap();

    // Verify sample folders were deleted
    assert!(!sample_dir.exists());
    assert!(!samples_dir.exists());

    // Verify normal folder still exists
    assert!(content_dir.exists());
    assert!(content_dir.join("video.mkv").exists());
}

#[tokio::test]
async fn test_cleanup_case_insensitive() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(
        tx,
        config.clone(),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    fs::create_dir_all(&download_path).await.unwrap();

    // Create files with uppercase extensions
    fs::write(download_path.join("file.PAR2"), b"par2 data")
        .await
        .unwrap();
    fs::write(download_path.join("file.NZB"), b"nzb data")
        .await
        .unwrap();
    fs::write(download_path.join("file.RAR"), b"rar data")
        .await
        .unwrap();

    crate::post_processing::cleanup::run_cleanup_stage(
        DownloadId(1),
        &download_path,
        &processor.event_tx,
        &config,
    )
    .await
    .unwrap();

    // Verify uppercase files were deleted (case-insensitive)
    assert!(!download_path.join("file.PAR2").exists());
    assert!(!download_path.join("file.NZB").exists());
    assert!(!download_path.join("file.RAR").exists());
}

#[tokio::test]
async fn test_cleanup_recursive() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(
        tx,
        config.clone(),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    fs::create_dir_all(&download_path).await.unwrap();

    // Create nested directory structure
    let subdir = download_path.join("subdir");
    fs::create_dir_all(&subdir).await.unwrap();

    // Create target files in subdirectory
    fs::write(subdir.join("file.par2"), b"par2 data")
        .await
        .unwrap();
    fs::write(subdir.join("file.nzb"), b"nzb data")
        .await
        .unwrap();

    // Create normal file in subdirectory
    fs::write(subdir.join("video.mkv"), b"video data")
        .await
        .unwrap();

    crate::post_processing::cleanup::run_cleanup_stage(
        DownloadId(1),
        &download_path,
        &processor.event_tx,
        &config,
    )
    .await
    .unwrap();

    // Verify target files in subdirectory were deleted
    assert!(!subdir.join("file.par2").exists());
    assert!(!subdir.join("file.nzb").exists());

    // Verify normal file still exists
    assert!(subdir.join("video.mkv").exists());
}

#[tokio::test]
async fn test_cleanup_disabled() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let mut config = Config::default();
    config.processing.cleanup.enabled = false;
    let config = Arc::new(config);
    let processor = PostProcessor::new(
        tx,
        config.clone(),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    fs::create_dir_all(&download_path).await.unwrap();

    // Create files that would normally be deleted
    fs::write(download_path.join("file.par2"), b"par2 data")
        .await
        .unwrap();
    fs::write(download_path.join("file.nzb"), b"nzb data")
        .await
        .unwrap();

    crate::post_processing::cleanup::run_cleanup_stage(
        DownloadId(1),
        &download_path,
        &processor.event_tx,
        &config,
    )
    .await
    .unwrap();

    // Verify files still exist (cleanup was disabled)
    assert!(download_path.join("file.par2").exists());
    assert!(download_path.join("file.nzb").exists());
}

#[tokio::test]
async fn test_cleanup_delete_samples_disabled() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let mut config = Config::default();
    config.processing.cleanup.delete_samples = false;
    let config = Arc::new(config);
    let processor = PostProcessor::new(
        tx,
        config.clone(),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    fs::create_dir_all(&download_path).await.unwrap();

    // Create sample folder
    let sample_dir = download_path.join("sample");
    fs::create_dir_all(&sample_dir).await.unwrap();
    fs::write(sample_dir.join("sample.mkv"), b"sample video")
        .await
        .unwrap();

    // Create target files (should still be deleted)
    fs::write(download_path.join("file.par2"), b"par2 data")
        .await
        .unwrap();

    crate::post_processing::cleanup::run_cleanup_stage(
        DownloadId(1),
        &download_path,
        &processor.event_tx,
        &config,
    )
    .await
    .unwrap();

    // Verify sample folder still exists (delete_samples disabled)
    assert!(sample_dir.exists());
    assert!(sample_dir.join("sample.mkv").exists());

    // Verify target files were still deleted
    assert!(!download_path.join("file.par2").exists());
}

#[tokio::test]
async fn test_cleanup_nonexistent_path() {
    let (tx, _rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    let processor = PostProcessor::new(
        tx,
        config.clone(),
        test_parity_handler(),
        test_database().await,
    );

    let nonexistent_path = PathBuf::from("/tmp/nonexistent_path_12345");

    // Should not error when path doesn't exist
    let result = crate::post_processing::cleanup::run_cleanup_stage(
        DownloadId(1),
        &nonexistent_path,
        &processor.event_tx,
        &config,
    )
    .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_verify_stage_handles_not_supported() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, mut rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    // NoOpParityHandler returns Error::NotSupported for verify
    let processor = PostProcessor::new(
        tx.clone(),
        config,
        test_parity_handler(),
        test_database().await,
    );

    // Create temporary directory with a PAR2 file
    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    fs::create_dir_all(&download_path).await.unwrap();
    fs::write(download_path.join("test.par2"), b"fake par2 data")
        .await
        .unwrap();

    // Verify stage should NOT fail even though NoOpParityHandler doesn't support verify
    let result = crate::post_processing::verify::run_verify_stage(
        DownloadId(1),
        &download_path,
        &tx,
        &processor.parity_handler,
    )
    .await;
    assert!(result.is_ok());

    // Should have emitted Verifying event
    let event1 = rx.recv().await.unwrap();
    assert!(matches!(event1, Event::Verifying { id } if id == DownloadId(1)));

    // Should have emitted VerifyComplete event (skipped, assume no damage)
    let event2 = rx.recv().await.unwrap();
    assert!(matches!(
        event2,
        Event::VerifyComplete {
            id,
            damaged: false
        } if id == DownloadId(1)
    ));
}

#[tokio::test]
async fn test_move_nested_subdirectory_with_collision_rename() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, _rx) = broadcast::channel(100);
    let mut config = Config::default();
    config.download.file_collision = crate::config::FileCollisionAction::Rename;
    let processor = PostProcessor::new(
        tx,
        Arc::new(config),
        test_parity_handler(),
        test_database().await,
    );

    let temp_dir = TempDir::new().unwrap();
    let source_dir = temp_dir.path().join("source");
    let dest_dir = temp_dir.path().join("dest");

    // Create source with nested subdirectory containing a file
    let source_subdir = source_dir.join("subdir");
    fs::create_dir_all(&source_subdir).await.unwrap();
    fs::write(source_subdir.join("file.txt"), b"new nested content")
        .await
        .unwrap();

    // Create destination with same subdirectory and a conflicting file
    let dest_subdir = dest_dir.join("subdir");
    fs::create_dir_all(&dest_subdir).await.unwrap();
    fs::write(dest_subdir.join("file.txt"), b"existing nested content")
        .await
        .unwrap();

    let result = processor
        .move_files(DownloadId(1), &source_dir, &dest_dir)
        .await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), dest_dir);

    // Original file should be preserved at its original path
    let original = fs::read_to_string(dest_subdir.join("file.txt"))
        .await
        .unwrap();
    assert_eq!(
        original, "existing nested content",
        "original nested file should be preserved"
    );

    // New file should have been renamed to avoid collision
    let renamed = fs::read_to_string(dest_subdir.join("file (1).txt"))
        .await
        .unwrap();
    assert_eq!(
        renamed, "new nested content",
        "moved file should be renamed with (1) suffix"
    );

    // Source subdirectory should have been cleaned up (emptied and removed)
    assert!(
        !source_subdir.exists(),
        "source subdirectory should be removed after move"
    );
}

#[tokio::test]
async fn test_repair_stage_handles_not_supported() {
    use tempfile::TempDir;
    use tokio::fs;

    let (tx, mut rx) = broadcast::channel(100);
    let config = Arc::new(Config::default());
    // NoOpParityHandler returns Error::NotSupported for both verify and repair
    let processor = PostProcessor::new(
        tx.clone(),
        config,
        test_parity_handler(),
        test_database().await,
    );

    // Create temporary directory with a PAR2 file
    let temp_dir = TempDir::new().unwrap();
    let download_path = temp_dir.path().join("download");
    fs::create_dir_all(&download_path).await.unwrap();
    fs::write(download_path.join("test.par2"), b"fake par2 data")
        .await
        .unwrap();

    // Repair stage should NOT fail even though NoOpParityHandler doesn't support verify/repair
    let result = crate::post_processing::repair::run_repair_stage(
        DownloadId(1),
        &download_path,
        &tx,
        &processor.parity_handler,
    )
    .await;
    assert!(result.is_ok());

    // Verify now returns NotSupported, so the repair stage hits the verify catch
    // and emits only RepairSkipped (never reaches Repairing)
    let event1 = rx.recv().await.unwrap();
    assert!(matches!(event1, Event::RepairSkipped { id, .. } if id == DownloadId(1)));
}
