# Progress: implementation_1

Started: do 22 jan 2026 15:45:56 CET

## Status

IN_PROGRESS

**Phase 0 Complete** - All 5 initialization tasks finished. Project structure is in place and compiling successfully. Ready for Phase 1 (Core Library).

## Analysis

### Current Codebase State

**Project Status:** Design complete, implementation not yet started

The usenet-dl project exists only as comprehensive design documentation. There is NO Rust code, NO Cargo.toml, and NO src/ directory yet. The implementation plan is extremely well-documented with:
- Complete 2600+ line design specification (implementation_1.md)
- Full API design with OpenAPI 3.1 specification
- Complete SQLite database schema
- Detailed dependency list (40+ crates)
- 35-step implementation roadmap across 5 phases

### nntp-rs Dependency Analysis

**Status:** Production-ready, fully implemented

The nntp-rs library (located at ../nntp-rs) is a comprehensive, high-quality Rust library that provides:
- ✅ Complete NNTP client (RFC 3977) with 600+ tests
- ✅ NZB XML parsing with segment management
- ✅ yEnc encoding/decoding with multipart assembly
- ✅ PAR2 parsing and verification (repair NOT implemented)
- ✅ Connection pooling (bb8-based, configurable)
- ✅ TLS support (implicit TLS on port 563)
- ✅ Compression (RFC 8054 DEFLATE + XFEATURE GZIP)
- ✅ Rate limiting (token bucket)
- ✅ Multi-server failover
- ✅ Article format parsing (RFC 5536)

**Key Finding:** nntp-rs handles ALL low-level NNTP operations. usenet-dl only needs to orchestrate downloads, manage queues, handle post-processing, and provide a REST API.

### What Already Exists vs What's Missing

| Component | Status | Location |
|-----------|--------|----------|
| **Design Documentation** | ✅ Complete | implementation_1.md |
| **nntp-rs Library** | ✅ Production Ready | ../nntp-rs/ |
| **Development Environment** | ✅ Complete | shell.nix |
| **Rust Project Structure** | ❌ Missing | Need Cargo.toml + src/ |
| **Core Types** | ❌ Not Implemented | - |
| **SQLite Persistence** | ❌ Not Implemented | - |
| **Event System** | ❌ Not Implemented | - |
| **Download Manager** | ❌ Not Implemented | - |
| **Post-Processing** | ❌ Not Implemented | - |
| **REST API** | ❌ Not Implemented | - |
| **Automation (RSS, Watch)** | ❌ Not Implemented | - |
| **Notifications** | ❌ Not Implemented | - |
| **Tests** | ❌ None Yet | - |

### Architecture Summary

```
┌─────────────────────────────────────────┐
│  Spotnet App    │  SABnzbd Alternative  │
├─────────────────┴───────────────────────┤
│              usenet-dl                  │  ← THIS PROJECT (not implemented)
│  - Queue management                     │
│  - Post-processing (extract, rename)    │
│  - REST API (Axum + OpenAPI)            │
│  - SQLite persistence                   │
│  - Event system (tokio::broadcast)      │
│  - Automation (RSS, watch folders)      │
├─────────────────────────────────────────┤
│              nntp-rs                    │  ← DEPENDENCY (production ready)
│  - NNTP protocol                        │
│  - NZB parsing                          │
│  - yEnc decoding                        │
│  - PAR2 verification                    │
│  - Connection pooling                   │
└─────────────────────────────────────────┘
```

### Key Dependencies

The implementation will require these major dependencies:
- **Core:** tokio, sqlx (SQLite), serde, tracing, thiserror
- **Archives:** unrar, sevenz-rust, zip
- **REST API:** axum, tower, tower-http, utoipa, utoipa-swagger-ui
- **Automation:** reqwest, notify, rss, atom_syndication, chrono
- **Utilities:** sha2, regex

### Critical Design Decisions

1. **Event System:** Using `tokio::broadcast` for multiple subscribers (UI, logging, webhooks)
2. **Database:** SQLite with article-level download tracking for resume support
3. **API:** OpenAPI 3.1 compliant REST API with Swagger UI
4. **Defaults:** Everything works out-of-box (only NNTP server config required)
5. **Safety:** Never silently overwrite files, preserve failed downloads by default
6. **Post-Processing Pipeline:** Download → Verify → Repair → Extract → Rename → Move → Cleanup

### Dependency Challenges & Contingencies

