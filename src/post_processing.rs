//! Post-processing pipeline for completed downloads
//!
//! This module handles the post-processing pipeline after articles are downloaded:
//! 1. Verify - PAR2 verification
//! 2. Repair - PAR2 repair (if verification fails)
//! 3. Extract - Archive extraction (RAR, 7z, ZIP)
//! 4. Move - Move files to final destination
//! 5. Cleanup - Remove intermediate files (.par2, .nzb, archives, samples)

use crate::config::PostProcess;
use crate::error::Result;
use crate::types::{DownloadId, Event};
use std::path::PathBuf;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// Post-processing pipeline executor
pub struct PostProcessor {
    /// Event channel for emitting pipeline events
    event_tx: broadcast::Sender<Event>,
}

impl PostProcessor {
    /// Create a new post-processing pipeline executor
    pub fn new(event_tx: broadcast::Sender<Event>) -> Self {
        Self { event_tx }
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

        // TODO: Implement file moving with collision handling
        warn!(download_id, "file moving not yet implemented");

        // For now, return the destination unchanged
        Ok(destination.clone())
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
        let processor = PostProcessor::new(tx);

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
        let processor = PostProcessor::new(tx);

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
        let (tx, mut rx) = broadcast::channel(100);
        let processor = PostProcessor::new(tx);

        let download_path = PathBuf::from("/tmp/download");
        let destination = PathBuf::from("/tmp/destination");

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
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok())
            .collect();

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
    }

    #[tokio::test]
    async fn test_stage_executor_ordering() {
        // Verify that stages execute in the correct order
        let (tx, mut rx) = broadcast::channel(100);
        let processor = PostProcessor::new(tx);

        let download_path = PathBuf::from("/tmp/download");
        let destination = PathBuf::from("/tmp/destination");

        processor
            .start_post_processing(
                1,
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
}
