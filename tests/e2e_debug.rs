//! Quick debug test to understand download failures

mod common;

use common::{create_live_downloader, has_live_credentials};
use serial_test::serial;
use std::time::Duration;
use usenet_dl::{DownloadOptions, Event, Status};

#[tokio::test]
#[ignore]
#[serial]
async fn test_debug_download() {
    dotenvy::dotenv().ok();

    if !has_live_credentials() {
        eprintln!("No credentials");
        return;
    }

    let nzb_path = match std::env::var("TEST_NZB_PATH") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("TEST_NZB_PATH not set");
            return;
        }
    };

    println!("Reading NZB: {}", nzb_path);
    let nzb_content = std::fs::read(&nzb_path).expect("Failed to read NZB");
    println!("NZB size: {} bytes", nzb_content.len());

    let (downloader, _temp_dir) = create_live_downloader()
        .await
        .expect("Failed to create downloader");

    println!("Downloader created, adding NZB...");

    let id = downloader
        .add_nzb_content(&nzb_content, "debug_test", DownloadOptions::default())
        .await
        .expect("Failed to add NZB");

    println!("NZB added with ID: {}", id);

    // Check initial status
    if let Ok(downloads) = downloader.db.list_downloads().await {
        for d in &downloads {
            println!(
                "Initial - ID: {}, Status: {:?}, Progress: {}%",
                d.id,
                Status::from_i32(d.status),
                d.progress
            );
        }
    }

    // Subscribe to ALL events
    let mut events = downloader.subscribe();

    // Start queue processor
    println!("Starting queue processor...");
    let _processor = downloader.start_queue_processor();

    // Wait and collect events
    println!("Waiting for events (30 seconds max)...");
    let timeout = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match events.recv().await {
                Ok(event) => {
                    println!("EVENT: {:?}", event);
                    match &event {
                        Event::Complete { .. } | Event::Failed { .. } => break,
                        _ => {}
                    }
                }
                Err(e) => {
                    println!("Event recv error: {:?}", e);
                    break;
                }
            }
        }
    })
    .await;

    if timeout.is_err() {
        println!("Timeout waiting for events");
    }

    // Check final status
    println!("\nFinal status:");
    if let Ok(downloads) = downloader.db.list_downloads().await {
        for d in &downloads {
            println!("  ID: {}", d.id);
            println!("  Status: {:?}", Status::from_i32(d.status));
            println!("  Progress: {}%", d.progress);
            println!("  Error: {:?}", d.error_message);
        }
    }

    downloader.shutdown().await.ok();
}