1. **PAR2 Repair:** nntp-rs does NOT implement PAR2 repair (only verification)
   - **Contingency:** May need external `par2cmdline` tool or skip repair in MVP
   - **Plan:** Start with verification only, add repair in Phase 5 if needed

2. **Archive Extraction:** Need RAR, 7z, ZIP support
   - **Contingency:** Start with ZIP (built-in), add RAR/7z incrementally
   - **Plan:** Test unrar and sevenz-rust compatibility early

3. **Password-Protected Archives:** Need to test multiple passwords
   - **Contingency:** May need external tools if Rust crates don't support password testing
   - **Plan:** Prototype password testing in Phase 2 step 11

4. **Disk Space Checking:** Cross-platform fs stat differences
   - **Contingency:** Use nix/libc crates for statvfs on Linux, platform-specific for Windows
   - **Plan:** Implement Linux first, add Windows support in Phase 5

5. **External Script Execution:** Async Command with timeout
   - **Contingency:** Scripts may hang or fail silently
   - **Plan:** Use tokio::time::timeout and non-blocking execution (fire-and-forget)

## Task List

### Phase 0: Project Initialization (NEW)

- [x] Task 0.1: Create Cargo.toml with workspace structure and core dependencies
- [x] Task 0.2: Create src/ directory structure (lib.rs, modules, error.rs)
- [x] Task 0.3: Add nntp-rs as path dependency and verify it compiles
- [x] Task 0.4: Set up initial module structure (config, types, error, db, events)
- [x] Task 0.5: Create basic README.md with getting-started instructions

### Phase 1: Core Library (Steps 1-9)

- [x] Task 1.1: Define core types (DownloadId, Status, Priority, Stage enums)
- [x] Task 1.2: Implement Config structure with Default trait (all 40+ settings)
- [x] Task 1.3: Implement RetryConfig with exponential backoff logic
- [x] Task 1.4: Create DownloadInfo, DownloadOptions, HistoryEntry types

- [x] Task 2.1: Create SQLite schema (downloads, download_articles, passwords, processed_nzbs, history)
- [x] Task 2.2: Implement Database struct with sqlx connection pool
- [x] Task 2.3: Implement CRUD operations for downloads table
- [x] Task 2.4: Implement article-level tracking (insert, update, query pending articles)
- [x] Task 2.5: Add password cache operations (set_correct_password, get_cached_password)
- [x] Task 2.6: Add duplicate detection queries (find_by_nzb_hash, find_by_name, find_by_job_name)
- [ ] Task 2.7: Implement history operations (insert, query, cleanup)
- [x] Task 2.8: Add database migration system (sqlx migrations or embedded SQL)

- [ ] Task 3.1: Create Event enum with all event types (Queued, Downloading, Complete, Failed, etc.)
- [ ] Task 3.2: Implement Stage enum (Download, Verify, Repair, Extract, Move, Cleanup)
- [ ] Task 3.3: Set up tokio::broadcast channel in UsenetDownloader
- [ ] Task 3.4: Implement subscribe() method returning broadcast::Receiver<Event>
- [ ] Task 3.5: Add event emission throughout codebase (emit_event helper method)

- [ ] Task 4.1: Create UsenetDownloader struct with fields (db, event_tx, config, nntp_pool)
- [ ] Task 4.2: Implement UsenetDownloader::new(config) constructor
- [ ] Task 4.3: Create nntp-rs connection pool (NntpPool) from ServerConfig
- [ ] Task 4.4: Implement add_nzb_content() to parse NZB and create download record
- [ ] Task 4.5: Implement add_nzb() to read file and delegate to add_nzb_content()
- [ ] Task 4.6: Create download task spawner (spawn_download_task)
- [ ] Task 4.7: Implement basic article downloading loop using nntp-rs
- [ ] Task 4.8: Add progress tracking (update download progress in DB and emit events)

- [ ] Task 5.1: Implement priority queue (BinaryHeap or sorted Vec with Priority ordering)
- [ ] Task 5.2: Add queue management (add, remove, reorder by priority)
- [ ] Task 5.3: Implement max_concurrent_downloads limiter (Semaphore)
- [ ] Task 5.4: Create queue processor task that spawns downloads
- [ ] Task 5.5: Implement pause() to stop download without removing from queue
- [ ] Task 5.6: Implement resume() to restart paused download
- [ ] Task 5.7: Implement cancel() to remove download and delete files
- [ ] Task 5.8: Add pause_all() and resume_all() queue-wide operations
- [ ] Task 5.9: Persist queue state to SQLite on every change

