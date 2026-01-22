//! Post-processing pipeline for completed downloads
//!
//! This module handles the post-processing pipeline after articles are downloaded:
//! 1. Verify - PAR2 verification
//! 2. Repair - PAR2 repair (if verification fails)
//! 3. Extract - Archive extraction (RAR, 7z, ZIP)
//! 4. Move - Move files to final destination
//! 5. Cleanup - Remove intermediate files (.par2, .nzb, archives, samples)

use crate::config::{Config, PostProcess};
use crate::error::Result;
use crate::types::{DownloadId, Event};
use crate::utils::get_unique_path;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// Post-processing pipeline executor
pub struct PostProcessor {
    /// Event channel for emitting pipeline events
    event_tx: broadcast::Sender<Event>,
    /// Configuration for file collision handling
    config: Arc<Config>,
}

impl PostProcessor {
    /// Create a new post-processing pipeline executor
    pub fn new(event_tx: broadcast::Sender<Event>, config: Arc<Config>) -> Self {
        Self { event_tx, config }
    }

    /// Execute post-processing pipeline for a completed download
    ///
    /// This is the main entry point for post-processing. It orchestrates
    /// the pipeline stages based on the configured PostProcess mode.
    ///
    /// # Arguments
    ///
    /// * `download_id` - The download to post-process
    /// * `download_path` - Path to the downloaded files
    /// * `post_process` - Post-processing mode to use
    /// * `destination` - Final destination for files
    ///
    /// # Returns
    ///
    /// Returns Ok(final_path) on success, Err on failure
    pub async fn start_post_processing(
        &self,
        download_id: DownloadId,
        download_path: PathBuf,
        post_process: PostProcess,
        destination: PathBuf,
    ) -> Result<PathBuf> {
        info!(
            download_id,
            ?post_process,
            ?download_path,
            ?destination,
            "starting post-processing pipeline"
        );

        // Execute pipeline stages based on post-processing mode
        match post_process {
            PostProcess::None => {
                // No post-processing, just return the download path
                debug!(download_id, "skipping post-processing (mode: None)");
                Ok(download_path)
            }

            PostProcess::Verify => {
                // Just verify
                self.run_verify_stage(download_id, &download_path).await?;
                Ok(download_path)
            }

            PostProcess::Repair => {
                // Verify and repair if needed
                self.run_verify_stage(download_id, &download_path).await?;
                self.run_repair_stage(download_id, &download_path).await?;
                Ok(download_path)
            }

            PostProcess::Unpack => {
                // Verify, repair, and extract
                self.run_verify_stage(download_id, &download_path).await?;
                self.run_repair_stage(download_id, &download_path).await?;
                let extracted_path = self
                    .run_extract_stage(download_id, &download_path)
                    .await?;
                Ok(extracted_path)
            }

            PostProcess::UnpackAndCleanup => {
                // Full pipeline: verify, repair, extract, move, cleanup
                self.run_verify_stage(download_id, &download_path).await?;
                self.run_repair_stage(download_id, &download_path).await?;
                let extracted_path = self
                    .run_extract_stage(download_id, &download_path)
                    .await?;
                let final_path = self
                    .run_move_stage(download_id, &extracted_path, &destination)
                    .await?;
                self.run_cleanup_stage(download_id, &download_path).await?;
                Ok(final_path)
            }
        }
    }

    /// Execute the verify stage
    async fn run_verify_stage(
        &self,
        download_id: DownloadId,
        download_path: &PathBuf,
    ) -> Result<()> {
        debug!(download_id, ?download_path, "running verify stage");

        // Emit Verifying event
        self.event_tx
            .send(Event::Verifying {
                id: download_id,
            })
            .ok();

        // TODO: Implement PAR2 verification using nntp-rs
        // For now, just simulate success
        warn!(download_id, "PAR2 verification not yet implemented");

        // Emit VerifyComplete event
        self.event_tx
            .send(Event::VerifyComplete {
                id: download_id,
                damaged: false,
            })
            .ok();

        Ok(())
    }

    /// Execute the repair stage
    async fn run_repair_stage(
        &self,
        download_id: DownloadId,
        download_path: &PathBuf,
    ) -> Result<()> {
        debug!(download_id, ?download_path, "running repair stage");

        // TODO: Implement PAR2 repair
        // Note: nntp-rs only supports verification, not repair
        // We may need external par2cmdline tool or skip this in MVP
        warn!(download_id, "PAR2 repair not yet implemented");

        Ok(())
    }

    /// Execute the extract stage
    async fn run_extract_stage(
        &self,
        download_id: DownloadId,
        download_path: &PathBuf,
    ) -> Result<PathBuf> {
        debug!(download_id, ?download_path, "running extract stage");

        // Emit Extracting event
        self.event_tx
            .send(Event::Extracting {
                id: download_id,
                archive: String::new(),
                percent: 0.0,
            })
            .ok();

        // TODO: Implement archive extraction (RAR, 7z, ZIP)
        warn!(download_id, "archive extraction not yet implemented");

        // Emit ExtractComplete event
        self.event_tx
            .send(Event::ExtractComplete {
                id: download_id,
            })
            .ok();

        // For now, return the download path unchanged
        Ok(download_path.clone())
    }

