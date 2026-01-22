# usenet-dl

A backend library for building SABnzbd/NZBGet-like applications in Rust.

## Status

**⚠️ Work in Progress - Phase 0 Complete**

This library is currently under active development. Basic project structure is complete. Core functionality implementation is in progress.

## Design Philosophy

usenet-dl is designed to be:

- **Highly configurable** - Almost every behavior can be customized
- **Sensible defaults** - Works out of the box with zero configuration
- **Library-first** - No CLI or UI, purely a Rust crate for embedding
- **Event-driven** - Consumers subscribe to events, no polling required

## Architecture

```
┌─────────────────────────────────────────┐
│  Spotnet App    │  SABnzbd Alternative  │
├─────────────────┴───────────────────────┤
│              usenet-dl                  │
├─────────────────────────────────────────┤
│              nntp-rs                    │
└─────────────────────────────────────────┘
```

### Responsibility Split

**usenet-dl handles:**
- Download queue management
- Post-processing pipeline (PAR2, extraction, renaming)
- RAR/7z/ZIP extraction with password management
- File organization and cleanup
- Progress and event callbacks
- Persistence (SQLite)
- NZB folder watching and URL fetching
- REST API with OpenAPI documentation
- External notifications (webhooks, scripts)
- RSS feed monitoring
- Scheduler for time-based rules

**nntp-rs handles:**
- NNTP protocol (RFC 3977)
- NZB parsing
- yEnc decoding
- PAR2 verification (repair not yet implemented)
- Connection pooling

## Features (Roadmap)

### Phase 0: Project Initialization ✅
- [x] Project structure created
- [x] Core type definitions (Event, Status, Priority, Config)
- [x] Error handling foundation
- [x] Basic module structure

### Phase 1: Core Library 🚧 Next
- [ ] Download queue with priority support
- [ ] Resume downloads after restart
- [ ] Speed limiting (global, shared across downloads)
- [ ] Retry logic with exponential backoff
- [ ] SQLite persistence
- [ ] Event system via tokio::broadcast
- [ ] Graceful shutdown

### Phase 2: Post-Processing
- [ ] RAR/7z/ZIP extraction
- [ ] Password management (per-download, global file, NZB metadata)
- [ ] Nested archive extraction
- [ ] Obfuscated filename detection and renaming
- [ ] File collision handling (rename, overwrite, skip)
- [ ] Cleanup (remove .par2, .nzb, samples, archives)

### Phase 3: REST API
- [ ] OpenAPI 3.1 compliant endpoints
- [ ] Server-Sent Events for real-time updates
- [ ] Swagger UI for interactive documentation
- [ ] Optional rate limiting
- [ ] CORS support for frontend development

### Phase 4: Automation
- [ ] Folder watching (auto-import NZBs)
- [ ] URL fetching (download NZB from HTTP)
- [ ] RSS feed monitoring with filters
- [ ] Scheduler (time-based speed limits and pausing)
- [ ] Duplicate detection (hash and name based)

### Phase 5: Notifications & Polish
- [ ] Webhooks (HTTP POST on events)
- [ ] Script execution (run commands on completion)
- [ ] Category-specific scripts
- [ ] Disk space checking
- [ ] Server health testing
- [ ] Comprehensive error handling
- [ ] Full documentation

## Installation

**Not yet published to crates.io**

For now, you can use it as a path dependency:

```toml
[dependencies]
usenet-dl = { path = "../usenet-dl" }
```

## Planned Usage

```rust
use usenet_dl::{UsenetDownloader, Config, ServerConfig};

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
            }
        ],
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await?;

    // Subscribe to events
    let mut events = downloader.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            println!("Event: {:?}", event);
        }
    });

    // API coming in Phase 1
    // downloader.add_nzb(path, options).await?;
    // downloader.pause(id).await?;
    // downloader.set_speed_limit(Some(10_000_000)).await?;

    Ok(())
}
```

## Development

### Requirements

- Rust 1.70 or later
- OpenSSL development headers
- Optional: Nix (for reproducible environment)

### Setup with Nix

```bash
nix-shell
cargo build
cargo test
```

### Setup without Nix

```bash
# Install OpenSSL (Ubuntu/Debian)
sudo apt-get install libssl-dev pkg-config

# Or on Fedora/RHEL
sudo dnf install openssl-devel pkg-config

cargo build
cargo test
```

### Useful Commands

```bash
cargo check          # Fast syntax/type checking
cargo build          # Build the library
cargo test           # Run tests
cargo clippy         # Lint checks
cargo fmt            # Format code
cargo doc --open     # Build and view documentation
```

## Configuration Defaults

All settings have sensible defaults. Only NNTP server credentials are required.

| Setting | Default | Rationale |
|---------|---------|-----------|
| Download directory | `./downloads` | Easy to find |
| Temp directory | `./temp` | Separate from final downloads |
| Concurrent downloads | 3 | Balanced throughput |
| Speed limit | Unlimited | Full speed by default |
| Post-processing | Unpack + Cleanup | Ready-to-use files |
| Failed download action | Keep files | Don't lose data |
| File collision | Rename (add number) | Never overwrite |
| Nested extraction depth | 2 levels | Handle archive-in-archive |
| Try empty password | Yes | Common for public releases |
| Delete samples | Yes | Usually unwanted |
| API bind address | 127.0.0.1:6789 | Localhost only for security |

See [implementation_1.md](implementation_1.md) for the complete design specification.

## Dependencies

- **nntp-rs** - NNTP client and NZB parsing (local path dependency)
- **tokio** - Async runtime
- **sqlx** - SQLite persistence
- **axum** - REST API framework
- **utoipa** - OpenAPI documentation
- **unrar, sevenz-rust, zip** - Archive extraction
- **reqwest** - HTTP client
- **notify** - File watching

See [Cargo.toml](Cargo.toml) for the complete dependency list (40+ crates).

## Known Issues

- **RSS/Atom dependencies temporarily disabled:** The `rss` and `atom_syndication` crates conflict with nntp-rs due to quick-xml feature flags. Will be resolved in Phase 4.

## Documentation

- [implementation_1.md](implementation_1.md) - Complete design specification (2600+ lines)
- [implementation_1_PROGRESS.md](implementation_1_PROGRESS.md) - Detailed task tracking and progress

## License

MIT OR Apache-2.0

## Contributing

This project is in early development. Contributions welcome once Phase 1 is complete.
