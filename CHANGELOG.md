# Changelog

All notable changes to the usenet-dl project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Phase 0: Project Structure
- Initialized Cargo workspace with core library structure
- Created initial `Config` type with sensible defaults
- Set up `DownloadId` type for download identification
- Implemented comprehensive error types with thiserror

#### Phase 1: Core Library (137 tests)
- **Core Types** (Tasks 1.1-1.4)
  - `Config` with builder pattern and validation
  - `DownloadId` type for download identification
  - `DownloadInfo` and `HistoryEntry` types
  - `Status`, `Priority`, and `Stage` enums

- **SQLite Persistence** (Tasks 2.1-2.8, 33 tests)
  - Complete database schema with migrations
  - Article-level download tracking for resume support
  - Password caching for successful extractions
  - Download history tracking
  - Processed NZB tracking for watch folders
  - RSS feed state persistence
  - Duplicate detection support (NZB hash, job name)

- **Event System** (Tasks 3.1-3.5)
  - `tokio::broadcast` channel-based event delivery
  - 20+ event types covering all lifecycle stages
  - Multiple subscriber support
  - Event cloning and debugging

- **Download Manager** (Tasks 4.1-4.8)
  - Integration with nntp-rs for NNTP operations
  - Article download with progress tracking
  - Speed calculation and ETA estimation
  - Concurrent download management
  - Article-level status tracking

- **Priority Queue** (Tasks 5.1-5.9, 79 tests)
  - Priority-based queue ordering (Force > High > Normal > Low)
  - Queue persistence to SQLite
  - Download pause/resume support
  - Queue-wide pause/resume operations
  - Priority changes for queued downloads
  - Download cancellation with cleanup

- **Resume Support** (Tasks 6.1-6.6, 92 tests)
  - Article-level progress tracking
  - Automatic resume on startup
  - Crash recovery detection
  - Partial download verification
  - State restoration from database

- **Speed Limiting** (Tasks 7.1-7.7, 111 tests)
  - Token bucket algorithm implementation
  - Global speed limit shared across downloads
  - Runtime speed limit changes
  - Burst capacity support
  - Bandwidth distribution across concurrent downloads

- **Retry Logic** (Tasks 8.1-8.6, 121 tests)
  - Configurable exponential backoff
  - Retryable error detection
  - Jitter to prevent thundering herd
  - Maximum retry attempts
  - Initial delay and max delay configuration

- **Graceful Shutdown** (Tasks 9.1-9.8, 137 tests)
  - SIGTERM and SIGINT signal handling
  - Clean state preservation on shutdown
  - In-progress download pause
  - Database connection cleanup
  - Configurable shutdown timeout
  - Unclean shutdown detection and recovery

#### Phase 2: Post-Processing (240 tests)
- **Pipeline Skeleton** (Tasks 10.1-10.6, 141 tests)
  - Five-stage post-processing pipeline (Verify → Repair → Extract → Move → Cleanup)
  - `PostProcess` enum (None, Verify, Repair, Unpack, UnpackAndCleanup)
  - Stage-specific event emission
  - Failed download action handling (Keep, Delete, MoveToFailed)
  - Post-processing state persistence

- **RAR Extraction** (Tasks 11.1-11.8, 152 tests)
  - Multi-part RAR archive support (.rar, .r00, .r01, etc.)
  - Password-protected RAR extraction
  - Multi-source password collection (cached, per-download, NZB meta, global file, empty)
  - Password caching for successful extractions
  - RAR error handling and validation

- **Archive Support** (Tasks 12.1-12.6, 171 tests)
  - 7-Zip archive extraction with passwords
  - ZIP archive extraction with passwords
  - TAR/GZ/BZ2 extraction support
  - Password list management with deduplication
  - Comprehensive password testing across formats

- **Nested Extraction** (Tasks 13.1-13.5, 192 tests)
  - Recursive archive extraction
  - Configurable recursion depth (default: 2 levels)
  - Archive-in-archive detection
  - Extracted file tracking across recursion levels
  - Archive cleanup after successful extraction

- **Deobfuscation** (Tasks 14.1-14.6, 213 tests)
  - Obfuscated filename detection (entropy, UUID, hex patterns)
  - Multi-source name resolution (job name, NZB meta, archive comment, largest file)
  - Configurable minimum length threshold
  - SABnzbd-compatible deobfuscation heuristics
  - Deobfuscation enable/disable toggle