    /// Execute the move stage
    async fn run_move_stage(
        &self,
        download_id: DownloadId,
        source_path: &PathBuf,
        destination: &PathBuf,
    ) -> Result<PathBuf> {
        debug!(
            download_id,
            ?source_path,
            ?destination,
            "running move stage"
        );

        // Emit Moving event
        self.event_tx
            .send(Event::Moving {
                id: download_id,
                destination: destination.clone(),
            })
            .ok();

        // Perform the actual file move with collision handling
        self.move_files(download_id, source_path, destination)
            .await
    }

    /// Move files from source to destination with collision handling
    ///
    /// This function handles moving files/directories from the source path to the
    /// destination path, applying the configured FileCollisionAction when files
    /// already exist at the destination.
    ///
    /// # Arguments
    ///
    /// * `download_id` - The download ID for logging
    /// * `source_path` - Path to the source files/directory
    /// * `destination` - Path to the destination directory
    ///
    /// # Returns
    ///
    /// Returns Ok(final_path) where final_path is the actual destination used,
    /// or Err if the move operation fails
    async fn move_files(
        &self,
        download_id: DownloadId,
        source_path: &PathBuf,
        destination: &PathBuf,
    ) -> Result<PathBuf> {
        use tokio::fs;

        debug!(
            download_id,
            ?source_path,
            ?destination,
            "moving files with collision action: {:?}",
            self.config.file_collision
        );

        // Check if source exists
        if !source_path.exists() {
            return Err(crate::error::Error::InvalidPath {
                path: source_path.clone(),
                reason: "Source path does not exist".to_string(),
            });
        }

        // Ensure destination parent directory exists
        if let Some(parent) = destination.parent() {
            if !parent.exists() {
                debug!(
                    download_id,
                    ?parent,
                    "creating destination parent directory"
                );
                fs::create_dir_all(parent).await?;
            }
        }

        // If source is a file, move it directly
        if source_path.is_file() {
            return self
                .move_single_file(download_id, source_path, destination)
                .await;
        }

        // If source is a directory, move all its contents
        if source_path.is_dir() {
            return self
                .move_directory_contents(download_id, source_path, destination)
                .await;
        }

        // If we get here, source is neither file nor directory
        Err(crate::error::Error::InvalidPath {
            path: source_path.clone(),
            reason: "Source is neither a file nor a directory".to_string(),
        })
    }

    /// Move a single file to destination with collision handling
    async fn move_single_file(
        &self,
        download_id: DownloadId,
        source_file: &PathBuf,
        destination: &PathBuf,
    ) -> Result<PathBuf> {
        use tokio::fs;

        // Apply collision handling to get the actual destination path
        let final_destination = get_unique_path(destination, self.config.file_collision)?;

        debug!(
            download_id,
            ?source_file,
            ?final_destination,
            "moving single file"
        );

        // Perform the move
        fs::rename(source_file, &final_destination).await?;

        info!(
            download_id,
            ?source_file,
            ?final_destination,
            "successfully moved file"
        );

        Ok(final_destination)
    }

    /// Move directory contents to destination with collision handling
    fn move_directory_contents<'a>(
        &'a self,
        download_id: DownloadId,
        source_dir: &'a PathBuf,
        destination: &'a PathBuf,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PathBuf>> + Send + 'a>> {
        Box::pin(async move {
            use tokio::fs;

            debug!(
                download_id,
                ?source_dir,
                ?destination,
                "moving directory contents"
            );

            // Create destination directory if it doesn't exist
            if !destination.exists() {
                fs::create_dir_all(destination).await?;
            }

            // Read all entries in source directory
            let mut entries = fs::read_dir(source_dir).await?;

            // Move each entry
            while let Some(entry) = entries.next_entry().await? {
                let source_entry_path = entry.path();
                let entry_name = entry.file_name();
                let dest_entry_path = destination.join(&entry_name);

                if source_entry_path.is_file() {
                    // Move file with collision handling
                    self.move_single_file(download_id, &source_entry_path, &dest_entry_path)
                        .await?;
                } else if source_entry_path.is_dir() {
                    // Recursively move subdirectory
                    self.move_directory_contents(download_id, &source_entry_path, &dest_entry_path)
                        .await?;

                    // Remove the now-empty source subdirectory
                    fs::remove_dir(&source_entry_path).await?;
                }
            }

            info!(
                download_id,
                ?source_dir,
                ?destination,
                "successfully moved directory contents"
            );

            Ok(destination.clone())
        })
    }

