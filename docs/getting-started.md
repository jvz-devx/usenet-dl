# Getting Started

This guide walks you through installing usenet-dl and creating your first Usenet downloader application.

## Prerequisites

### System Requirements

- **Rust**: Version 1.93 or later
- **SQLite**: Embedded via sqlx (no manual installation needed)
- **Archive extraction tools** (optional):
  - `unrar` for RAR extraction
  - `7z` for 7-Zip extraction
  - ZIP support is built-in

### Development Environment Setup

#### Using Nix (Recommended)

```bash
nix-shell
cargo build
cargo test
```

#### Manual Setup

**Ubuntu/Debian:**
```bash
sudo apt-get install libssl-dev pkg-config sqlite3
cargo build
```

**macOS:**
```bash
brew install openssl sqlite
cargo build
```

**Fedora/RHEL:**
```bash
sudo dnf install openssl-devel sqlite-devel
cargo build
```

## Installation

**Note**: usenet-dl is not yet published to crates.io.

Add as a dependency in your `Cargo.toml`:

```toml
[dependencies]
usenet-dl = { path = "../usenet-dl" }
# or
usenet-dl = { git = "https://github.com/jvz-devx/usenet-dl" }
```

## Basic Usage

### Minimal Example

Here's the simplest way to get started:

```rust
use usenet_dl::config::{Config, DownloadConfig, ServerConfig};
use usenet_dl::{UsenetDownloader, DownloadOptions, Event, Priority};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    let downloader = UsenetDownloader::new(config).await?;

    let id = downloader.add_nzb(
        "file.nzb".as_ref(),
        DownloadOptions {
            category: Some("movies".into()),
            priority: Priority::Normal,
            ..Default::default()
        }
    ).await?;

    println!("Queued download: {}", id);
    Ok(())
}
```

### Event Monitoring

Subscribe to download events for progress tracking:

```rust
use usenet_dl::{UsenetDownloader, Config, Event};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    // Add downloads...

    Ok(())
}
```

### Download Control

Control downloads with pause, resume, and speed limiting:

```rust
// Pause a download
downloader.pause(id).await?;

// Resume a download
downloader.resume(id).await?;

// Set global speed limit (10 MB/s)
downloader.set_speed_limit(Some(10_000_000)).await;

// Remove speed limit
downloader.set_speed_limit(None).await;
```

### Multiple Event Subscribers

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

## Starting the REST API

Enable the REST API for remote control:

```rust
use std::sync::Arc;
use usenet_dl::api::start_api_server;

let downloader = Arc::new(UsenetDownloader::new(config.clone()).await?);
let config = Arc::new(config);
start_api_server(downloader, config).await?;
```

The API will be available at:
- Base URL: `http://localhost:6789/api/v1`
- Swagger UI: `http://localhost:6789/swagger-ui`

### Example API Calls

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

## Common Configuration

### Multiple NNTP Servers

Configure multiple servers with priority-based failover:

```rust
let config = Config {
    servers: vec![
        ServerConfig {
            host: "primary.news.com".to_string(),
            port: 563,
            tls: true,
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            connections: 10,
            priority: 0,  // Tried first
            pipeline_depth: 10,
        },
        ServerConfig {
            host: "backup.news.com".to_string(),
            port: 563,
            tls: true,
            username: Some("user2".to_string()),
            password: Some("pass2".to_string()),
            connections: 5,
            priority: 1,  // Tried if primary fails
            pipeline_depth: 10,
        }
    ],
    ..Default::default()
};
```

### Categories

Organize downloads by category with custom destinations:

```rust
use usenet_dl::config::{Config, PersistenceConfig, CategoryConfig, PostProcess};

let config = Config {
    persistence: PersistenceConfig {
        categories: [
            ("movies".to_string(), CategoryConfig {
                destination: "/mnt/media/movies".into(),
                post_process: Some(PostProcess::UnpackAndCleanup),
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
    ..Default::default()
};
```

### Post-Processing Options

Control what happens after download completion:

```rust
use usenet_dl::config::{Config, DownloadConfig, PostProcess};

let config = Config {
    download: DownloadConfig {
        default_post_process: PostProcess::UnpackAndCleanup,
        delete_samples: true,
        ..Default::default()
    },
    ..Default::default()
};
```

## Examples

The `examples/` directory contains complete working examples:

- **basic_download.rs** - Simple download with event monitoring
- **custom_configuration.rs** - Advanced configuration options
- **multi_subscriber.rs** - Multiple event subscribers
- **rest_api_server.rs** - REST API server with all features
- **speedtest.rs** - Download speed benchmarking

Run an example:

```bash
cargo run --example basic_download
cargo run --example rest_api_server
```

## Testing Your Setup

Verify your installation and configuration:

```bash
# Build the library
cargo build

# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Build documentation
cargo doc --open
```

## Next Steps

- **Configuration**: See [configuration.md](configuration.md) for all available options
- **REST API**: See [api-reference.md](api-reference.md) for complete API documentation
- **Architecture**: See [architecture.md](architecture.md) to understand the system design
- **Post-Processing**: See [post-processing.md](post-processing.md) for extraction and cleanup details
- **Contributing**: See [contributing.md](contributing.md) for development guidelines

## Troubleshooting

### "Connection refused" errors

Verify your NNTP server credentials and network connectivity:
```bash
# Test with telnet (non-TLS)
telnet news.example.com 119

# Test with openssl (TLS)
openssl s_client -connect news.example.com:563
```

### Archive extraction fails

Ensure extraction tools are installed and in PATH:
```bash
# Check for unrar
which unrar

# Check for 7z
which 7z

# Install on Ubuntu/Debian
sudo apt-get install unrar p7zip-full

# Install on macOS
brew install unrar p7zip
```

### Database locked errors

Only one `UsenetDownloader` instance can access the database at a time. Ensure you're not running multiple instances pointing to the same `database_path`.

### Out of disk space

Enable disk space checking to prevent extraction failures:
```rust
use usenet_dl::config::{Config, ProcessingConfig, DiskSpaceConfig};

let config = Config {
    processing: ProcessingConfig {
        disk_space: DiskSpaceConfig {
            enabled: true,
            min_free_space: 1024 * 1024 * 1024, // 1 GB
            size_multiplier: 2.5,
        },
        ..Default::default()
    },
    ..Default::default()
};
```
