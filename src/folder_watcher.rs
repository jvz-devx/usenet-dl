//! Folder watching for automatic NZB import
//!
//! This module provides filesystem watching capabilities to automatically import NZB files
//! from monitored directories. It supports:
//! - Automatic detection of new `.nzb` files
//! - Configurable post-import actions (delete, move to processed folder, or keep)
//! - Per-folder category assignment
//! - Non-recursive watching (only monitors the specified directory, not subdirectories)
//!
//! # Example
//!
//! ```no_run
//! use usenet_dl::{UsenetDownloader, config::{Config, WatchFolderConfig, WatchFolderAction}};
//! use usenet_dl::folder_watcher::FolderWatcher;
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = Config::default();
//! let downloader = Arc::new(UsenetDownloader::new(config).await?);
//!
//! let watch_config = WatchFolderConfig {
//!     path: "/path/to/watch/folder".into(),
//!     after_import: WatchFolderAction::MoveToProcessed,
//!     category: Some("movies".to_string()),
//!     scan_interval: Duration::from_secs(5),
//! };
//!
//! let mut watcher = FolderWatcher::new(downloader, vec![watch_config])?;
//! watcher.start()?;
//!
//! // Run the watcher (blocks until stopped)
//! watcher.run().await;
//! # Ok(())
//! # }
//! ```

use crate::config::{WatchFolderAction, WatchFolderConfig};
use crate::error::{Error, Result};
use crate::types::DownloadOptions;
use crate::UsenetDownloader;
use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Watches folders for new NZB files and automatically adds them to the download queue
pub struct FolderWatcher {
    /// Filesystem watcher instance
    watcher: RecommendedWatcher,

    /// Channel for receiving filesystem events
    rx: mpsc::UnboundedReceiver<notify::Result<Event>>,

    /// Reference to the downloader for adding NZBs
    downloader: Arc<UsenetDownloader>,

    /// Watched folder configurations
    configs: Vec<WatchFolderConfig>,
}

impl FolderWatcher {
    /// Create a new folder watcher
    ///
    /// # Arguments
    /// * `downloader` - Reference to the UsenetDownloader instance
    /// * `configs` - List of folder configurations to watch
    ///
    /// # Errors
    /// Returns error if the filesystem watcher cannot be initialized
    pub fn new(downloader: Arc<UsenetDownloader>, configs: Vec<WatchFolderConfig>) -> Result<Self> {
        let (tx, rx) = mpsc::unbounded_channel();

        // Create watcher with debouncing to avoid duplicate events
        let watcher = RecommendedWatcher::new(
            move |res| {
                if let Err(e) = tx.send(res) {
                    error!("Failed to send filesystem event: {}", e);
                }
            },
            NotifyConfig::default(),
        )
        .map_err(|e| Error::FolderWatch(e.to_string()))?;

        Ok(Self {
            watcher,
            rx,
            downloader,
            configs,
        })
    }

    /// Start watching all configured folders
    ///
    /// This method registers all folders with the filesystem watcher.
    ///
    /// # Errors
    /// Returns error if any folder cannot be watched (e.g., doesn't exist, permission denied)
    pub fn start(&mut self) -> Result<()> {
        for config in &self.configs {
            // Create directory if it doesn't exist
            if !config.path.exists() {
                std::fs::create_dir_all(&config.path).map_err(|e| {
                    Error::FolderWatch(format!("Failed to create watch folder: {}", e))
                })?;
                info!("Created watch folder: {}", config.path.display());
            }

            // Start watching the directory
            self.watcher
                .watch(&config.path, RecursiveMode::NonRecursive)
                .map_err(|e| Error::FolderWatch(format!("Failed to watch folder: {}", e)))?;

            info!(
                "Watching folder: {} (category: {:?})",
                config.path.display(),
                config.category.as_deref().unwrap_or("default")
            );
        }

        Ok(())
    }

    /// Run the folder watcher event loop
    ///
    /// This is the main event loop that processes filesystem events.
    /// It should be spawned as a tokio task and will run until the channel is closed.
    pub async fn run(mut self) {
        info!("Folder watcher started");

        while let Some(result) = self.rx.recv().await {
            match result {
                Ok(event) => {
                    if let Err(e) = self.handle_event(event).await {
                        error!("Error handling folder event: {}", e);
                    }
                }
                Err(e) => {
                    error!("Filesystem watcher error: {}", e);
                }
            }
        }

        info!("Folder watcher stopped");
    }

    /// Stop watching all folders
    pub fn stop(self) {
        // Dropping the watcher will automatically stop watching
        drop(self.watcher);
        info!("Folder watcher stopped");
    }

