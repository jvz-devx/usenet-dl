use super::*;
use std::time::{Duration, Instant};
use tempfile::tempdir;

/// Helper to create a test UsenetDownloader instance with a persistent database
/// Returns the downloader and the tempdir (which must be kept alive)
async fn create_test_downloader() -> (UsenetDownloader, tempfile::TempDir) {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let mut config = Config::default();
    config.persistence.database_path = db_path;
    config.servers = vec![]; // No servers for testing
    config.download.max_concurrent_downloads = 3;

    // Initialize database
    let db = Database::new(&config.persistence.database_path)
        .await
        .unwrap();

    // Create broadcast channel
    let (event_tx, _rx) = tokio::sync::broadcast::channel(1000);

    // No NNTP pools since we have no servers
    let nntp_pools = Vec::new();

    // Create priority queue
    let queue = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BinaryHeap::new()));

    // Create semaphore
    let concurrent_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
        config.download.max_concurrent_downloads,
    ));

    // Create active downloads tracking map
    let active_downloads =
        std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

    // Create speed limiter with configured limit
    let speed_limiter = speed_limiter::SpeedLimiter::new(config.download.speed_limit_bps);

    // Create config Arc early so we can share it
    let config_arc = std::sync::Arc::new(config.clone());

    // Initialize runtime-mutable categories from config
    let categories = std::sync::Arc::new(tokio::sync::RwLock::new(
        config.persistence.categories.clone(),
    ));

    // Initialize runtime-mutable schedule rules (empty for tests)
    let schedule_rules = std::sync::Arc::new(tokio::sync::RwLock::new(vec![]));
    let next_schedule_rule_id = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));

    // Use NoOp parity handler for tests (no external binary required)
    let parity_handler: std::sync::Arc<dyn crate::ParityHandler> =
        std::sync::Arc::new(crate::NoOpParityHandler);

    // Wrap database in Arc for sharing
    let db_arc = std::sync::Arc::new(db);

    // Create post-processing pipeline executor
    let post_processor = std::sync::Arc::new(post_processing::PostProcessor::new(
        event_tx.clone(),
        config_arc.clone(),
        parity_handler.clone(),
        db_arc.clone(),
    ));

    // Group queue and download state
    let queue_state = crate::downloader::QueueState {
        queue,
        concurrent_limit,
        active_downloads,
        accepting_new: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
    };

    // Group runtime configuration
    let runtime_config = crate::downloader::RuntimeConfig {
        categories,
        schedule_rules,
        next_schedule_rule_id,
    };

    // Group post-processing pipeline
    let processing = crate::downloader::ProcessingPipeline {
        post_processor,
        parity_handler,
    };

    let downloader = UsenetDownloader {
        db: db_arc,
        event_tx,
        config: config_arc,
        nntp_pools: std::sync::Arc::new(nntp_pools),
        speed_limiter,
        queue_state,
        runtime_config,
        processing,
    };

    (downloader, temp_dir)
}

/// Sample NZB content for testing
const SAMPLE_NZB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <head>
<meta type="title">Test Download</meta>
<meta type="password">testpass123</meta>
<meta type="category">movies</meta>
  </head>
  <file poster="user@example.com" date="1234567890" subject="test.file.rar [1/2]">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="768000" number="1">part1of2@example.com</segment>
  <segment bytes="512000" number="2">part2of2@example.com</segment>
</segments>
  </file>
</nzb>"#;

mod control;
mod disk_space;
mod duplicates;
mod lifecycle;
mod nzb;
mod post_process;
mod queue;
mod rss;
mod scheduler;
mod scripts;
mod server;
mod speed;
mod webhooks;
