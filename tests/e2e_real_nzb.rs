//! End-to-end test with a real NZB file
//!
//! This test downloads actual content from Usenet using a real NZB file.
//! Configure via environment variables:
//!
//! # Required
//! - `TEST_NZB_PATH` - Path to the NZB file to download
//!
//! # Optional
//! - `TEST_DOWNLOAD_TIMEOUT` - Timeout in seconds (default: 300)
//! - `TEST_DOWNLOAD_DIR` - Custom download directory (default: temp dir)
//! - `TEST_SPEED_LIMIT` - Speed limit in bytes/sec (default: unlimited)
//!
//! # Running
//!
//! ```bash
//! # Basic usage
//! TEST_NZB_PATH=/path/to/file.nzb cargo test --test e2e_real_nzb -- --ignored --nocapture
//!
//! # With options
//! TEST_NZB_PATH=/path/to/file.nzb \
//! TEST_DOWNLOAD_TIMEOUT=600 \
//! TEST_SPEED_LIMIT=10000000 \
//!   cargo test --test e2e_real_nzb -- --ignored --nocapture
//! ```

mod common;

use common::{WaitResult, create_live_downloader, has_live_credentials, wait_for_completion};
use serial_test::serial;
use std::path::PathBuf;
use std::time::Duration;
use usenet_dl::{DownloadOptions, Event, Status};

/// Get test configuration from environment
#[allow(dead_code)]
struct TestConfig {
    nzb_path: PathBuf,
    timeout_secs: u64,
    download_dir: Option<PathBuf>,
    speed_limit: Option<u64>,
}

impl TestConfig {
    fn from_env() -> Option<Self> {
        let nzb_path = std::env::var("TEST_NZB_PATH").ok()?;
        let nzb_path = PathBuf::from(nzb_path);

        if !nzb_path.exists() {
            eprintln!("NZB file not found: {:?}", nzb_path);
            return None;
        }

        let timeout_secs = std::env::var("TEST_DOWNLOAD_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);

        let download_dir = std::env::var("TEST_DOWNLOAD_DIR").ok().map(PathBuf::from);

        let speed_limit = std::env::var("TEST_SPEED_LIMIT")
            .ok()
            .and_then(|s| s.parse().ok());

        Some(Self {
            nzb_path,
            timeout_secs,
            download_dir,
            speed_limit,
        })
    }

    fn nzb_name(&self) -> String {
        self.nzb_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "download".to_string())
    }
}

