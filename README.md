# usenet-dl

A high-performance, highly configurable backend library for building SABnzbd/NZBGet-like Usenet download applications in Rust.

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Rust Version: 1.93+](https://img.shields.io/badge/rust-1.93%2B-orange.svg)](https://www.rust-lang.org)

## Features

### Core Capabilities

- **Queue Management**: Priority-based download queue with pause/resume/cancel
- **Resume Support**: Article-level download tracking, survives crashes and restarts
- **Parallel Downloads**: Concurrent article fetching using all configured connections (~N× speedup with N connections)
- **Speed Limiting**: Global bandwidth control with token bucket algorithm
- **Retry Logic**: Exponential backoff with jitter for transient failures
- **Event System**: Real-time events via `tokio::broadcast` channels
- **Graceful Shutdown**: Signal handling with state preservation

### Post-Processing

- **Archive Extraction**: RAR, 7z, and ZIP with password support
- **Nested Extraction**: Automatic recursive extraction (configurable depth)
- **Password Management**: Multi-source passwords (per-download, NZB metadata, global file, cache)
- **Deobfuscation**: Automatic detection and renaming of obfuscated filenames
- **File Collision Handling**: Rename, overwrite, or skip on conflicts
- **Smart Cleanup**: Remove .par2, .nzb, .sfv, sample folders, and archives after extraction

### REST API

- **OpenAPI 3.1 Compliant**: Full schema generation with utoipa
- **Swagger UI**: Interactive API documentation at `/swagger-ui`
- **Server-Sent Events**: Real-time updates via `/events` endpoint
- **37 Endpoints**: Complete CRUD for downloads, queue, history, config, categories, RSS, scheduler
- **Authentication**: Optional API key protection
- **CORS**: Configurable cross-origin support for frontend development
- **Rate Limiting**: Optional per-IP rate limiting (disabled by default)

### Automation

- **Folder Watching**: Auto-import NZB files from watched directories
- **URL Fetching**: Download NZBs directly from HTTP(S) URLs
- **RSS Feed Monitoring**: Automatic download with regex filters and scheduling
- **Time-Based Scheduler**: Speed limits and pause/resume based on time rules
- **Duplicate Detection**: Hash and name-based duplicate checking

### Notifications

- **Webhooks**: HTTP POST on download events (complete, failed, queued)
- **Script Execution**: Run external scripts with environment variables
- **Category Scripts**: Per-category script configuration
- **Disk Space Checks**: Pre-download validation with configurable buffer
- **Server Health Checks**: Test NNTP server connectivity and capabilities

## Design Philosophy

**usenet-dl is a library-first backend.** No CLI, no UI - just a solid Rust crate that frontend applications can embed.

- **Highly Configurable**: Almost every behavior can be customized
- **Sensible Defaults**: Works out of the box with minimal configuration
- **Event-Driven**: Subscribe to events, no polling required
- **Async Native**: Built on tokio for efficient concurrent operations

## Architecture

```
┌─────────────────────────────────────────┐
│  Spotnet App    │  SABnzbd Alternative  │
├─────────────────┴───────────────────────┤
│              usenet-dl                  │
│   (Queue, Post-processing, API, DB)     │
├─────────────────────────────────────────┤
│              nntp-rs                    │
│   (NNTP, NZB parsing, yEnc, PAR2)      │
└─────────────────────────────────────────┘
```

### Responsibility Split

**usenet-dl handles:**
- Download queue management and persistence (SQLite)
- Post-processing pipeline (verify, repair, extract, rename, cleanup)
- Archive extraction (RAR/7z/ZIP) with password management
- File organization and collision handling
- REST API with OpenAPI documentation
- Event broadcasting to subscribers
- Folder watching and RSS feed monitoring
- Scheduler for time-based rules
- External notifications (webhooks, scripts)
- Disk space checking and health monitoring

**nntp-rs handles:**
- NNTP protocol implementation (RFC 3977)
- NZB file parsing
- yEnc decoding
- PAR2 verification and repair
- Connection pooling

## Installation

**Not yet published to crates.io**

Add as a path or git dependency:

```toml
[dependencies]
usenet-dl = { path = "../usenet-dl" }
# or
usenet-dl = { git = "https://github.com/jvz-devx/usenet-dl" }
```

### Requirements

- Rust 1.93 or later
- SQLite (embedded via sqlx)
- Optional: unrar command-line tool for RAR extraction
- Optional: 7z command-line tool for 7z extraction

## Quick Start

### Basic Usage

```rust
use usenet_dl::config::{Config, DownloadConfig, ServerConfig};
use usenet_dl::{UsenetDownloader, DownloadOptions, Event, Priority};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure NNTP server
    let config = Config {
        servers: vec![
            ServerConfig {
                host: "news.example.com".to_string(),
                port: 563,
                tls: true,
                username: Some("user".to_string()),
                password: Some("pass".to_string()),
                connections: 10,
                priority: 0,
                pipeline_depth: 10,
            }
        ],
        download: DownloadConfig {
            download_dir: "downloads".into(),
            temp_dir: "temp".into(),
            ..Default::default()
        },
        ..Default::default()
    };

    // Create downloader
    let downloader = UsenetDownloader::new(config).await?;

    // Subscribe to events
    let mut events = downloader.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                Event::Downloading { id, percent, speed_bps, .. } => {
                    println!("Download {}: {:.1}% @ {} MB/s",
                        id, percent, speed_bps / 1_000_000);
                }
                Event::Complete { id, path } => {
                    println!("Download {} complete: {:?}", id, path);
                }
                Event::Failed { id, stage, error, .. } => {
                    eprintln!("Download {} failed at {:?}: {}", id, stage, error);
                }
                _ => {}
            }
        }
    });

    // Add NZB download
    let id = downloader.add_nzb(
        "file.nzb".as_ref(),
        DownloadOptions {
            category: Some("movies".into()),
            priority: Priority::Normal,
            ..Default::default()
        }
    ).await?;

    println!("Queued download: {}", id);

    // Control downloads
    downloader.pause(id).await?;
    downloader.resume(id).await?;
    downloader.set_speed_limit(Some(10_000_000)).await; // 10 MB/s

    Ok(())
}
```

### REST API

Start the API server:

```rust
use std::sync::Arc;
use usenet_dl::api::start_api_server;
use usenet_dl::config::{Config, DownloadConfig, ServerConfig};

let config = Config { /* ... */ };
let downloader = Arc::new(UsenetDownloader::new(config.clone()).await?);
let config = Arc::new(config);
start_api_server(downloader, config).await?;
```

The API will be available at `http://localhost:6789` with Swagger UI at `http://localhost:6789/swagger-ui`.

#### Example API Calls

```bash
# Add NZB from URL
curl -X POST http://localhost:6789/api/v1/downloads/url \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/file.nzb", "options": {"category": "movies"}}'

# List all downloads
curl http://localhost:6789/api/v1/downloads

# Get download status
curl http://localhost:6789/api/v1/downloads/1

# Pause download
curl -X POST http://localhost:6789/api/v1/downloads/1/pause

# Set speed limit to 10 MB/s
curl -X PUT http://localhost:6789/api/v1/config/speed-limit \
  -H "Content-Type: application/json" \
  -d '{"limit_bps": 10485760}'

# Stream real-time events
curl -N http://localhost:6789/api/v1/events
```

### Event Streaming

Multiple subscribers can independently receive events:

```rust
// UI subscriber
let mut ui_events = downloader.subscribe();
tokio::spawn(async move {
    while let Ok(event) = ui_events.recv().await {
        update_ui(event);
    }
});

// Logging subscriber
let mut log_events = downloader.subscribe();
tokio::spawn(async move {
    while let Ok(event) = log_events.recv().await {
        tracing::info!(?event, "download event");
    }
});

// Notification subscriber
let mut notify_events = downloader.subscribe();
tokio::spawn(async move {
    while let Ok(event) = notify_events.recv().await {
        if matches!(event, Event::Complete { .. }) {
            send_push_notification(event);
        }
    }
});
```

## Configuration

All settings have sensible defaults. Only NNTP server configuration is required.

### Default Settings

| Setting | Default | Rationale |
|---------|---------|-----------|
| **Download directory** | `./downloads` | Current directory, easy to find |
| **Temp directory** | `./temp` | Separate from final downloads |
| **Concurrent downloads** | 3 | Balanced throughput without overwhelming |
| **Speed limit** | Unlimited | Users expect full speed by default |
| **Post-processing** | Unpack + Cleanup | Most users want ready-to-use files |
| **Failed download action** | Keep files | Don't delete potentially recoverable data |
| **File collision** | Rename (add number) | Never lose data silently |
| **Nested extraction depth** | 2 levels | Handle common archive-in-archive |
| **Deobfuscation** | Enabled | Most users want readable names |
| **Duplicate detection** | Warn only | Alert but don't block |
| **Try empty password** | Yes | Common for public releases |
| **Delete samples** | Yes | Usually unwanted |
| **Disk space check** | Enabled, 1GB buffer | Prevent failed extractions |
| **Retry attempts** | 5 with exponential backoff | Resilient to transient failures |
| **Pipeline depth** | 10 articles per connection | Reduces round-trip latency overhead |
| **API bind address** | 127.0.0.1:6789 | Localhost only for security |
| **API authentication** | None | Easy local development |
| **CORS** | Enabled for all origins | Easy frontend development |
| **Swagger UI** | Enabled | Self-documenting API |
| **Rate limiting** | Disabled | Trust local network |

### Configuration Example

```rust
use usenet_dl::config::*;
use std::time::Duration;

let config = Config {
    // Required: NNTP servers
    servers: vec![
        ServerConfig {
            host: "news.example.com".to_string(),
            port: 563,
            tls: true,
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            connections: 10,  // More connections = faster downloads (10 connections ≈ 10× speed)
            priority: 0, // Lower = tried first
            pipeline_depth: 10,  // Number of articles to pipeline per connection (reduces latency)
        }
    ],

    // Optional: Download settings
    download: DownloadConfig {
        download_dir: "/mnt/media/downloads".into(),
        temp_dir: "/mnt/temp".into(),
        max_concurrent_downloads: 3,
        speed_limit_bps: Some(10_000_000), // 10 MB/s
        default_post_process: PostProcess::UnpackAndCleanup,
        delete_samples: true,
        file_collision: FileCollisionAction::Rename,
        ..Default::default()
    },

    // Optional: Tools configuration
    tools: ToolsConfig {
        password_file: Some("passwords.txt".into()),
        try_empty_password: true,
        ..Default::default()
    },

    // Optional: Processing settings
    processing: ProcessingConfig {
        extraction: ExtractionConfig {
            max_recursion_depth: 2,
            archive_extensions: vec!["rar".into(), "7z".into(), "zip".into()],
        },
        disk_space: DiskSpaceConfig {
            enabled: true,
            min_free_space: 1024 * 1024 * 1024, // 1 GB
            size_multiplier: 2.5,
        },
        retry: RetryConfig {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            jitter: true,
        },
        ..Default::default()
    },

    // Optional: Notifications
    notifications: NotificationConfig {
        webhooks: vec![
            WebhookConfig {
                url: "https://api.example.com/webhook".to_string(),
                events: vec![WebhookEvent::OnComplete, WebhookEvent::OnFailed],
                auth_header: Some("Bearer token123".into()),
                timeout: Duration::from_secs(30),
            }
        ],
        scripts: vec![
            ScriptConfig {
                path: "/usr/local/bin/notify.sh".into(),
                events: vec![ScriptEvent::OnComplete],
                timeout: Duration::from_secs(300),
            }
        ],
    },

    // Optional: Persistence (database, scheduler, categories)
    persistence: PersistenceConfig {
        database_path: "usenet-dl.db".into(),
        schedule_rules: vec![
            ScheduleRule {
                name: "Night owl".into(),
                days: vec![], // All days
                start_time: "00:00".to_string(),
                end_time: "06:00".to_string(),
                action: ScheduleAction::Unlimited,
                enabled: true,
            }
        ],
        categories: [
            ("movies".to_string(), CategoryConfig {
                destination: "/mnt/media/movies".into(),
                post_process: None, // Use default
                scripts: vec![],
            }),
            ("tv".to_string(), CategoryConfig {
                destination: "/mnt/media/tv".into(),
                post_process: Some(PostProcess::UnpackAndCleanup),
                scripts: vec![],
            }),
        ].into_iter().collect(),
        ..Default::default()
    },

    // Optional: Automation
    automation: AutomationConfig {
        deobfuscation: DeobfuscationConfig {
            enabled: true,
            min_length: 12,
        },
        watch_folders: vec![
            WatchFolderConfig {
                path: "/mnt/nzb-drop".into(),
                after_import: WatchFolderAction::MoveToProcessed,
                category: Some("movies".into()),
                scan_interval: Duration::from_secs(5),
            }
        ],
        rss_feeds: vec![
            RssFeedConfig {
                url: "https://indexer.example/rss".to_string(),
                check_interval: Duration::from_secs(900), // 15 minutes
                category: Some("tv".into()),
                filters: vec![
                    RssFilter {
                        name: "TV Shows".into(),
                        include: vec![r"S\d{2}E\d{2}".into()],
                        exclude: vec![r"(?i)cam|ts|screener".into()],
                        min_size: Some(100_000_000), // 100 MB
                        max_size: Some(5_000_000_000), // 5 GB
                        max_age: None,
                    }
                ],
                auto_download: true,
                priority: Priority::Normal,
                enabled: true,
            }
        ],
        ..Default::default()
    },

    // Optional: API server
    server: ServerIntegrationConfig {
        api: ApiConfig {
            bind_address: "127.0.0.1:6789".parse().unwrap(),
            api_key: Some("secret".to_string()),
            cors_enabled: true,
            cors_origins: vec!["http://localhost:3000".into()],
            swagger_ui: true,
            rate_limit: RateLimitConfig {
                enabled: false,
                requests_per_second: 100,
                burst_size: 200,
                exempt_paths: vec!["/api/v1/events".into()],
                exempt_ips: vec!["127.0.0.1".parse().unwrap()],
            },
        },
    },
};
```

## REST API Documentation

### Base URL

`http://localhost:6789/api/v1`

### Endpoints

#### Queue Management

- `GET /downloads` - List all downloads
- `GET /downloads/:id` - Get download details
- `POST /downloads` - Add NZB from file upload (multipart/form-data)
- `POST /downloads/url` - Add NZB from URL
- `POST /downloads/:id/pause` - Pause download
- `POST /downloads/:id/resume` - Resume download
- `DELETE /downloads/:id` - Delete/cancel download
- `PATCH /downloads/:id/priority` - Set download priority
- `POST /downloads/:id/reprocess` - Re-run post-processing
- `POST /downloads/:id/reextract` - Re-run extraction only

#### Queue-Wide Operations

- `POST /queue/pause` - Pause all downloads
- `POST /queue/resume` - Resume all downloads
- `GET /queue/stats` - Get queue statistics

#### History

- `GET /history` - Get download history (with pagination)
- `DELETE /history` - Clear history (with filters)

#### Configuration

- `GET /config` - Get current config (sensitive fields redacted)
- `PATCH /config` - Update config
- `GET /config/speed-limit` - Get current speed limit
- `PUT /config/speed-limit` - Set speed limit
- `GET /categories` - List categories
- `PUT /categories/:name` - Create/update category
- `DELETE /categories/:name` - Delete category

#### RSS Feeds

- `GET /rss` - List RSS feeds
- `POST /rss` - Add RSS feed
- `PUT /rss/:id` - Update RSS feed
- `DELETE /rss/:id` - Delete RSS feed
- `POST /rss/:id/check` - Force check feed now

#### Scheduler

- `GET /scheduler` - Get schedule rules
- `POST /scheduler` - Add schedule rule
- `PUT /scheduler/:id` - Update schedule rule
- `DELETE /scheduler/:id` - Delete schedule rule

#### Server Management

- `POST /servers/test` - Test server connection
- `GET /servers/test` - Test all configured servers

#### System

- `GET /health` - Health check
- `GET /openapi.json` - OpenAPI specification
- `GET /events` - Server-Sent Events stream (real-time updates)
- `POST /shutdown` - Graceful shutdown

### Interactive Documentation

Visit `http://localhost:6789/swagger-ui` for interactive API documentation with "Try it out" functionality.

## Event Types

The library emits the following events via `tokio::broadcast`:

```rust
pub enum Event {
    // Queue events
    Queued { id: DownloadId, name: String },
    Removed { id: DownloadId },

    // Download progress
    Downloading {
        id: DownloadId, percent: f32, speed_bps: u64,
        failed_articles: Option<u64>, total_articles: Option<u64>,
        health_percent: Option<f32>,
    },
    DownloadComplete {
        id: DownloadId,
        articles_failed: Option<u64>, articles_total: Option<u64>,
    },
    DownloadFailed {
        id: DownloadId, error: String,
        articles_succeeded: Option<u64>, articles_failed: Option<u64>,
        articles_total: Option<u64>,
    },

    // Duplicate detection
    DuplicateDetected { id: DownloadId, name: String, method: String, existing_name: String },

    // Post-processing stages
    Verifying { id: DownloadId },
    VerifyComplete { id: DownloadId, damaged: bool },
    Repairing { id: DownloadId, blocks_needed: u32, blocks_available: u32 },
    RepairComplete { id: DownloadId, success: bool },
    RepairSkipped { id: DownloadId, reason: String },
    Extracting { id: DownloadId, archive: String, percent: f32 },
    ExtractComplete { id: DownloadId },
    Moving { id: DownloadId, destination: PathBuf },
    Cleaning { id: DownloadId },

    // Final states
    Complete { id: DownloadId, path: PathBuf },
    Failed { id: DownloadId, stage: Stage, error: String, files_kept: bool },

    // Global events
    SpeedLimitChanged { limit_bps: Option<u64> },
    QueuePaused,
    QueueResumed,
    Shutdown,

    // Notifications
    WebhookFailed { url: String, error: String },
    ScriptFailed { script: PathBuf, exit_code: Option<i32> },
}
```

## Development

### Setup with Nix

```bash
nix-shell --run "cargo build"
nix-shell --run "cargo test"
```

### Setup without Nix

```bash
# Install system dependencies (Ubuntu/Debian)
sudo apt-get install libssl-dev pkg-config sqlite3

# Or on macOS
brew install openssl sqlite

# Or on Fedora/RHEL
sudo dnf install openssl-devel sqlite-devel

# Build and test
cargo build
cargo test
```

### Useful Commands

```bash
nix-shell --run "cargo check"              # Fast syntax/type checking
nix-shell --run "cargo build"              # Build the library
nix-shell --run "cargo test"               # Run all tests
nix-shell --run "cargo test -- --nocapture" # Run tests with output
nix-shell --run "cargo clippy --all-targets" # Lint checks
nix-shell --run "cargo fmt --all"           # Format code
nix-shell --run "cargo doc --open"          # Build and view documentation
```

### Running the REST API in Development

```bash
# Set up environment
cp .env.example .env
# Edit .env with your NNTP credentials

# Run with logging
nix-shell --run "RUST_LOG=debug cargo run --example rest_api_server"

# Or use the test script
docs/test_api.sh
```

### Manual Testing Guides

The project includes comprehensive manual testing documentation in `tests/manual/`:

- [API Testing](tests/manual/api-testing.md) - REST API testing with curl and Postman
- [Server Testing](tests/manual/server-testing.md) - NNTP server health check testing
- [RSS Testing](tests/manual/rss-testing.md) - RSS feed integration testing
- `docs/test_api.sh` - Automated API testing script
- `docs/postman_collection.json` - Postman collection for API testing

## Testing

The project has comprehensive test coverage:

- **Core library tests** - Queue, persistence, events, retry, shutdown
- **Post-processing tests** - Extraction, deobfuscation, cleanup
- **REST API tests** - All endpoints with integration tests
- **Automation tests** - RSS, scheduler, folder watching, duplicates
- **Notification tests** - Webhooks, scripts, disk space, health checks

Run tests:

```bash
# All tests
nix-shell --run "cargo test"

# Specific module
nix-shell --run "cargo test db::tests"
nix-shell --run "cargo test api::tests"

# Integration tests
nix-shell --run "cargo test --test integration"

# With output
nix-shell --run "cargo test -- --nocapture --test-threads=1"
```

## Dependencies

### Core Dependencies

- **nntp-rs** - NNTP client and NZB parsing (local path dependency)
- **tokio** - Async runtime
- **sqlx** - SQLite persistence with compile-time query checking
- **axum** - REST API framework
- **utoipa** - OpenAPI documentation generation
- **tower-http** - HTTP middleware (CORS, tracing)
- **tower-governor** - Rate limiting

### Archive Extraction

- **unrar** - RAR archive extraction
- **sevenz-rust** - 7z archive extraction with AES-256 support
- **zip** - ZIP archive extraction

### Utilities

- **reqwest** - HTTP client for webhooks, URL fetching, RSS
- **notify** - File system watching
- **chrono** - Date/time handling
- **sha2** - Hashing for duplicate detection
- **regex** - Regular expressions for RSS filters
- **rand** - Random number generation for retry jitter

See [Cargo.toml](Cargo.toml) for the complete dependency list (40+ crates).

## Database Schema

The library uses SQLite for persistence with the following tables:

- **downloads** - Download queue with status, progress, metadata
- **download_articles** - Article-level tracking for resume support
- **passwords** - Cached successful passwords per download
- **processed_nzbs** - Tracking for watch folder "Keep" action
- **history** - Completed download history
- **rss_feeds** - RSS feed configurations
- **rss_filters** - Per-feed filter rules
- **rss_seen** - Tracking seen RSS items
- **schedule_rules** - Time-based scheduler rules

Database migrations are handled automatically on startup.

## Known Issues & Limitations

- **PAR2 repair not yet implemented in nntp-rs** - Verification works, repair planned
- **Archive extraction requires external tools** - unrar and 7z must be in PATH for RAR/7z support
- **No Windows testing yet** - Primarily developed and tested on Linux/macOS

## Documentation

Comprehensive documentation is available in the `docs/` directory:

- **[Getting Started](docs/getting-started.md)** - Installation, setup, and basic usage
- **[Configuration](docs/configuration.md)** - Complete configuration options reference
- **[API Reference](docs/api-reference.md)** - REST API endpoints and examples
- **[Architecture](docs/architecture.md)** - System design and module overview
- **[Post-Processing](docs/post-processing.md)** - Extraction, deobfuscation, and cleanup
- **[Contributing](docs/contributing.md)** - Development guidelines and workflow
- **API Documentation** - Run `cargo doc --open` for inline Rustdoc

### Manual Testing

See [tests/manual/](tests/manual/) for comprehensive testing guides.

## Contributing

Contributions are welcome! Please see [docs/contributing.md](docs/contributing.md) for development guidelines and workflow.

**Quick start:**

```bash
# Create feature branch
git checkout -b feature/my-feature

# Make changes and test
nix-shell --run "cargo test"
nix-shell --run "cargo clippy --all-targets"

# Commit with descriptive message
git commit -m "feat: Add my feature"

# Push and create PR
git push origin feature/my-feature
```

## License

Licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)

at your option.

## Acknowledgments

- **SABnzbd** and **NZBGet** for inspiration and reference implementations
- The Rust community for excellent libraries and tooling
- Contributors to nntp-rs for NNTP protocol implementation

## Support

- **Issues**: https://github.com/jvz-devx/usenet-dl/issues
- **Discussions**: https://github.com/jvz-devx/usenet-dl/discussions
- **Documentation**: Run `cargo doc --open` or visit https://docs.rs/usenet-dl

---

Built with Rust