- [ ] Task 6.1: Implement article status tracking in download_articles table
- [ ] Task 6.2: Create resume_download() to query pending articles and continue
- [ ] Task 6.3: Implement restore_queue() called on startup
- [ ] Task 6.4: Handle incomplete downloads (status=Downloading) on startup
- [ ] Task 6.5: Handle processing downloads (status=Processing) on startup
- [ ] Task 6.6: Test resume after simulated crash (kill process mid-download)

- [ ] Task 7.1: Implement SpeedLimiter with token bucket algorithm
- [ ] Task 7.2: Use AtomicU64 for lock-free token tracking
- [ ] Task 7.3: Implement acquire(bytes) async method with wait logic
- [ ] Task 7.4: Share SpeedLimiter (Arc) across all download tasks
- [ ] Task 7.5: Implement set_speed_limit(limit_bps) to change limit dynamically
- [ ] Task 7.6: Emit SpeedLimitChanged event when limit is updated
- [ ] Task 7.7: Test speed limiting with multiple concurrent downloads

- [ ] Task 8.1: Create IsRetryable trait for error classification
- [ ] Task 8.2: Implement download_with_retry() generic function
- [ ] Task 8.3: Add jitter calculation (rand crate for randomization)
- [ ] Task 8.4: Classify nntp-rs errors (NntpError) into retryable vs non-retryable
- [ ] Task 8.5: Add retry attempt tracking and logging
- [ ] Task 8.6: Test retry with simulated transient failures

- [ ] Task 9.1: Implement shutdown() method with graceful sequence
- [ ] Task 9.2: Add accepting_new flag (AtomicBool) to stop new downloads
- [ ] Task 9.3: Implement pause_graceful() to finish current article
- [ ] Task 9.4: Add wait_for_articles() with timeout
- [ ] Task 9.5: Implement persist_all_state() to save final state
- [ ] Task 9.6: Set up signal handling (SIGTERM, SIGINT) using tokio::signal
- [ ] Task 9.7: Add shutdown flag to database (was_unclean_shutdown check)
- [ ] Task 9.8: Test graceful shutdown and recovery on restart

### Phase 2: Post-Processing (Steps 10-16)

- [ ] Task 10.1: Create PostProcess enum (None, Verify, Repair, Unpack, UnpackAndCleanup)
- [ ] Task 10.2: Define post-processing pipeline trait/interface
- [ ] Task 10.3: Implement start_post_processing() entry point
- [ ] Task 10.4: Create stage executor (run_stage function that calls verify/repair/extract)
- [ ] Task 10.5: Add post-processing state machine (track current stage in DB)
- [ ] Task 10.6: Emit stage events (Verifying, Extracting, Moving, Cleaning)

- [ ] Task 11.1: Integrate unrar crate for RAR extraction
- [ ] Task 11.2: Implement detect_rar_files() to find .rar/.r00 archives
- [ ] Task 11.3: Create PasswordList collector (from cache, download, NZB meta, file, empty)
- [ ] Task 11.4: Implement try_extract() with single password attempt
- [ ] Task 11.5: Implement extract_with_passwords() loop over PasswordList
- [ ] Task 11.6: Cache successful password in SQLite
- [ ] Task 11.7: Handle extraction errors (wrong password vs corrupt archive)
- [ ] Task 11.8: Test RAR extraction with no password, single password, multiple attempts

- [ ] Task 12.1: Integrate sevenz-rust crate for 7z extraction
- [ ] Task 12.2: Integrate zip crate for ZIP extraction
- [ ] Task 12.3: Implement detect_archive_type() by extension
- [ ] Task 12.4: Create unified extract_archive() dispatcher
- [ ] Task 12.5: Add password support for 7z and ZIP
- [ ] Task 12.6: Test 7z and ZIP extraction with passwords

- [ ] Task 13.1: Implement ExtractionConfig with max_recursion_depth and archive_extensions
- [ ] Task 13.2: Create extract_recursive() with depth tracking
- [ ] Task 13.3: Implement is_archive() helper to check extensions
- [ ] Task 13.4: Test nested extraction (archive within archive)
- [ ] Task 13.5: Add safeguard against infinite recursion (depth limit)