/// Test downloading a real NZB file
#[tokio::test]
#[ignore]
#[serial]
async fn test_real_nzb_download() {
    dotenvy::dotenv().ok();

    // Check credentials
    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    // Get test config
    let config = match TestConfig::from_env() {
        Some(c) => c,
        None => {
            eprintln!("Skipping: TEST_NZB_PATH environment variable not set");
            eprintln!(
                "Usage: TEST_NZB_PATH=/path/to/file.nzb cargo test --test e2e_real_nzb -- --ignored"
            );
            return;
        }
    };

    println!("═══════════════════════════════════════════════════════════");
    println!("  Real NZB Download Test");
    println!("═══════════════════════════════════════════════════════════");
    println!("  NZB file: {:?}", config.nzb_path);
    println!("  Timeout:  {} seconds", config.timeout_secs);
    if let Some(limit) = config.speed_limit {
        println!("  Speed:    {} bytes/sec", limit);
    }
    println!("═══════════════════════════════════════════════════════════");

    // Read NZB content
    let nzb_content = match std::fs::read(&config.nzb_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Failed to read NZB file: {}", e);
            return;
        }
    };

    // Create downloader
    let (downloader, temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    // Set speed limit if configured
    if let Some(limit) = config.speed_limit {
        downloader.set_speed_limit(Some(limit)).await;
        println!("Speed limit set to {} bytes/sec", limit);
    }

    // Add NZB
    let name = config.nzb_name();
    let id = downloader
        .add_nzb_content(&nzb_content, &name, DownloadOptions::default())
        .await
        .expect("Failed to add NZB");

    println!("\nDownload ID: {}", id);
    println!("Download name: {}", name);

    // Subscribe to events for progress reporting
    let mut events = downloader.subscribe();
    let _downloader_clone = downloader.clone();
    let progress_task = tokio::spawn(async move {
        let mut last_percent = -1.0_f32;
        loop {
            match events.recv().await {
                Ok(Event::Downloading {
                    id: _,
                    percent,
                    speed_bps,
                    ..
                }) => {
                    // Only print on significant progress change
                    if (percent - last_percent).abs() >= 1.0 || last_percent < 0.0 {
                        let speed_mbps = speed_bps as f64 / 1_000_000.0;
                        println!("  Progress: {:.1}% ({:.2} MB/s)", percent, speed_mbps);
                        last_percent = percent;
                    }
                }
                Ok(Event::Verifying { id: _, .. }) => {
                    println!("  Status: Verifying (PAR2)...");
                }
                Ok(Event::Extracting { id: _, .. }) => {
                    println!("  Status: Extracting archives...");
                }
                Ok(Event::Complete { id: _, .. }) => {
                    println!("  Status: Complete!");
                    break;
                }
                Ok(Event::Failed { id: _, error, .. }) => {
                    println!("  Status: Failed - {}", error);
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    // Start queue processor
    let _processor = downloader.start_queue_processor();
    println!("\nDownload started...\n");

    // Wait for completion
    let timeout = Duration::from_secs(config.timeout_secs);
    let result = wait_for_completion(&downloader, id, timeout).await;

    // Cancel progress task
    progress_task.abort();

    // Report results
    println!("\n═══════════════════════════════════════════════════════════");
    match result {
        WaitResult::Completed => {
            println!("  Result: SUCCESS");

            // Show downloaded files
            let download_dir = temp_dir.path().join("downloads").join(&name);
            if download_dir.exists() {
                println!("  Location: {:?}", download_dir);
                if let Ok(entries) = std::fs::read_dir(&download_dir) {
                    println!("  Files:");
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        let size_mb = size as f64 / 1_000_000.0;
                        println!(
                            "    - {} ({:.2} MB)",
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            size_mb
                        );
                    }
                }
            }
        }
        WaitResult::Failed(error) => {
            println!("  Result: FAILED");
            println!("  Error: {}", error);
        }
        WaitResult::Timeout => {
            println!("  Result: TIMEOUT");
            println!(
                "  Download did not complete within {} seconds",
                config.timeout_secs
            );

            // Show current status
            if let Ok(downloads) = downloader.db.list_downloads().await
                && let Some(d) = downloads.iter().find(|d| d.id == id)
            {
                println!("  Progress: {:.1}%", d.progress);
                println!("  Status: {:?}", Status::from_i32(d.status));
            }
        }
        WaitResult::ChannelClosed => {
            println!("  Result: ERROR (channel closed)");
        }
    }
    println!("═══════════════════════════════════════════════════════════");

    downloader.shutdown().await.ok();
}

/// Test downloading with pause/resume using a real NZB
#[tokio::test]
#[ignore]
#[serial]
async fn test_real_nzb_pause_resume() {
    dotenvy::dotenv().ok();

    if !has_live_credentials() {
        eprintln!("Skipping: NNTP credentials not found in .env");
        return;
    }

    let config = match TestConfig::from_env() {
        Some(c) => c,
        None => {
            eprintln!("Skipping: TEST_NZB_PATH not set");
            return;
        }
    };

    println!("Testing pause/resume with: {:?}", config.nzb_path);

    let nzb_content = std::fs::read(&config.nzb_path).expect("Failed to read NZB");
    let (downloader, _temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    let name = config.nzb_name();
    let id = downloader
        .add_nzb_content(&nzb_content, &name, DownloadOptions::default())
        .await
        .expect("Failed to add NZB");

    // Start download
    let _processor = downloader.start_queue_processor();
    println!("Download started, waiting 5 seconds before pause...");

    // Wait for download to start
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Pause
    println!("Pausing download...");
    match downloader.pause(id).await {
        Ok(_) => println!("Paused successfully"),
        Err(e) => {
            println!("Pause failed (download may have finished or failed): {}", e);
            downloader.shutdown().await.ok();
            return;
        }
    }

    // Check status
    if let Ok(downloads) = downloader.db.list_downloads().await
        && let Some(d) = downloads.iter().find(|d| d.id == id)
    {
        println!(
            "Status after pause: {:?}, Progress: {:.1}%",
            Status::from_i32(d.status),
            d.progress
        );
    }

    // Wait a bit
    println!("Waiting 3 seconds while paused...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Resume
    println!("Resuming download...");
    match downloader.resume(id).await {
        Ok(_) => println!("Resumed successfully"),
        Err(e) => println!("Resume failed: {}", e),
    }

    // Wait a bit more to see progress
    tokio::time::sleep(Duration::from_secs(5)).await;

    if let Ok(downloads) = downloader.db.list_downloads().await
        && let Some(d) = downloads.iter().find(|d| d.id == id)
    {
        println!(
            "Status after resume: {:?}, Progress: {:.1}%",
            Status::from_i32(d.status),
            d.progress
        );
    }

    println!("Pause/resume test complete");
    downloader.shutdown().await.ok();
}

/// Test that shows download info without actually downloading
#[tokio::test]
#[ignore]
#[serial]
async fn test_real_nzb_info() {
    dotenvy::dotenv().ok();

    let config = match TestConfig::from_env() {
        Some(c) => c,
        None => {
            eprintln!("Skipping: TEST_NZB_PATH not set");
            return;
        }
    };

    println!("═══════════════════════════════════════════════════════════");
    println!("  NZB File Information");
    println!("═══════════════════════════════════════════════════════════");
    println!("  Path: {:?}", config.nzb_path);

    let content = std::fs::read_to_string(&config.nzb_path).expect("Failed to read NZB");

    // Count files and segments
    let file_count = content.matches("<file ").count();
    let segment_count = content.matches("<segment ").count();

    // Estimate size from segment bytes
    let mut total_bytes: u64 = 0;
    for cap in content.split("bytes=\"").skip(1) {
        if let Some(end) = cap.find('"')
            && let Ok(bytes) = cap[..end].parse::<u64>()
        {
            total_bytes += bytes;
        }
    }

    println!("  Files: {}", file_count);
    println!("  Segments: {}", segment_count);
    println!(
        "  Estimated size: {:.2} MB",
        total_bytes as f64 / 1_000_000.0
    );

    // Check for password
    if content.contains("<meta type=\"password\">") {
        println!("  Password: Yes (embedded in NZB)");
    }

    // Check for PAR2
    if content.contains(".par2") {
        println!("  PAR2 files: Yes");
    }

    println!("═══════════════════════════════════════════════════════════");
}
