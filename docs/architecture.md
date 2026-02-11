# Architecture

This document describes the internal architecture, design patterns, and component organization of usenet-dl.

## Overview

usenet-dl is a Rust backend library for building Usenet download applications. It provides a library-first design with no CLI or UI, intended for embedding into larger applications or building custom frontends.

The library is built on top of `nntp-rs` (a sibling crate) which handles the NNTP protocol implementation, NZB parsing, yEnc decoding, and PAR2 operations.

## Design Principles

- **Library-First**: Pure Rust crate designed for embedding, not a standalone application
- **Event-Driven**: Consumers subscribe to events rather than polling for status
- **Async-Native**: Built on tokio for efficient concurrent operations
- **Highly Configurable**: Almost every behavior can be customized
- **Sensible Defaults**: Works out of the box with minimal configuration
- **Thread-Safe**: Arc-wrapped state enables safe sharing across async tasks

## Module Structure

### Core Modules

**downloader/** - Main orchestration
Contains the `UsenetDownloader` struct and all download coordination: queue management (`queue.rs`, `queue_processor.rs`), NZB import (`nzb.rs`), download control (`control.rs`), background tasks (`background_tasks.rs`), webhook dispatch (`webhooks.rs`), and DirectUnpack (`direct_unpack/`).

**config.rs** - Configuration types
Defines all configuration structures with sensible defaults for NNTP servers, post-processing, categories, rate limiting, and API settings. Uses `#[serde(flatten)]` for sub-configs so TOML/JSON fields appear at top level.

**types.rs** - Core data types
Defines `Event`, `Status`, `Priority`, `DownloadInfo`, `Stage`, and other fundamental types used throughout the library.

**db/** - Database persistence
SQLite persistence layer using sqlx. Split into sub-modules: `downloads.rs`, `articles.rs`, `history.rs`, `rss.rs`, `migrations.rs`.

**error.rs** - Error handling
Comprehensive error types with context information and HTTP status mapping for API responses.

### Post-Processing

**post_processing/** - Processing pipeline
Implements the five-stage pipeline: Verify (`verify.rs`) → Repair (`repair.rs`) → Extract → Move → Cleanup (`cleanup.rs`). Orchestrated by `mod.rs`.

**extraction/** - Archive extraction
RAR (`rar.rs`), 7z (`sevenz.rs`), and ZIP (`zip.rs`) extraction with multi-source password support (`password_list.rs`).

**parity/** - PAR2 verification and repair
`ParityHandler` trait (`traits.rs`) with pluggable implementations: `CliParityHandler` (`cli.rs`) for systems with `par2` binary, `NoOpParityHandler` (`noop.rs`) as fallback. Includes PAR2 output parser (`parser.rs`) and PAR2 binary metadata parser (`par2_metadata.rs`) for DirectRename.

**deobfuscation.rs** - Filename cleanup
Cleans up obfuscated filenames common in Usenet releases.

### API Layer

**api/routes/** - REST endpoints
37 HTTP endpoints split by resource: `downloads.rs`, `queue.rs`, `history.rs`, `config.rs`, `categories.rs`, `rss.rs`, `scheduler.rs`, `servers.rs`, `system.rs`.

**api/openapi.rs** - OpenAPI schema
Generates OpenAPI 3.1 specification and serves Swagger UI at `/swagger-ui`.

**api/auth.rs** - Authentication
Optional API key-based authentication middleware.

**api/rate_limit.rs** - Rate limiting
Per-IP rate limiting middleware using token bucket algorithm.

### Automation

**folder_watcher.rs** - Automatic NZB import
Watches directories for new NZB files and automatically queues them with configurable post-import actions.

**rss_manager/** - RSS feed monitoring
Periodic checking of RSS/Atom feeds with regex-based filtering and duplicate prevention.

**rss_scheduler.rs** - RSS scheduling
Coordinates periodic RSS feed checks.

**scheduler/** - Time-based scheduling
Implements schedule rules for speed limiting, queue pausing, and other time-based actions.

### Utilities

**speed_limiter.rs** - Bandwidth control
Token bucket algorithm for global download speed limiting.

**retry.rs** - Retry logic
Exponential backoff with jitter for failed operations.

**utils.rs** - Utility functions
Disk space checking and other shared utility functions.

## Key Design Patterns

### Event-Driven Architecture

The library uses `tokio::broadcast` channels to emit events. Consumers subscribe via `downloader.subscribe()` and receive a stream of events:

```rust
let mut receiver = downloader.subscribe();
while let Ok(event) = receiver.recv().await {
    match event {
        Event::Downloading { id, percent, speed_bps, .. } => { /* handle progress */ }
        Event::Complete { id, .. } => { /* handle completion */ }
        _ => {}
    }
}
```

Events are buffered (1000-event capacity) and broadcast to all subscribers independently.

### Arc-Wrapped State

The `UsenetDownloader` struct implements `Clone` by wrapping all internal state in `Arc`. This enables safe sharing across async tasks:

```rust
pub struct UsenetDownloader {
    db: Arc<Database>,
    config: Arc<Config>,
    event_tx: broadcast::Sender<Event>,
    queue: Arc<Mutex<BinaryHeap<QueuedDownload>>>,
    // ... other Arc-wrapped fields
}
```

### Priority Queue

Downloads are managed in a `BinaryHeap<QueuedDownload>` that orders by:
1. Priority (High > Normal > Low)
2. Queue timestamp (FIFO within same priority)

The queue is protected by a tokio `Mutex` for atomic operations.

### Semaphore-Limited Concurrency

A semaphore controls the number of concurrent downloads:

```rust
let permit = self.concurrent_limit.acquire().await?;
// Download happens here with permit held
drop(permit); // Release permit when done
```

This ensures the `max_concurrent_downloads` setting is respected.

### Cancellation Tokens

Each active download has an associated `CancellationToken` stored in a map. This enables graceful pause/cancel operations:

```rust
// Store token when download starts
active_downloads.insert(download_id, token.clone());