- [ ] Task 14.1: Implement is_obfuscated() with heuristics (entropy, UUID, hex, no vowels)
- [ ] Task 14.2: Create DeobfuscationConfig with enabled flag and min_length
- [ ] Task 14.3: Implement determine_final_name() with priority order (job name, NZB meta, largest file)
- [ ] Task 14.4: Add NZB metadata parsing for <meta type="name">
- [ ] Task 14.5: Implement find_largest_file() helper
- [ ] Task 14.6: Test deobfuscation with obfuscated and normal filenames

- [ ] Task 15.1: Implement FileCollisionAction enum (Rename, Overwrite, Skip)
- [ ] Task 15.2: Create get_unique_path() with (1), (2) suffix logic
- [ ] Task 15.3: Implement move_files() to final destination with collision handling
- [ ] Task 15.4: Add category destination resolution
- [ ] Task 15.5: Emit Moving event with destination path
- [ ] Task 15.6: Test file collision handling (rename, overwrite, skip modes)

- [ ] Task 16.1: Define cleanup target file extensions (.par2, .nzb, .sfv, .srr, archives)
- [ ] Task 16.2: Implement delete_samples flag and folder detection
- [ ] Task 16.3: Create cleanup() function to remove intermediate files
- [ ] Task 16.4: Add error handling (log warnings, don't fail on cleanup errors)
- [ ] Task 16.5: Emit Cleaning event
- [ ] Task 16.6: Test cleanup with various file types

### Phase 3: REST API (Steps 17-23)

- [ ] Task 17.1: Add axum, tower, tower-http dependencies
- [ ] Task 17.2: Create ApiConfig struct with bind_address, api_key, cors, swagger_ui, rate_limit
- [ ] Task 17.3: Implement create_router() with all route definitions
- [ ] Task 17.4: Create AppState with Arc<UsenetDownloader> for handler access
- [ ] Task 17.5: Implement API server startup (tokio::spawn api_server)
- [ ] Task 17.6: Add CORS middleware (tower-http CorsLayer)
- [ ] Task 17.7: Add optional authentication middleware (check X-Api-Key header)
- [ ] Task 17.8: Test API server starts and responds to /health

- [ ] Task 18.1: Add utoipa and utoipa-swagger-ui dependencies
- [ ] Task 18.2: Annotate all types with #[derive(ToSchema)]
- [ ] Task 18.3: Annotate all route handlers with #[utoipa::path]
- [ ] Task 18.4: Create ApiDoc struct with #[derive(OpenApi)]
- [ ] Task 18.5: Implement /openapi.json endpoint serving OpenAPI spec
- [ ] Task 18.6: Mount Swagger UI at /swagger-ui
- [ ] Task 18.7: Test Swagger UI loads and shows all endpoints

- [ ] Task 19.1: Implement GET /downloads (list_downloads handler)
- [ ] Task 19.2: Implement GET /downloads/:id (get_download handler)
- [ ] Task 19.3: Implement POST /downloads with multipart/form-data (add_download handler)
- [ ] Task 19.4: Implement POST /downloads/url (add_download_url handler)
- [ ] Task 19.5: Implement POST /downloads/:id/pause (pause_download handler)
- [ ] Task 19.6: Implement POST /downloads/:id/resume (resume_download handler)
- [ ] Task 19.7: Implement DELETE /downloads/:id (delete_download handler)
- [ ] Task 19.8: Implement PATCH /downloads/:id/priority (set_priority handler)
- [ ] Task 19.9: Implement POST /downloads/:id/reprocess (reprocess handler)
- [ ] Task 19.10: Implement POST /downloads/:id/reextract (reextract handler)
- [ ] Task 19.11: Implement POST /queue/pause (pause_all handler)
- [ ] Task 19.12: Implement POST /queue/resume (resume_all handler)
- [ ] Task 19.13: Implement GET /queue/stats (queue_stats handler)
- [ ] Task 19.14: Implement GET /history with pagination (get_history handler)
- [ ] Task 19.15: Implement DELETE /history (clear_history handler)
- [ ] Task 19.16: Test all queue endpoints with curl/Postman

- [ ] Task 20.1: Add tokio-stream dependency
- [ ] Task 20.2: Implement GET /events endpoint with text/event-stream response
- [ ] Task 20.3: Convert tokio::broadcast events to SSE format (event: type, data: json)
- [ ] Task 20.4: Handle client disconnects gracefully
- [ ] Task 20.5: Test SSE stream with curl -N or browser EventSource

- [ ] Task 21.1: Implement GET /config (get_config handler) with sensitive field redaction
- [ ] Task 21.2: Implement PATCH /config (update_config handler)
- [ ] Task 21.3: Implement GET /config/speed-limit (get_speed_limit handler)
- [ ] Task 21.4: Implement PUT /config/speed-limit (set_speed_limit handler)
- [ ] Task 21.5: Implement GET /categories (list_categories handler)
- [ ] Task 21.6: Implement PUT /categories/:name (create_or_update_category handler)
- [ ] Task 21.7: Implement DELETE /categories/:name (delete_category handler)
- [ ] Task 21.8: Test config endpoints

- [ ] Task 22.1: Verify Swagger UI shows all endpoints with schemas
- [ ] Task 22.2: Test Swagger UI "Try it out" functionality for each endpoint
- [ ] Task 22.3: Verify OpenAPI spec is valid (use openapi-generator validate)
- [ ] Task 22.4: Test API documentation completeness

- [ ] Task 23.1: Add tower-governor dependency
- [ ] Task 23.2: Create RateLimitConfig with requests_per_second, burst_size, exempt_paths, exempt_ips
- [ ] Task 23.3: Implement conditional rate limiting layer (only if enabled)
- [ ] Task 23.4: Add exempt path/IP checking logic
- [ ] Task 23.5: Test rate limiting returns 429 when exceeded
- [ ] Task 23.6: Verify exempt paths are not rate limited

### Phase 4: Automation (Steps 24-28)

- [ ] Task 24.1: Add notify crate dependency
- [ ] Task 24.2: Create WatchFolderConfig with path, after_import, category, scan_interval
- [ ] Task 24.3: Implement WatchFolderAction enum (Delete, MoveToProcessed, Keep)
- [ ] Task 24.4: Create FolderWatcher struct with notify::Watcher
- [ ] Task 24.5: Implement watch_folder() task that monitors directory
- [ ] Task 24.6: Process .nzb files found in folder (call add_nzb)
- [ ] Task 24.7: Handle after_import action (delete, move, or track in processed_nzbs table)
- [ ] Task 24.8: Test folder watching with file creation
- [ ] Task 24.9: Add multiple watch folder support
- [ ] Task 24.10: Implement category-specific watch folders

- [ ] Task 25.1: Add reqwest dependency
- [ ] Task 25.2: Implement add_nzb_url() to fetch NZB from HTTP
- [ ] Task 25.3: Extract filename from Content-Disposition or URL
- [ ] Task 25.4: Handle HTTP errors (404, 403, timeout)
- [ ] Task 25.5: Test URL fetching with various NZB URLs

- [ ] Task 26.1: Add rss and atom_syndication dependencies
- [ ] Task 26.2: Create RssFeedConfig with url, check_interval, category, filters, auto_download, priority
- [ ] Task 26.3: Create RssFilter with include/exclude patterns, min/max size, max age
- [ ] Task 26.4: Add RSS feed tables to SQLite schema (rss_feeds, rss_filters, rss_seen)
- [ ] Task 26.5: Implement RssManager struct
- [ ] Task 26.6: Implement check_feed() to fetch and parse RSS/Atom
- [ ] Task 26.7: Implement matches_filters() using regex for include/exclude
- [ ] Task 26.8: Track seen items in rss_seen table (guid or link)
- [ ] Task 26.9: Auto-download matching items if auto_download=true
- [ ] Task 26.10: Implement scheduled feed checking task
- [ ] Task 26.11: Add API endpoints for RSS management (GET/POST/PUT/DELETE /rss, POST /rss/:id/check)
- [ ] Task 26.12: Test RSS feed with real indexer feed

- [ ] Task 27.1: Create ScheduleRule with name, days, start_time, end_time, action, enabled
- [ ] Task 27.2: Implement ScheduleAction enum (SpeedLimit, Unlimited, Pause)
- [ ] Task 27.3: Implement Weekday enum
- [ ] Task 27.4: Create Scheduler struct with rules list
- [ ] Task 27.5: Implement get_current_action() to evaluate rules for current time
- [ ] Task 27.6: Create scheduler task that checks rules every minute
- [ ] Task 27.7: Apply actions (set speed limit or pause queue)
- [ ] Task 27.8: Add API endpoints for scheduler management (GET/POST/PUT/DELETE /scheduler)
- [ ] Task 27.9: Test schedule rules with time changes

- [ ] Task 28.1: Create DuplicateConfig with enabled, action, methods
- [ ] Task 28.2: Implement DuplicateAction enum (Block, Warn, Allow)
- [ ] Task 28.3: Implement DuplicateMethod enum (NzbHash, NzbName, JobName)
- [ ] Task 28.4: Add nzb_hash and job_name columns to downloads table
- [ ] Task 28.5: Implement check_duplicate() with sha256 hashing
- [ ] Task 28.6: Add duplicate detection to add_nzb_content()
- [ ] Task 28.7: Emit warning event or block based on DuplicateAction
- [ ] Task 28.8: Test duplicate detection with same NZB added twice

### Phase 5: Notifications & Polish (Steps 29-35)

- [ ] Task 29.1: Create WebhookConfig with url, events, auth_header, timeout
- [ ] Task 29.2: Implement WebhookEvent enum (OnComplete, OnFailed, OnQueued)
- [ ] Task 29.3: Create WebhookPayload struct (event, download_id, name, category, status, destination, error, timestamp)
- [ ] Task 29.4: Implement trigger_webhooks() to POST to configured URLs
- [ ] Task 29.5: Add timeout and error handling (don't fail download on webhook failure)
- [ ] Task 29.6: Emit WebhookFailed event on error
- [ ] Task 29.7: Test webhook with httpbin.org/post

- [ ] Task 30.1: Create ScriptConfig with path, events, timeout
- [ ] Task 30.2: Implement ScriptEvent enum (OnComplete, OnFailed, OnPostProcessComplete)
- [ ] Task 30.3: Define environment variables (USENET_DL_ID, USENET_DL_NAME, etc.)
- [ ] Task 30.4: Implement run_script_async() using tokio::process::Command
- [ ] Task 30.5: Add timeout handling with tokio::time::timeout
- [ ] Task 30.6: Emit ScriptFailed event on error
- [ ] Task 30.7: Implement category-specific scripts in CategoryConfig
- [ ] Task 30.8: Execute category scripts before global scripts
- [ ] Task 30.9: Test script execution with sample script

- [ ] Task 31.1: Create DiskSpaceConfig with enabled, min_free_space, size_multiplier
- [ ] Task 31.2: Implement get_available_space() using platform-specific APIs (statvfs on Linux)
- [ ] Task 31.3: Implement check_disk_space() before download
- [ ] Task 31.4: Create DiskSpaceError enum (InsufficientSpace, CheckFailed)
- [ ] Task 31.5: Test disk space check with low space scenario

- [ ] Task 32.1: Implement test_server() to verify connectivity and authentication
- [ ] Task 32.2: Create ServerTestResult with success, latency, error, capabilities
- [ ] Task 32.3: Create ServerCapabilities struct (posting_allowed, max_connections, compression)
- [ ] Task 32.4: Implement test_all_servers() to check all configured servers
- [ ] Task 32.5: Add API endpoints POST /servers/test and GET /servers/test
- [ ] Task 32.6: Test server health check with real NNTP server

- [ ] Task 33.1: Implement reprocess() to re-run full post-processing
- [ ] Task 33.2: Verify download files still exist before reprocessing
- [ ] Task 33.3: Reset status to Processing and emit Verifying event
- [ ] Task 33.4: Implement reextract() to skip verify/repair
- [ ] Task 33.5: Test reprocessing on completed and failed downloads

- [ ] Task 34.1: Create comprehensive error types (DownloadError, PostProcessError, ApiError)
- [ ] Task 34.2: Implement Error trait and Display for all error types
- [ ] Task 34.3: Add context to errors (which stage, which file, etc.)
- [ ] Task 34.4: Implement ApiError JSON response format with code, message, details
- [ ] Task 34.5: Add HTTP status code mapping for API errors
- [ ] Task 34.6: Test error responses in API

- [ ] Task 35.1: Write comprehensive README.md (features, installation, usage, configuration)
- [ ] Task 35.2: Create examples/ directory with sample code
- [ ] Task 35.3: Write API usage documentation with curl examples
- [ ] Task 35.4: Document configuration file format (TOML or JSON)
- [ ] Task 35.5: Create CHANGELOG.md
- [ ] Task 35.6: Write CONTRIBUTING.md with development guidelines
- [ ] Task 35.7: Add inline code documentation (rustdoc comments)
- [ ] Task 35.8: Generate and verify cargo doc output

## Completed This Iteration

**Phase 1 Duplicate Detection Queries - Task 2.6 Complete**

- Task 2.6: Implemented duplicate detection queries for preventing re-downloads ✓
  - find_by_nzb_hash(hash) - finds download by NZB content hash (most reliable method)
  - find_by_name(name) - finds download by exact name match (case-sensitive)
  - find_by_job_name(job_name) - finds download by deobfuscated job name (catches renamed NZBs)
  - All methods return Option<Download> and use LIMIT 1 for safety
  - Leverages existing indexes (idx_downloads_nzb_hash, idx_downloads_job_name) for fast lookups
  - 7 comprehensive tests verify all duplicate detection scenarios:
    - test_find_by_nzb_hash - basic hash-based detection
    - test_find_by_nzb_hash_multiple - multiple downloads with different hashes
    - test_find_by_name - exact name matching (case-sensitive)
    - test_find_by_name_returns_first_match - LIMIT 1 behavior with duplicates
    - test_find_by_job_name - deobfuscated job name detection
    - test_find_by_job_name_null_handling - NULL vs string comparison
    - test_duplicate_detection_priority - all three methods find the same download
  - All 25 database tests passing (18 from previous tasks + 7 new duplicate detection tests)
  - Ready for integration with duplicate detection logic in Phase 4 (Task 28)

### Implementation Details

**Database Module (src/db.rs):**
Created a complete database layer with:
- `Database` struct wrapping SqlitePool for connection management
- Auto-creation of database file and parent directories
- Migration system with `schema_version` table for tracking applied migrations
- Migration v1 creates all 5 tables with proper foreign keys and indexes:
  - `downloads`: Main download queue with 18 columns including nzb_hash, job_name for duplicate detection
  - `download_articles`: Article-level tracking for resume support (message_id, segment_number, status)
  - `passwords`: Password cache for successful archive extractions
  - `processed_nzbs`: Track processed NZB files for watch folder "Keep" action
  - `history`: Historical records with completed_at timestamp
- 8 indexes for query optimization (status, priority, nzb_hash, job_name, articles, history)
- Full test coverage with 2 passing tests (creation and migration idempotency)

**Error Handling:**
- Added `Database(String)` error variant for custom error messages
- Kept `Sqlx(sqlx::Error)` for automatic conversion from sqlx errors
- All database operations return `Result<T>` with proper error context

**Key Design Decisions:**
1. Embedded migrations (no external .sql files) - simpler deployment
2. Integer timestamps (Unix epoch) - SQLite-friendly, easy to work with
3. Cascade deletes on foreign keys - automatic cleanup when download is removed
4. AUTOINCREMENT on primary keys - prevents ID reuse after deletion
5. Idempotent migrations - safe to run Database::new() multiple times

## Notes

### Architecture Dependencies

**Critical Path:** Tasks must be completed in phase order due to dependencies:
- Phase 1 (Core) must be complete before Phase 2 (Post-processing)
- Phase 3 (API) depends on Phase 1 and 2
- Phase 4 (Automation) depends on Phase 1 and 3
- Phase 5 (Polish) depends on all previous phases

**Within Phase 1:**
1. Types (Task 1) → Config (Task 2)
2. Config → Database (Tasks 2.1-2.8)
3. Database + Types → Events (Tasks 3.1-3.5)
4. Events + Database → Download Manager (Tasks 4.1-4.8)
5. Download Manager → Queue (Tasks 5.1-5.9)
6. Queue + Database → Resume (Tasks 6.1-6.6)
7. Download Manager → Speed Limiting (Tasks 7.1-7.7)
8. Download Manager → Retry (Tasks 8.1-8.6)
9. All Core → Shutdown (Tasks 9.1-9.8)

**Within Phase 2:**
1. Pipeline skeleton (Task 10) → All extraction tasks (11-13)
2. Extraction → Deobfuscation (Task 14)
3. Deobfuscation → File Organization (Task 15)
4. File Organization → Cleanup (Task 16)

**Within Phase 3:**
1. API Server Setup (Task 17) → All other API tasks
2. OpenAPI (Task 18) must be done before Swagger UI (Task 22)

**Within Phase 4:**
All tasks are independent except RSS needs SQLite schema updates

**Within Phase 5:**
All tasks are independent except error handling (Task 34) should be early

### Testing Strategy

**Unit Tests:**
- Test each module in isolation (types, config, database, events)
- Mock nntp-rs for download manager tests
- Test retry logic with simulated failures
- Test speed limiting with controlled byte transfers
- Test duplicate detection with known hashes

**Integration Tests:**
- Test full download pipeline with real nntp-rs connection
- Test post-processing with sample archives
- Test API endpoints with HTTP client
- Test folder watching with temp directories
- Test RSS feed processing with sample feeds

**Edge Cases:**
- Resume after crash (kill process mid-download)
- Disk full during extraction
- Password-protected archives with wrong passwords
- Obfuscated filenames (UUID-like, hex, high entropy)
- File collisions (multiple files with same name)
- Nested archives (3+ levels)
- Duplicate detection (same NZB added twice)
- Scheduler rule conflicts (overlapping time ranges)
- Rate limit enforcement with concurrent requests

### Performance Considerations

**Database:**
- Use connection pool (sqlx) for concurrent access
- Add indexes on frequently queried columns (status, priority, nzb_hash)
- Batch insert for article tracking (insert multiple segments at once)

**Download Speed:**
- nntp-rs handles connection pooling and compression
- Token bucket speed limiter should be lock-free (AtomicU64)
- Avoid blocking operations in download tasks

**Post-Processing:**
- Run extraction in separate task pool (don't block download tasks)
- Stream archive extraction (don't load entire archive in memory)
- Cleanup can be deferred (run in background)

**API:**
- Use async handlers throughout (no blocking I/O)
- Implement pagination for large result sets (history, downloads)
- Consider caching for expensive queries (queue stats)

### Security Considerations

**API:**
- Bind to localhost by default (127.0.0.1:6789)
- Optional API key authentication
- CORS enabled for frontend development (restrictable in production)
- Rate limiting disabled by default (trust local network)

**Script Execution:**
- Scripts run with same privileges as usenet-dl process
- Use timeout to prevent hung scripts
- Environment variables only (no shell expansion)

**Archive Extraction:**
- Validate archive before extraction (avoid zip bombs)
- Extract to temp directory first (prevent directory traversal)
- Limit recursion depth (prevent infinite loops)

**Database:**
- Use parameterized queries (sqlx prevents SQL injection)
- Store sensitive data (passwords) with caution (consider encryption in future)

### Known Limitations & Future Work

**PAR2 Repair:**
- nntp-rs only does verification, not repair
- May need external par2cmdline tool or implement Reed-Solomon
- Can be added in future release

**STARTTLS:**
- nntp-rs only supports implicit TLS (port 563)
- Most modern servers use implicit TLS, so this is acceptable

**Posting:**
- Read-only client (no POST/IHAVE support)
- Out of scope for download manager

**Windows Support:**
- Initial implementation focuses on Linux
- Disk space checking needs platform-specific code
- Archive tools (unrar, 7z) need Windows compatibility testing

**Internationalization:**
- No i18n support initially (English only)
- Can be added in future release

### Success Criteria

**Phase 1 Complete:**
- Download NZB from file or URL
- Queue management (add, pause, resume, cancel)
- Resume downloads after restart
- Speed limiting works correctly
- Graceful shutdown preserves state

**Phase 2 Complete:**
- Extract RAR/7z/ZIP archives with passwords
- Handle nested archives
- Deobfuscate filenames
- Move files to destination with collision handling
- Clean up intermediate files

**Phase 3 Complete:**
- REST API serves all endpoints
- Swagger UI is accessible and functional
- Server-Sent Events stream real-time updates
- Configuration can be updated via API

**Phase 4 Complete:**
- Watch folders auto-import NZBs
- RSS feeds are monitored and matched
- Scheduler applies time-based rules
- Duplicate detection prevents re-downloads

**Phase 5 Complete:**
- Webhooks notify external systems
- Scripts execute on events
- Disk space is checked before download
- Server health can be tested
- Full documentation exists

**Definition of Done:**
- All tests pass (unit + integration)
- API documentation is complete (Swagger UI)
- README has getting-started guide
- Example code demonstrates usage
- No critical bugs or blocking issues

### Timeline Estimate

This is a planning phase - NO timeline estimates as per instructions. Tasks will be completed systematically in order.

### Resources Needed

**Development:**
- Rust toolchain (stable)
- SQLite (for development/testing)
- NNTP server access (for testing)
- Test NZB files with various archive types

**Testing:**
- Sample NZB files (small, large, multi-part)
- Archives (RAR, 7z, ZIP) with and without passwords
- Obfuscated filenames (for deobfuscation testing)
- Nested archives (archive within archive)

**Documentation:**
- Markdown editor
- OpenAPI validator
- Curl or Postman for API testing
