# Changelog

All notable changes to the usenet-dl project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Core Download Features
- Core types: `Config`, `DownloadId`, `DownloadInfo`, `HistoryEntry`, `Status`, `Priority`, and `Stage` enums
- SQLite persistence with automatic migrations for downloads, articles, history, RSS feeds, and scheduler
- Article-level download tracking for granular resume support
- Event system using `tokio::broadcast` channels with 20+ event types
- NNTP integration via nntp-rs library for protocol operations
- Article download with real-time progress tracking, speed calculation, and ETA estimation
- Priority-based queue ordering (Force > High > Normal > Low)
- Download pause/resume support with queue-wide controls
- Automatic resume on startup with crash recovery detection
- Token bucket algorithm for global speed limiting across downloads
- Runtime speed limit changes with burst capacity support
- Exponential backoff retry logic with jitter
- Graceful shutdown with SIGTERM/SIGINT signal handling
- Parallel article downloads with automatic concurrency scaling

#### Post-Processing Pipeline
- Five-stage post-processing pipeline: Verify → Repair → Extract → Move → Cleanup
- Multi-part RAR archive extraction with password support
- 7-Zip, ZIP, TAR/GZ/BZ2 archive extraction
- Recursive archive extraction with configurable depth
- Password caching for successful extractions
- Multi-source password collection (cached, per-download, NZB metadata, global file)
- Obfuscated filename detection and cleanup using entropy analysis
- Multi-source name resolution (job name, NZB metadata, archive comments, largest file)
- File collision handling (Rename, Overwrite, Skip)
- Category-based destination routing
- Intermediate file cleanup (.par2, .nzb, .sfv, .srr)
- Sample folder detection and removal

#### REST API
- Axum-based HTTP server with CORS support
- Optional API key authentication
- OpenAPI 3.1 specification with Swagger UI at `/swagger-ui`
- Real-time event streaming via Server-Sent Events at `/events`
- Queue management endpoints (list, add, pause, resume, cancel, priority changes)
- NZB upload via multipart/form-data and URL fetching
- Configuration management with runtime updates
- Category management (create, update, delete)
- Download history endpoints
- Post-processing re-run endpoints (reprocess, reextract)
- Rate limiting with configurable requests per second
- Health check endpoint

#### Automation Features
- Automatic NZB import from watched directories
- RSS/Atom feed monitoring with automatic download
- Regex-based feed filtering (include/exclude patterns)
- Size and age-based filtering for RSS items
- Time-based scheduler with day-of-week rules
- Speed limit scheduling
- Pause/resume scheduling
- Duplicate detection via NZB hash, filename, or job name
- Configurable duplicate actions (Block, Warn, Allow)

#### Notifications
- HTTP webhook support with event-based triggers
- External script execution with standard environment variables
- Category-specific notification handlers
- JSON payload delivery with authentication headers

#### Utilities
- Pre-download disk space validation with extraction overhead calculation
- NNTP server health checks (connectivity, TLS, authentication, capabilities)
- Latency measurement for server selection

### Dependencies
- tokio 1.0 - Async runtime
- sqlx 0.7 - Database operations
- axum 0.7 - HTTP server
- utoipa 4.0 - OpenAPI documentation
- serde 1.0 - Serialization
- tracing 0.1 - Logging
- thiserror 1.0 - Error handling
- reqwest 0.11 - HTTP client
- notify 6.0 - File system watching
- rss 2.0 - RSS feed parsing
- futures 0.3 - Async stream utilities

### Architecture
- Event-driven design with tokio::broadcast channels
- SQLite persistence for queue state and article tracking
- Article-level granularity for resume support
- Token bucket algorithm for global speed limiting
- Exponential backoff for retry logic
- Five-stage post-processing pipeline
- OpenAPI 3.1 compliant REST API
- Real-time updates via Server-Sent Events

### Known Limitations
- Archive extraction requires external tools (unrar, 7z)
- Password-protected archives use trial-and-error approach
- Sample folder detection is heuristic-based

## [0.1.0] - Initial Development

### Added
- Initial project structure and core architecture
- Database schema design
- REST API endpoint specification
- Configuration system design

---

## Release Notes Format

### Version Numbering
- **Major version** (1.x.x): Breaking API changes
- **Minor version** (x.1.x): New features, backwards compatible
- **Patch version** (x.x.1): Bug fixes, backwards compatible

### Release Categories
- **Added**: New features
- **Changed**: Changes in existing functionality
- **Deprecated**: Soon-to-be removed features
- **Removed**: Removed features
- **Fixed**: Bug fixes
- **Security**: Security fixes

---

*This changelog is automatically maintained and follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) conventions.*