// Cancel from another task
if let Some(token) = active_downloads.get(&download_id) {
    token.cancel();
}
```

## Data Flow

### Download Pipeline

1. **Queue**: NZB added to priority queue with metadata
2. **Dispatch**: Semaphore permit acquired, download task spawned
3. **Download**: Articles fetched concurrently from NNTP servers
3b. **DirectUnpack** (optional): Background coordinator extracts completed RAR volumes during download
4. **Assembly**: Articles decoded (yEnc) and written to disk
5. **Post-Processing**: Optional verify → repair → extract → move → cleanup (stages 1-3 skipped if DirectUnpack succeeded)
6. **Completion**: Final event emitted, download moved to history

### Post-Processing Pipeline

The post-processor implements a configurable five-stage pipeline:

1. **Verify**: Check PAR2 files to verify data integrity
2. **Repair**: Use PAR2 files to repair corrupted data if needed
3. **Extract**: Unpack RAR/7z/ZIP archives with password support
4. **Move**: Move extracted files to final destination directory
5. **Cleanup**: Remove temporary files and archives

The pipeline can be configured to run none, some, or all stages via the `PostProcessingMode` setting.

## Database Schema

SQLite database with the following tables:

**downloads** - Queue state
Primary queue table with download metadata, status, progress, timestamps, configuration, and DirectUnpack state.

**download_articles** - Article tracking
Per-article status for resume capability. Tracks individual article download state.

**passwords** - Password cache
Successful archive passwords per download for retry efficiency.

**processed_nzbs** - Watch folder tracking
Tracks processed NZB files to prevent re-queueing when using "Keep" action.

**history** - Completed downloads
Historical record of completed downloads with final status and statistics.

**rss_feeds** - RSS configurations
Feed URLs, check intervals, categories, and settings.

**rss_filters** - RSS filter rules
Per-feed regex patterns for filtering items.

**rss_seen** - Duplicate prevention
Tracks seen RSS items by GUID to prevent duplicate downloads.

**schedule_rules** - Scheduler configuration
Time-based rules with actions (speed limit, pause, resume) and day specifications.

Migrations run automatically on startup to ensure schema is current.

## REST API Architecture

The API is built with Axum and provides:

- **37 HTTP endpoints** organized by function
- **OpenAPI 3.1** compliant schema
- **Swagger UI** at `/swagger-ui` for interactive documentation
- **Server-Sent Events** at `/events` for real-time event streaming
- **Optional authentication** via API key
- **CORS support** for web frontends
- **Rate limiting** (optional, per-IP)

Endpoints are grouped into categories:
- Queue operations (add, remove, pause, resume, priority)
- Global queue management (pause all, resume all, statistics)
- History management (retrieve, clear)
- Configuration updates (runtime config changes)
- Category management (add, update, delete)
- RSS feed management (CRUD operations)
- Scheduler management (CRUD for schedule rules)
- Server testing and health checks
- System operations (shutdown, health)

The `/events` endpoint streams Server-Sent Events, providing real-time updates without polling.

## Concurrency Model

The library uses several concurrency primitives:

- **Arc**: Shared ownership of immutable state
- **Mutex**: Protected access to mutable state (queue, active downloads)
- **Semaphore**: Limit concurrent downloads
- **Broadcast channels**: Multi-subscriber event delivery
- **Cancellation tokens**: Graceful task cancellation
- **Tokio tasks**: Concurrent download and background operations

All state is thread-safe and can be safely accessed from multiple async tasks.

## Error Handling

Errors are structured with context information:

- **Configuration errors**: Invalid settings with specific field information
- **Database errors**: SQLx integration with migration failures
- **NNTP errors**: Protocol-level failures from nntp-rs
- **Download errors**: Network failures, missing articles, disk errors
- **Post-processing errors**: PAR2, extraction, and file operation failures
- **API errors**: HTTP status code mapping for REST responses

Error types include machine-readable codes and rich context (stage, file path, download ID) for debugging.

## Automation Components

### Folder Watching

The folder watcher monitors directories for new NZB files and automatically queues them:

- Non-recursive watching (doesn't scan subdirectories)
- Configurable post-import actions: Delete, Move to processed directory, or Keep
- Per-folder category assignment
- Duplicate prevention

### RSS Feed Management

RSS manager periodically checks feeds and downloads matching items:

- Supports RSS and Atom feeds
- Regex-based filtering (accept/reject patterns)
- Duplicate prevention via tracking seen item GUIDs
- Automatic NZB download and queueing
- Per-feed check intervals and category assignment

### Time-Based Scheduling

Scheduler executes rules based on time windows:

- Schedule rules with start/end times
- Actions: Set speed limit, unlimited speed, pause queue, resume queue
- Per-day configuration (all days or specific weekdays)
- Overlapping rules handled with priority (most recent action wins)

## Dependency on nntp-rs

usenet-dl delegates NNTP protocol operations to `nntp-rs`:

**nntp-rs responsibilities**:
- NNTP protocol implementation (RFC 3977)
- NZB parsing
- yEnc decoding
- PAR2 verification and repair
- Connection pooling

**usenet-dl responsibilities**:
- Queue management and persistence
- Download orchestration
- Post-processing pipeline
- Archive extraction
- REST API
- Event broadcasting
- Automation (folder watching, RSS, scheduling)
- File organization and collision handling
- Disk space monitoring
- Notifications (webhooks, scripts)

## External Tool Dependencies

Archive extraction requires command-line tools in PATH:

- `unrar` for RAR extraction
- `7z` for 7-Zip extraction

These tools are invoked as subprocesses. If not available, extraction will fail but other functionality remains operational.

## Configuration Philosophy

The library follows a "sensible defaults" philosophy:

```rust
Config {
    servers: vec![],
    download: DownloadConfig {
        download_dir: "./downloads".into(),
        temp_dir: "./temp".into(),
        max_concurrent_downloads: 3,
        speed_limit_bps: None, // Unlimited
        default_post_process: PostProcess::UnpackAndCleanup,
        ..Default::default()
    },
    ..Default::default()
}
```

Almost every default can be overridden. Configuration is immutable after `UsenetDownloader::new()`, except for specific runtime-mutable fields (speed limit, categories, schedule rules) that can be updated via API calls.

## State Persistence

All critical state is persisted to SQLite:

- Download queue with metadata and progress
- Article-level download status for resume capability
- History of completed downloads
- RSS feed configurations and seen items
- Schedule rules

The library survives crashes and restarts by loading queue state on startup. Downloads can be resumed at the article level.

## Event Types

The event system emits typed events for all significant state changes:

**Queue events**: `Queued`, `Removed`
**Download lifecycle**: `Downloading`, `DownloadComplete`, `DownloadFailed`
**DirectUnpack**: `DirectUnpackStarted`, `FileCompleted`, `DirectUnpackExtracting`, `DirectUnpackExtracted`, `DirectUnpackCancelled`, `DirectUnpackComplete`, `DirectRenamed`
**Post-processing**: `Verifying`, `VerifyComplete`, `Repairing`, `RepairComplete`, `RepairSkipped`, `Extracting`, `ExtractComplete`, `Moving`, `Cleaning`
**Final states**: `Complete`, `Failed`
**Global events**: `SpeedLimitChanged`, `QueuePaused`, `QueueResumed`, `Shutdown`
**Notifications**: `WebhookFailed`, `ScriptFailed`
**Detection**: `DuplicateDetected`

Events include rich metadata (download ID, progress, stage, file paths, error messages) for building responsive UIs.