- **File Organization** (Tasks 15.1-15.6, 226 tests)
  - File moving to destination directories
  - Collision handling (Rename, Overwrite, Skip)
  - Unique path generation with numbered suffixes
  - Category-based destination routing
  - Move progress tracking and events

- **Cleanup** (Tasks 16.1-16.6, 240 tests)
  - Intermediate file removal (.par2, .nzb, .sfv, .srr)
  - Archive file cleanup (.rar, .r00-r99, .7z, .zip)
  - Sample folder detection and removal
  - Configurable delete_samples option
  - Cleanup error handling (non-fatal)

#### Phase 3: REST API (297 tests)
- **API Server** (Tasks 17.1-17.8)
  - Axum-based HTTP server
  - CORS support with configurable origins
  - Optional API key authentication
  - Health check endpoint (`GET /health`)
  - Request logging and tracing
  - Graceful shutdown support
  - Localhost-only binding by default (127.0.0.1:6789)

- **OpenAPI Integration** (Tasks 18.1-18.7)
  - utoipa-based OpenAPI 3.1 spec generation
  - 37 endpoints documented with full schemas
  - 34 types with utoipa annotations
  - Swagger UI at `/swagger-ui`
  - OpenAPI spec at `/openapi.json`
  - Interactive API documentation
  - "Try it out" functionality for all endpoints

- **Queue Endpoints** (Tasks 19.1-19.16)
  - `GET /downloads` - List all downloads
  - `GET /downloads/:id` - Get single download
  - `POST /downloads` - Upload NZB file (multipart/form-data)
  - `POST /downloads/url` - Add NZB from URL
  - `POST /downloads/:id/pause` - Pause download
  - `POST /downloads/:id/resume` - Resume download
  - `DELETE /downloads/:id` - Cancel/remove download
  - `PATCH /downloads/:id/priority` - Change priority
  - `POST /downloads/:id/reprocess` - Re-run post-processing
  - `POST /downloads/:id/reextract` - Re-run extraction only
  - `POST /queue/pause` - Pause all downloads
  - `POST /queue/resume` - Resume all downloads
  - `GET /queue/stats` - Get queue statistics
  - `GET /history` - Get download history
  - `DELETE /history` - Clear history
  - Manual testing tools (test_api.sh, Postman collection)

- **Server-Sent Events** (Tasks 20.1-20.5)
  - Real-time event streaming at `GET /events`
  - Event broadcast to all SSE clients
  - Automatic reconnection support
  - Event filtering and subscription
  - Client connection tracking

- **Config Endpoints** (Tasks 21.1-21.8)
  - `GET /config` - Get current config (sensitive fields redacted)
  - `PATCH /config` - Update config at runtime
  - `GET /config/speed-limit` - Get current speed limit
  - `PUT /config/speed-limit` - Set speed limit
  - `GET /categories` - List all categories
  - `PUT /categories/:name` - Create/update category
  - `DELETE /categories/:name` - Delete category
  - Runtime category management

- **Swagger UI** (Tasks 22.1-22.4)
  - Complete API documentation (26 paths, 34 schemas, 9 tags)
  - Interactive endpoint testing
  - Request/response schema validation
  - Example values for all types
  - API documentation completeness tests

- **Rate Limiting** (Tasks 23.1-23.6)
  - Token bucket rate limiting (disabled by default)
  - Configurable requests per second and burst size
  - Exempt paths (health check, SSE endpoint)
  - Exempt IPs (localhost by default)
  - HTTP 429 responses with retry-after headers

#### Phase 4: Automation (50 tests)
- **Folder Watching** (Tasks 24.1-24.10, 8 tests)
  - Automatic NZB import from watched directories
  - Configurable watch folder actions (Delete, MoveToProcessed, Keep)
  - Category assignment for watched NZBs
  - Scan interval configuration
  - File creation event handling
  - Duplicate import prevention

- **URL Fetching** (Tasks 25.1-25.5, 7 tests)
  - HTTP/HTTPS NZB downloading
  - Filename extraction from URLs and headers
  - Request timeout handling
  - Redirect following
  - Error handling for network failures

- **RSS Feed Support** (Tasks 26.1-26.12, 38 tests)
  - RSS/Atom feed monitoring
  - Automatic NZB discovery and download
  - Regex-based filtering (include/exclude patterns)
  - Size-based filtering (min/max)
  - Age-based filtering (max age from publish date)
  - Feed check interval configuration
  - Duplicate item detection (GUID tracking)
  - Category and priority assignment
  - Auto-download toggle
  - Feed error tracking
  - Manual feed refresh endpoint
  - Integration tests with real feeds

