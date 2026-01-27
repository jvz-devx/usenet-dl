//! Post-processing pipeline entry point.

use crate::error::{Error, Result};
use crate::types::{DownloadId, Event, Status};
use std::path::PathBuf;

use super::UsenetDownloader;

impl UsenetDownloader {
    /// Start post-processing for a completed download
    ///
    /// This is the entry point to the post-processing pipeline. It coordinates
    /// verification, repair, extraction, moving, and cleanup based on the
    /// configured PostProcess mode.
    ///
    /// # Arguments
    ///
    /// * `download_id` - The download to post-process
    ///
    /// # Returns
    ///
    /// Returns Ok(()) on success, Err on any stage failure
    ///
    /// # Example
    ///
    /// ```no_run
    /// use usenet_dl::{UsenetDownloader, Config};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let downloader = UsenetDownloader::new(Config::default()).await?;
    ///
    ///     // After download completes, start post-processing
    ///     downloader.start_post_processing(1).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn start_post_processing(&self, download_id: DownloadId) -> Result<()> {
        tracing::info!(download_id, "starting post-processing");

        // Update status to Processing
        self.db
            .update_status(download_id, Status::Processing.to_i32())
            .await?;

        // Get download info from database
        let download = self
            .db
            .get_download(download_id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("download {} not found", download_id)))?;

        // Determine download path (temp directory)
        let download_path = self
            .config
            .download
            .temp_dir
            .join(format!("download_{}", download_id));

        // Determine final destination
        let destination = PathBuf::from(&download.destination);

        // Determine post-processing mode
        let post_process = crate::config::PostProcess::from_i32(download.post_process);

        // Execute post-processing pipeline
        match self
            .post_processor
            .start_post_processing(download_id, download_path, post_process, destination)
            .await
        {
            Ok(final_path) => {
                self.handle_post_process_success(
                    download_id,
                    download.name,
                    download.category,
                    download.size_bytes as u64,
                    final_path,
                )
                .await
            }
            Err(e) => {
                self.handle_post_process_failure(
                    download_id,
                    download.name,
                    download.category,
                    download.size_bytes as u64,
                    e,
                )
                .await
            }
        }
    }

    /// Handle successful post-processing: update status, emit events, trigger webhooks/scripts.
    async fn handle_post_process_success(
        &self,
        download_id: DownloadId,
        name: String,
        category: Option<String>,
        size_bytes: u64,
        final_path: PathBuf,
    ) -> Result<()> {
        self.db
            .update_status(download_id, Status::Complete.to_i32())
            .await?;

        self.event_tx
            .send(Event::Complete {
                id: download_id,
                path: final_path.clone(),
            })
            .ok();

        self.trigger_webhooks(
            crate::config::WebhookEvent::OnComplete,
            download_id,
            name.clone(),
            category.clone(),
            "complete".to_string(),
            Some(final_path.clone()),
            None,
        );

        self.trigger_scripts(
            crate::config::ScriptEvent::OnPostProcessComplete,
            download_id,
            name.clone(),
            category.clone(),
            "complete".to_string(),
            Some(final_path.clone()),
            None,
            size_bytes,
        );
        self.trigger_scripts(
            crate::config::ScriptEvent::OnComplete,
            download_id,
            name,
            category,
            "complete".to_string(),
            Some(final_path),
            None,
            size_bytes,
        );

        tracing::info!(download_id, "post-processing completed successfully");
        Ok(())
    }

    /// Handle failed post-processing: update status, emit events, trigger webhooks/scripts.
    async fn handle_post_process_failure(
        &self,
        download_id: DownloadId,
        name: String,
        category: Option<String>,
        size_bytes: u64,
        e: Error,
    ) -> Result<()> {
        let error_message = e.to_string();

        self.db
            .update_status(download_id, Status::Failed.to_i32())
            .await?;
        self.db.set_error(download_id, &error_message).await?;

        self.event_tx
            .send(Event::Failed {
                id: download_id,
                stage: crate::types::Stage::Extract, // Default to Extract stage
                error: error_message.clone(),
                files_kept: true, // Default: keep files on failure
            })
            .ok();

        self.trigger_webhooks(
            crate::config::WebhookEvent::OnFailed,
            download_id,
            name.clone(),
            category.clone(),
            "failed".to_string(),
            None,
            Some(error_message.clone()),
        );

        self.trigger_scripts(
            crate::config::ScriptEvent::OnFailed,
            download_id,
            name,
            category,
            "failed".to_string(),
            None,
            Some(error_message),
            size_bytes,
        );

        tracing::error!(download_id, error = %e, "post-processing failed");
        Err(e)
    }
}
