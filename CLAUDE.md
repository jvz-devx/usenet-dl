# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

usenet-dl is a Rust backend library for building SABnzbd/NZBGet-like Usenet download applications. It's library-first (no CLI/UI) and depends on `nntp-rs` (sibling directory) for NNTP protocol implementation.

## Common Commands

```bash
# Development (requires Nix or manual setup of openssl, sqlite)
nix-shell                           # Enter dev environment with all dependencies

# Building and checking
cargo build                         # Build the library
cargo check                         # Fast syntax/type checking
cargo clippy                        # Lint checks
cargo fmt                           # Format code

# Running tests
cargo test                          # Run all tests (300+ tests)
cargo test db::tests                # Run specific module tests
cargo test api::tests               # Run API tests
cargo test test_queue_priority      # Run single test by name
cargo test -- --nocapture           # Run tests with stdout output

# E2E tests with real NZB files (requires .env with NNTP credentials)
TEST_NZB_PATH="./file.nzb" cargo test --release --test e2e_real_nzb test_real_nzb_download -- --ignored --nocapture

# Documentation
cargo doc --open                    # Build and view rustdoc
```

## Architecture

```
usenet-dl (this crate)
    └── nntp-rs (path = "../nntp-rs")
```

### Module Structure

- **lib.rs** - `UsenetDownloader` main struct, queue management, download orchestration. Large file (~8K+ lines), contains most download logic
- **api/** - REST API with Axum (routes.rs for handlers, openapi.rs for Swagger, auth.rs, rate_limit.rs)
- **db.rs** - SQLite persistence via sqlx (downloads, articles, history, RSS, scheduler tables)
- **config.rs** - All configuration types with sensible defaults
- **types.rs** - Core types: `Event`, `Status`, `Priority`, `DownloadInfo`, `Stage`
- **extraction.rs** - RAR/7z/ZIP extraction with password support
- **post_processing.rs** - Pipeline: verify → repair → extract → move → cleanup
- **deobfuscation.rs** - Filename cleanup for obfuscated releases
- **speed_limiter.rs** - Token bucket algorithm for bandwidth control
- **retry.rs** - Exponential backoff with jitter
- **folder_watcher.rs** - Auto-import NZBs from watched directories
- **rss_manager.rs** / **rss_scheduler.rs** - RSS feed monitoring
- **scheduler.rs** / **scheduler_task.rs** - Time-based scheduling rules

### Key Design Patterns

- **Event-driven**: `tokio::broadcast` channel for events (subscribe via `downloader.subscribe()`)
- **Arc-wrapped**: `UsenetDownloader` is `Clone` - all internal state wrapped in `Arc`
- **Priority queue**: `BinaryHeap<QueuedDownload>` for download ordering
- **Semaphore-limited concurrency**: `concurrent_limit` controls parallel downloads
- **Cancellation tokens**: Each download has a `CancellationToken` for pause/cancel

### Database

SQLite via sqlx. Tables: `downloads`, `download_articles`, `passwords`, `processed_nzbs`, `history`, `rss_feeds`, `rss_filters`, `rss_seen`, `schedule_rules`. Migrations run automatically on startup.

## Testing

- Unit tests are in-file with `#[cfg(test)]` modules
- Integration tests in `tests/` directory
- E2E tests (`e2e_live.rs`, `e2e_real_nzb.rs`) require NNTP credentials in `.env`
- Feature flags: `live-tests` for real NNTP tests, `docker-tests` for containerized tests
- API tests use `wiremock` for HTTP mocking

## External Tool Dependencies

Archive extraction requires command-line tools in PATH:
- `unrar` for RAR extraction
- `7z` for 7-Zip extraction