- **Scheduler** (Tasks 27.1-27.9, 50 tests)
  - Time-based schedule rules
  - Day-of-week filtering
  - Speed limit scheduling
  - Pause/resume scheduling
  - Rule priority ordering
  - Comprehensive time-based tests
  - Current action evaluation
  - Scheduler API endpoints

- **Duplicate Detection** (Tasks 28.1-28.8, 13 tests)
  - NZB content hash-based detection
  - NZB filename-based detection
  - Job name-based detection
  - Configurable duplicate actions (Block, Warn, Allow)
  - Multiple detection method support
  - API integration tests
  - Duplicate info tracking

#### Phase 5: Notifications & Polish (14 tests)
- **Webhook Notifications** (Tasks 29.1-29.7, 3 tests)
  - HTTP POST webhook support
  - Event-based triggers (OnComplete, OnFailed, OnQueued)
  - JSON payload with download metadata
  - Optional authentication headers
  - Timeout configuration
  - Async execution (non-blocking)
  - Integration tests with httpbin.org

- **Script Execution** (Tasks 30.1-30.9, 2 tests)
  - External script/executable execution
  - Event-based triggers (OnComplete, OnFailed, OnPostProcessComplete)
  - SABnzbd-compatible environment variables
  - Script timeout configuration
  - Exit code monitoring
  - Category-specific scripts
  - Async execution with timeout handling

- **Disk Space Checking** (Tasks 31.1-31.5, 7 tests)
  - Pre-download disk space validation
  - Size multiplier for extraction overhead (default: 2.5x)
  - Minimum free space buffer (default: 1 GB)
  - Platform-specific space checking (Unix/Windows)
  - Insufficient space error handling
  - Enable/disable toggle

- **Server Health Check** (Tasks 32.1-32.6, 5 tests)
  - TCP connectivity testing
  - TLS handshake validation
  - NNTP authentication testing
  - Server capability detection
  - Latency measurement
  - Bulk server testing endpoint
  - Integration tests with real NNTP servers

- **Re-Processing API** (Tasks 33.1-33.5, 2 tests)
  - Full post-processing re-run (`reprocess()`)
  - Extract-only re-run (`reextract()`)
  - File existence verification before reprocessing
  - Status reset to Processing
  - Event emission for reprocessing stages

- **Error Handling** (Tasks 34.1-34.6, 11 tests)
  - Comprehensive error types (DownloadError, PostProcessError, ApiError, etc.)
  - Error trait implementations
  - Context-rich error messages (stage, file, operation)
  - JSON error response format (code, message, details)
  - HTTP status code mapping
  - API error response tests

- **Documentation** (Tasks 35.1-35.4)
  - Comprehensive README.md (869 lines)
  - Four runnable examples (basic usage, event streaming, API server, advanced config)
  - API usage guide with 70+ curl examples
  - Configuration documentation (1187 lines, TOML and JSON formats)

#### Phase 6: Performance Optimizations
- **Parallel Article Downloads** (Tasks 1.1-7.2)
  - Converted sequential article downloads to parallel using `futures::stream::buffer_unordered()`
  - Automatic concurrency calculation from configured server connection counts
  - Lock-free atomic counters for progress tracking across parallel downloads
  - Dedicated progress reporting task to prevent event spam
  - Resilient error handling with partial success support (>50% threshold)
  - Cancellation support in parallel download contexts
  - Memory-efficient implementation (articles written to disk, not buffered in RAM)
  - Expected performance: ~N× speedup with N connections (4 connections: 4×, 20 connections: 20×, 50 connections: 40-50×)
  - Comprehensive test suite with stress testing up to 1200 concurrent segments
  - Queue processor and direct download methods both parallelized

### Testing
- **Unit Tests**: 297 tests across all modules
- **Integration Tests**: API, RSS, webhooks, health checks
- **Manual Testing**: Scripts, Postman collection, testing guides
- **Coverage**: Core library, API endpoints, post-processing, automation

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
- futures 0.3 - Async stream utilities for parallel downloads

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
- nntp-rs integration not yet implemented (mock implementation used for testing)
- Archive extraction requires external tools (unrar, 7z)
- Password-protected archives use trial-and-error approach
- Sample folder detection is heuristic-based

## [0.1.0] - Initial Development

Initial project structure and planning phase.

### Added
- Project design document (implementation_1.md)
- Core architecture planning
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