    /// Handle a filesystem event
    ///
    /// Processes filesystem events from the watcher and triggers NZB processing for creation/modification events.
    /// Only `.nzb` files are processed; other file types are ignored.
    async fn handle_event(&self, event: Event) -> Result<()> {
        // We only care about file creation events
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                for path in event.paths {
                    if self.is_nzb_file(&path) {
                        self.process_nzb_file(&path).await?;
                    }
                }
            }
            _ => {
                // Ignore other event types (delete, access, etc.)
            }
        }

        Ok(())
    }

    /// Check if a file is an NZB file
    ///
    /// Determines if a file path has the `.nzb` extension (case-insensitive).
    fn is_nzb_file(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("nzb"))
            .unwrap_or(false)
    }

    /// Process a newly detected NZB file
    ///
    /// This method:
    /// 1. Identifies the watch folder configuration for the file
    /// 2. Waits briefly to ensure the file is fully written
    /// 3. Adds the NZB to the download queue with the configured category
    /// 4. Executes the after_import action (delete, move, or keep)
    async fn process_nzb_file(&self, path: &Path) -> Result<()> {
        debug!("Processing NZB file: {}", path.display());

        // Find the config for this folder
        let config = self.find_config_for_path(path)?;

        // Add a small delay to ensure file is fully written
        // Some applications write files in chunks
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Add to download queue
        let options = DownloadOptions {
            category: config.category.clone(),
            ..Default::default()
        };

        match self.downloader.add_nzb(path, options).await {
            Ok(id) => {
                info!(
                    "Added NZB from watch folder: {} (download_id: {}, category: {:?})",
                    path.display(),
                    id,
                    config.category.as_deref().unwrap_or("default")
                );

                // Handle after_import action
                if let Err(e) = self.handle_after_import(path, &config).await {
                    error!(
                        "Failed to handle after_import action for {}: {}",
                        path.display(),
                        e
                    );
                }
            }
            Err(e) => {
                error!(
                    "Failed to add NZB from watch folder {}: {}",
                    path.display(),
                    e
                );
                return Err(e);
            }
        }

        Ok(())
    }

    /// Find the watch folder config that matches this path
    ///
    /// Searches through configured watch folders to find the one containing this file.
    /// Returns the first matching configuration or an error if no match is found.
    fn find_config_for_path(&self, path: &Path) -> Result<&WatchFolderConfig> {
        let parent = path
            .parent()
            .ok_or_else(|| Error::FolderWatch("File has no parent directory".to_string()))?;

        self.configs
            .iter()
            .find(|c| c.path == parent)
            .ok_or_else(|| {
                Error::FolderWatch(format!(
                    "No watch folder config found for: {}",
                    parent.display()
                ))
            })
    }

    /// Handle the after_import action for a processed NZB
    ///
    /// Executes the configured action after successfully adding an NZB to the queue:
    /// - `Delete`: Removes the NZB file
    /// - `MoveToProcessed`: Moves the file to a `processed` subdirectory
    /// - `Keep`: Leaves the file in place and marks it as processed in the database
    async fn handle_after_import(&self, path: &Path, config: &WatchFolderConfig) -> Result<()> {
        match config.after_import {
            WatchFolderAction::Delete => {
                debug!("Deleting NZB file: {}", path.display());
                tokio::fs::remove_file(path)
                    .await
                    .map_err(|e| Error::FolderWatch(format!("Failed to delete file: {}", e)))?;
                info!("Deleted processed NZB: {}", path.display());
            }
            WatchFolderAction::MoveToProcessed => {
                let parent = path.parent().ok_or_else(|| {
                    Error::FolderWatch("File has no parent directory".to_string())
                })?;
                let processed_dir = parent.join("processed");

                // Create processed directory if it doesn't exist
                if !processed_dir.exists() {
                    tokio::fs::create_dir(&processed_dir).await.map_err(|e| {
                        Error::FolderWatch(format!("Failed to create processed directory: {}", e))
                    })?;
                }

                let dest = processed_dir.join(
                    path.file_name()
                        .ok_or_else(|| Error::FolderWatch("File has no filename".to_string()))?,
                );

                debug!("Moving NZB file: {} -> {}", path.display(), dest.display());
                tokio::fs::rename(path, &dest)
                    .await
                    .map_err(|e| Error::FolderWatch(format!("Failed to move file: {}", e)))?;
                info!("Moved processed NZB to: {}", dest.display());
            }
            WatchFolderAction::Keep => {
                // Keep the file in place, but mark as processed in database
                debug!("Keeping NZB file in place: {}", path.display());

                // Store in processed_nzbs table to avoid re-adding
                if let Err(e) = self.downloader.mark_nzb_processed(path).await {
                    warn!("Failed to mark NZB as processed in database: {}", e);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, WatchFolderAction, WatchFolderConfig};
    use tempfile::TempDir;
    use tokio::time::{sleep, Duration};

    async fn create_test_downloader() -> Arc<UsenetDownloader> {
        let temp_dir = TempDir::new().unwrap();
        let mut config = Config::default();
        config.database_path = temp_dir.path().join("test.db");
        config.download.download_dir = temp_dir.path().join("downloads");
        config.download.temp_dir = temp_dir.path().join("temp");

        let downloader = UsenetDownloader::new(config).await.unwrap();
        Arc::new(downloader)
    }

    #[tokio::test]
    async fn test_folder_watcher_creation() {
        let downloader = create_test_downloader().await;
        let temp_dir = TempDir::new().unwrap();

        let config = WatchFolderConfig {
            path: temp_dir.path().to_path_buf(),
            after_import: WatchFolderAction::Delete,
            category: Some("test".to_string()),
            scan_interval: Duration::from_secs(5),
        };

        let watcher = FolderWatcher::new(downloader, vec![config]);
        assert!(watcher.is_ok());
    }

    #[tokio::test]
    async fn test_is_nzb_file() {
        let downloader = create_test_downloader().await;
        let watcher = FolderWatcher::new(downloader, vec![]).unwrap();

        assert!(watcher.is_nzb_file(Path::new("test.nzb")));
        assert!(watcher.is_nzb_file(Path::new("test.NZB")));
        assert!(watcher.is_nzb_file(Path::new("/path/to/file.nzb")));
        assert!(!watcher.is_nzb_file(Path::new("test.txt")));
        assert!(!watcher.is_nzb_file(Path::new("test")));
        assert!(!watcher.is_nzb_file(Path::new("test.zip")));
    }

    #[tokio::test]
    async fn test_folder_watcher_start() {
        let downloader = create_test_downloader().await;
        let temp_dir = TempDir::new().unwrap();
        let watch_path = temp_dir.path().join("watch");

        let config = WatchFolderConfig {
            path: watch_path.clone(),
            after_import: WatchFolderAction::Delete,
            category: Some("test".to_string()),
            scan_interval: Duration::from_secs(5),
        };

        let mut watcher = FolderWatcher::new(downloader, vec![config]).unwrap();

        // Should create directory if it doesn't exist
        assert!(!watch_path.exists());
        watcher.start().unwrap();
        assert!(watch_path.exists());
    }

    #[tokio::test]
    async fn test_find_config_for_path() {
        let downloader = create_test_downloader().await;
        let temp_dir = TempDir::new().unwrap();
        let watch_path = temp_dir.path().join("watch");
        std::fs::create_dir_all(&watch_path).unwrap();

        let config = WatchFolderConfig {
            path: watch_path.clone(),
            after_import: WatchFolderAction::Delete,
            category: Some("test".to_string()),
            scan_interval: Duration::from_secs(5),
        };

        let watcher = FolderWatcher::new(downloader, vec![config]).unwrap();

        let test_file = watch_path.join("test.nzb");
        let found_config = watcher.find_config_for_path(&test_file).unwrap();
        assert_eq!(found_config.path, watch_path);
        assert_eq!(found_config.category.as_deref(), Some("test"));
    }

    #[tokio::test]
    async fn test_folder_watching_with_file_creation() {
        // Create test downloader with temporary directories
        let temp_dir = TempDir::new().unwrap();
        let watch_path = temp_dir.path().join("watch");
        std::fs::create_dir_all(&watch_path).unwrap();

        let mut config = Config::default();
        config.database_path = temp_dir.path().join("test.db");
        config.download.download_dir = temp_dir.path().join("downloads");
        config.download.temp_dir = temp_dir.path().join("temp");

        let downloader = Arc::new(UsenetDownloader::new(config).await.unwrap());

        // Create watch folder configuration with Delete action
        let watch_config = WatchFolderConfig {
            path: watch_path.clone(),
            after_import: WatchFolderAction::Delete,
            category: Some("movies".to_string()),
            scan_interval: Duration::from_secs(1),
        };

        // Create and start folder watcher
        let mut watcher = FolderWatcher::new(downloader.clone(), vec![watch_config]).unwrap();
        watcher.start().unwrap();

        // Spawn watcher task
        let watcher_handle = tokio::spawn(async move {
            watcher.run().await;
        });

        // Give watcher time to start
        sleep(Duration::from_millis(100)).await;

        // Create a valid NZB file in the watch folder
        let nzb_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.com" date="1234567890" subject="test file">
    <groups>
      <group>alt.binaries.test</group>
    </groups>
    <segments>
      <segment bytes="1024" number="1">test-message-id@example.com</segment>
    </segments>
  </file>
</nzb>"#;

        let nzb_path = watch_path.join("test_movie.nzb");
        std::fs::write(&nzb_path, nzb_content).unwrap();

        // Wait for the file to be processed
        // The watcher has a 100ms delay + processing time
        sleep(Duration::from_millis(500)).await;

        // Verify the NZB was deleted (Delete action)
        assert!(
            !nzb_path.exists(),
            "NZB file should have been deleted after import"
        );

        // Verify download was added to queue
        let downloads = downloader.db.list_downloads().await.unwrap();
        assert_eq!(downloads.len(), 1, "Expected 1 download in queue");

        // Verify the download has the correct category
        let download = &downloads[0];
        assert_eq!(download.category.as_deref(), Some("movies"));
        assert!(download.name.contains("test_movie") || download.name.contains("test file"));

        // Cleanup: abort watcher task
        watcher_handle.abort();
        let _ = watcher_handle.await;
    }
}
