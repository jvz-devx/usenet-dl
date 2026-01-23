//! Simple speedtest benchmark for usenet-dl
//!
//! Usage: cargo run --release --example speedtest

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Notify;
use usenet_dl::{Config, DownloadOptions, Event, ServerConfig, UsenetDownloader};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    // Get NZB path
    let nzb_path = std::env::var("TEST_NZB_PATH").unwrap_or_else(|_| {
        "/home/jens/Documents/source/usenet-dl/Fallout.S02E06.The.Other.Player.2160p.AMZN.WEB-DL.DDP5.1.Atmos.DV.HDR10H.265-Kitsune.nzb".to_string()
    });

    // Load config from env
    let host = std::env::var("NNTP_HOST").expect("NNTP_HOST not set");
    let port: u16 = std::env::var("NNTP_PORT_SSL")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(563);
    let username = std::env::var("NNTP_USERNAME").expect("NNTP_USERNAME not set");
    let password = std::env::var("NNTP_PASSWORD").expect("NNTP_PASSWORD not set");
    let connections: usize = std::env::var("NNTP_CONNECTIONS")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(50);

    println!("═══════════════════════════════════════════════════════════");
    println!("  usenet-dl Speedtest");
    println!("═══════════════════════════════════════════════════════════");
    println!("  Server: {}:{}", host, port);
    println!("  Connections: {}", connections);
    println!("  NZB: {}", nzb_path);
    println!("═══════════════════════════════════════════════════════════");

    // Create temp directory
    let temp_dir = tempfile::tempdir()?;
    let download_dir = temp_dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir)?;

    let config = Config {
        servers: vec![ServerConfig {
            host,
            port,
            tls: true,
            username: Some(username),
            password: Some(password),
            connections,
            priority: 0,
            pipeline_depth: 10,
        }],
        database_path: temp_dir.path().join("test.db"),
        download_dir: download_dir.clone(),
        temp_dir: temp_dir.path().join("temp"),
        max_concurrent_downloads: 1,
        ..Default::default()
    };

    let downloader = Arc::new(UsenetDownloader::new(config).await?);

    // Read NZB content
    let nzb_content = std::fs::read(&nzb_path)?;
    let name = std::path::Path::new(&nzb_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());

    // Add NZB
    let id = downloader
        .add_nzb_content(&nzb_content, &name, DownloadOptions::default())
        .await?;

    println!("\nDownload ID: {}", id);

    // Track progress via events
    let done = Arc::new(Notify::new());
    let done_clone = done.clone();
    let download_dir_clone = download_dir.clone();

    let mut events = downloader.subscribe();
    let progress_task = tokio::spawn(async move {
        let mut last_percent = -1.0_f32;
        loop {
            match events.recv().await {
                Ok(Event::Downloading {
                    id: _,
                    percent,
                    speed_bps,
                }) => {
                    if (percent - last_percent).abs() >= 5.0 || last_percent < 0.0 {
                        let speed_mbps = speed_bps as f64 / 1_000_000.0;
                        println!("  Progress: {:5.1}%  Speed: {:7.2} MB/s", percent, speed_mbps);
                        last_percent = percent;
                    }
                }
                Ok(Event::Complete { id: _, path }) => {
                    println!("  Complete! Path: {:?}", path);
                    done_clone.notify_one();
                    break;
                }
                Ok(Event::Failed { id: _, error, .. }) => {
                    eprintln!("  Failed: {}", error);
                    done_clone.notify_one();
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    // Start download and measure time
    let start = Instant::now();
    let _processor = downloader.start_queue_processor();

    // Wait for completion with timeout
    tokio::select! {
        _ = done.notified() => {}
        _ = tokio::time::sleep(std::time::Duration::from_secs(600)) => {
            eprintln!("Timeout!");
        }
    }

    let elapsed = start.elapsed();
    progress_task.abort();

    // Calculate results from disk usage
    let disk_bytes: u64 = walkdir::WalkDir::new(&download_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();

    let secs = elapsed.as_secs_f64();
    let speed_mbps = if secs > 0.0 {
        disk_bytes as f64 / secs / 1_000_000.0
    } else {
        0.0
    };

    println!("\n═══════════════════════════════════════════════════════════");
    println!("  Results");
    println!("═══════════════════════════════════════════════════════════");
    println!("  Time:       {:.2} seconds", secs);
    println!("  On disk:    {:.2} MB", disk_bytes as f64 / 1_000_000.0);
    println!("  Speed:      {:.2} MB/s", speed_mbps);
    println!("═══════════════════════════════════════════════════════════");

    downloader.shutdown().await.ok();
    Ok(())
}