    /// Execute the cleanup stage
    async fn run_cleanup_stage(
        &self,
        download_id: DownloadId,
        download_path: &PathBuf,
    ) -> Result<()> {
        debug!(download_id, ?download_path, "running cleanup stage");

        // Emit Cleaning event
        self.event_tx
            .send(Event::Cleaning {
                id: download_id,
            })
            .ok();

        // TODO: Implement cleanup (.par2, .nzb, archives, samples)
        warn!(download_id, "cleanup not yet implemented");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    #[tokio::test]
    async fn test_post_processing_none() {
        let (tx, _rx) = broadcast::channel(100);
        let config = Arc::new(Config::default());
        let processor = PostProcessor::new(tx, config);

        let download_path = PathBuf::from("/tmp/download");
        let destination = PathBuf::from("/tmp/destination");

        let result = processor
            .start_post_processing(1, download_path.clone(), PostProcess::None, destination)
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), download_path);
    }

    #[tokio::test]
    async fn test_post_processing_verify() {
        let (tx, mut rx) = broadcast::channel(100);
        let config = Arc::new(Config::default());
        let processor = PostProcessor::new(tx, config);

        let download_path = PathBuf::from("/tmp/download");
        let destination = PathBuf::from("/tmp/destination");

        let result = processor
            .start_post_processing(1, download_path.clone(), PostProcess::Verify, destination)
            .await;

        assert!(result.is_ok());

        // Check that Verifying and VerifyComplete events were emitted
        let event1 = rx.recv().await.unwrap();
        assert!(matches!(event1, Event::Verifying { id: 1 }));

        let event2 = rx.recv().await.unwrap();
        assert!(matches!(
            event2,
            Event::VerifyComplete {
                id: 1,
                damaged: false
            }
        ));
    }

    #[tokio::test]
    async fn test_post_processing_unpack_and_cleanup() {
        use tempfile::TempDir;
        use tokio::fs;

        let (tx, mut rx) = broadcast::channel(100);
        let config = Arc::new(Config::default());
        let processor = PostProcessor::new(tx, config);

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
                1,
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
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::VerifyComplete { .. })));
        assert!(events.iter().any(|e| matches!(e, Event::Extracting { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::ExtractComplete { .. })));
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
        let processor = PostProcessor::new(tx, config);

        // Create temporary directories and files
        let temp_dir = TempDir::new().unwrap();
        let download_path = temp_dir.path().join("download");
        let destination = temp_dir.path().join("destination");

        fs::create_dir_all(&download_path).await.unwrap();
        fs::write(download_path.join("test.txt"), b"test content")
            .await
            .unwrap();

        processor
            .start_post_processing(1, download_path, PostProcess::UnpackAndCleanup, destination)
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
        let processor = PostProcessor::new(tx, config);

        let temp_dir = TempDir::new().unwrap();
        let source = temp_dir.path().join("source.txt");
        let dest = temp_dir.path().join("dest.txt");

        fs::write(&source, b"test content").await.unwrap();

        let result = processor.move_files(1, &source, &dest).await;
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
        config.file_collision = crate::config::FileCollisionAction::Rename;
        let processor = PostProcessor::new(tx, Arc::new(config));

        let temp_dir = TempDir::new().unwrap();
        let source = temp_dir.path().join("source.txt");
        let dest = temp_dir.path().join("dest.txt");

        // Create both source and existing destination
        fs::write(&source, b"new content").await.unwrap();
        fs::write(&dest, b"existing content").await.unwrap();

        let result = processor.move_files(1, &source, &dest).await;
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
        config.file_collision = crate::config::FileCollisionAction::Overwrite;
        let processor = PostProcessor::new(tx, Arc::new(config));

        let temp_dir = TempDir::new().unwrap();
        let source = temp_dir.path().join("source.txt");
        let dest = temp_dir.path().join("dest.txt");

        // Create both source and existing destination
        fs::write(&source, b"new content").await.unwrap();
        fs::write(&dest, b"existing content").await.unwrap();

        let result = processor.move_files(1, &source, &dest).await;
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
        config.file_collision = crate::config::FileCollisionAction::Skip;
        let processor = PostProcessor::new(tx, Arc::new(config));

        let temp_dir = TempDir::new().unwrap();
        let source = temp_dir.path().join("source.txt");
        let dest = temp_dir.path().join("dest.txt");

        // Create both source and existing destination
        fs::write(&source, b"new content").await.unwrap();
        fs::write(&dest, b"existing content").await.unwrap();

        let result = processor.move_files(1, &source, &dest).await;
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
        let processor = PostProcessor::new(tx, config);

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

        let result = processor.move_files(1, &source_dir, &dest_dir).await;
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
        config.file_collision = crate::config::FileCollisionAction::Rename;
        let processor = PostProcessor::new(tx, Arc::new(config));

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

        let result = processor.move_files(1, &source_dir, &dest_dir).await;
        assert!(result.is_ok());

        // Both files should exist (one renamed)
        assert!(dest_dir.join("file.txt").exists());
        assert!(dest_dir.join("file (1).txt").exists());

        // Verify original content preserved
        let original = fs::read_to_string(dest_dir.join("file.txt"))
            .await
            .unwrap();
        assert_eq!(original, "existing content");

        let renamed = fs::read_to_string(dest_dir.join("file (1).txt"))
            .await
            .unwrap();
        assert_eq!(renamed, "new content");
    }
}
