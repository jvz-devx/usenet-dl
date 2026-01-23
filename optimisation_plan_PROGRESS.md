# Progress: optimisation_plan

Started: vr 23 jan 2026 14:21:32 CET

## Status

IN_PROGRESS

## Analysis

### Codebase Exploration Summary

I've completed a thorough exploration of the codebase to understand what exists and what needs to be implemented for parallel downloads:

#### What Already Exists

1. **NNTP Connection Pools** (fully implemented)
   - Using `nntp_rs::NntpPool` built on `bb8` connection pool manager
   - Per-server pooling: `Vec<NntpPool>` stored in `Arc` for thread-safe sharing
   - Pool methods: `get()`, `get_no_retry()`, `state()`, `connections_in_use()`, `idle_connections()`
   - Configuration: Each server can have a different number of connections (default: 10)
   - Location: Created in `src/lib.rs:185-191`, used at lines 3168-3187, 3697-3703

2. **Sequential Download Loops** (two locations need parallelization)
   - **Queue processor loop**: `src/lib.rs:3153-3310` - Downloads articles for queued downloads
   - **Direct download loop**: `src/lib.rs:3694-3794` - Downloads articles in `download_nzb()` method
   - Both currently use `for article in pending_articles` sequential iteration
   - Both use only the first pool (`nntp_pools.first()`)

3. **Speed Limiter** (fully compatible with concurrent downloads)
   - Lock-free token bucket implementation using `AtomicU64`
   - Located in `src/speed_limiter.rs`
   - Already used at line 3208: `speed_limiter_clone.acquire(article.size_bytes as u64).await`
   - Thread-safe, concurrent-safe, no modifications needed
   - Tests confirm it works with concurrent acquires

4. **Concurrency Primitives Already in Use**
   - `tokio::sync::broadcast` - Event distribution (1000-event buffer)
   - `tokio::sync::Mutex` - Queue and active downloads protection
   - `tokio::sync::RwLock` - Categories and schedule rules
   - `tokio::sync::Semaphore` - Max concurrent downloads limit
   - `tokio_util::sync::CancellationToken` - Download pause/cancel
   - `std::sync::Arc` - Thread-safe sharing of all state
   - `std::sync::atomic::AtomicU64/AtomicBool/AtomicI64` - Lock-free counters

5. **Existing Streaming/Async Patterns**
   - `tokio-stream` is in `Cargo.toml` with `["sync"]` features
   - Used in `src/api/routes.rs:1456-1521` for SSE event streaming
   - Pattern: `BroadcastStream::new()` with `.filter_map()` extensions

#### What's Missing (Needs Implementation)

1. **`futures` crate** - NOT currently a dependency
   - Need to add to `Cargo.toml` for `futures::stream::StreamExt` and `buffer_unordered()`
   - Only `tokio-stream` is present, not the full `futures` crate

2. **Parallel download implementation** - Sequential loops need conversion
   - Need to replace `for article in pending_articles` with `stream::iter().buffer_unordered()`
   - Two locations: queue processor (line 3153) and direct download (line 3694)

3. **Concurrency calculation** - Need to determine buffer size
   - Pool has no public `max_size()` method exposed
   - Options:
     - A) Sum from config: `config.servers.iter().map(|s| s.connections).sum()`
     - B) Use pool state: `pool.state()` returns `bb8::State` (has connection counts)

4. **Progress tracking for out-of-order completion**
   - Current: Sequential progress updates (lines 3249-3284, 3737-3771)
   - Need: Atomic counters or channel-based progress for concurrent updates
   - Options: `Arc<AtomicU64>` (simpler) or `mpsc::channel` (more flexible)

5. **Error handling strategy**
   - Current: First article failure aborts entire download (lines 3286-3309, 3773-3792)
   - Need: Collect all results, handle failures gracefully, potentially retry

6. **Cancellation support in parallel context**
   - Current: Check `cancel_token.is_cancelled()` before each article (line 3155)
   - Need: Check inside each parallel future

#### Dependencies and Prerequisites

1. **No breaking changes to existing code** - Speed limiter, pools, and sync primitives all work as-is
2. **Multi-server support NOT blocking** - Plan mentions using first pool only, which is current behavior
3. **Database and event systems compatible** - Already handle concurrent updates from multiple spawned tasks

### Key Implementation Decisions

1. **Use `futures::stream::iter().buffer_unordered()`** as specified in the plan
2. **Calculate concurrency from config** - Safer than relying on pool internals: `config.servers.iter().map(|s| s.connections).sum()`
3. **Use `Arc<AtomicU64>` for progress tracking** - Simpler than channels, matches existing patterns (speed_limiter.rs)
4. **Collect results in a `Vec`** - Process errors after all downloads complete or fail
5. **Clone cancel_token into each future** - Check at start of each article download
6. **Preserve existing error messages and event emissions** - Keep user-facing behavior consistent

### Potential Issues and Mitigations

1. **Pool method access** - If we need `max_size()` and it's not public:
   - Mitigation: Use config values instead (`server.connections`)
   - Already have access to `config` field in `UsenetDownloader`

2. **Database concurrent writes** - Multiple article status updates:
   - Not an issue: `sqlx` handles concurrent writes, database already used from multiple tasks

3. **Event channel saturation** - Many concurrent progress events:
   - Channel has 1000-event buffer (line 182)
   - Broadcast channels drop old events if full (acceptable for progress updates)

4. **Memory usage** - Buffering futures:
   - Plan confirms: Article content goes to disk, futures are tiny (~1KB each)
   - 50 connections × 1KB = 50KB overhead (negligible)

5. **Speed limiter fairness** - One download grabbing all tokens:
   - Current implementation: First-come-first-served
   - Acceptable: Global limit still enforced, downloads share bandwidth over time

## Task List

### Phase 1: Setup and Dependencies

- [x] Task 1.1: Add `futures` crate to `Cargo.toml` dependencies
  - Add: `futures = "0.3"` to `[dependencies]` section
  - File: `/home/jens/Documents/source/usenet-dl/Cargo.toml`

- [x] Task 1.2: Add necessary imports to `src/lib.rs`
  - Add at top of file: `use futures::stream::{self, StreamExt};`
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs`

### Phase 2: Progress Tracking Infrastructure

- [x] Task 2.1: Add atomic counter fields for parallel progress tracking
  - Before queue processor loop (around line 3130), replace existing counters with:
    ```rust
    let downloaded_articles = Arc::new(AtomicU64::new(0));
    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    ```
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs:3128-3129`

- [x] Task 2.2: Create progress reporting task for queue processor
  - Spawn separate task that periodically reads atomic counters and emits progress events
  - Prevents event spam while maintaining real-time updates
  - Location: After atomic counter setup, before article download loop
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs` (around line 3150)

### Phase 3: Parallelize Queue Processor Download Loop

- [x] Task 3.1: Calculate concurrency limit from config
  - Before article download loop (around line 3150):
    ```rust
    let concurrency: usize = config_clone.servers.iter()
        .map(|s| s.connections)
        .sum();
    ```
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs`

- [x] Task 3.2: Convert queue processor sequential loop to parallel stream (Part 1: Setup)
  - Replace `for article in pending_articles` (line 3153) with stream setup
  - Create the stream: `let results = stream::iter(pending_articles)`
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs:3153`

- [x] Task 3.3: Convert queue processor sequential loop to parallel stream (Part 2: Map function)
  - Add `.map()` closure with article download logic
  - Clone necessary variables: pool, db, speed_limiter, cancel_token, download_temp_dir, downloaded_bytes, downloaded_articles
  - Move article fetch logic (lines 3166-3247) into async closure
  - Keep error handling, but return `Result<(segment_number, size_bytes), Error>` instead of early return
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs:3153-3247`

- [x] Task 3.4: Convert queue processor sequential loop to parallel stream (Part 3: Buffer and collect)
  - Add `.buffer_unordered(concurrency)` after map
  - Add `.collect::<Vec<_>>().await` to get all results
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs:3153-3310`

- [x] Task 3.5: Add result processing logic for queue processor
  - After collect, iterate through results
  - Count successes and failures
  - Update atomic counters in map closure (increment on success)
  - Emit final progress event
  - Handle failures: decide whether to fail entire download or continue with partial results
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs` (after line 3310)

- [x] Task 3.6: Update cancellation handling in queue processor
  - Check `cancel_token.is_cancelled()` at start of each article's async closure
  - Return early if cancelled
  - Ensure cleanup still happens (active_downloads removal)
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs` (inside map closure)

- [x] Task 3.7: Preserve progress event emissions in queue processor
  - Update atomic counters in map closure: `downloaded_bytes.fetch_add(size, Ordering::Relaxed)`
  - Progress reporting task reads these and emits events
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs` (inside map closure)

### Phase 4: Parallelize Direct Download Loop

- [ ] Task 4.1: Add atomic counters for direct download method
  - In `download_nzb()` method, replace counters at lines 3674-3675 with `Arc<AtomicU64>`
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs:3674-3675`

- [ ] Task 4.2: Calculate concurrency for direct download
  - Same as Task 3.1, but using `config` reference (not `config_clone`)
  - Location: Around line 3690
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs`

- [ ] Task 4.3: Convert direct download sequential loop to parallel stream
  - Replace `for article in pending_articles` (line 3694)
  - Same pattern as Task 3.2-3.4: `stream::iter().map().buffer_unordered().collect()`
  - Move article fetch logic (lines 3695-3793) into async closure
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs:3694-3794`

- [ ] Task 4.4: Add result processing for direct download
  - Process collected results
  - Handle errors appropriately (currently returns error, may want to collect partial failures)
  - Update atomic counters in map closure
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs` (after line 3794)

- [ ] Task 4.5: Remove unused `article_data` vector in direct download
  - Line 3691: `let mut article_data = Vec::new();` - no longer needed
  - Line 3728: `article_data.push(...)` - remove (articles already saved to disk)
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs:3691,3728`

### Phase 5: Error Handling and Retry Strategy

- [ ] Task 5.1: Define error handling strategy for parallel downloads
  - Decide: Fail entire download on first error, or collect all errors and retry?
  - Plan suggests: Don't abort on single failure, mark articles as failed
  - Implementation: Collect both successes and failures in result processing
  - File: Design decision, affects Tasks 3.5 and 4.4

- [ ] Task 5.2: Implement failure collection in queue processor
  - In result processing (Task 3.5), separate `Ok` and `Err` results
  - Mark failed articles in database: `db.update_article_status(article.id, FAILED)`
  - Log warnings for failures: `tracing::warn!("Article fetch failed: {}", e)`
  - Only fail download if ALL articles fail or critical threshold exceeded
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs` (result processing section)

- [ ] Task 5.3: Implement failure collection in direct download
  - Same as Task 5.2 but for direct download method
  - May want to return partial success instead of hard error
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs` (result processing section)

### Phase 6: Testing and Validation

- [ ] Task 6.1: Create unit test for parallel article download
  - Test: Mock NNTP pool, verify concurrent fetches happen
  - Verify: Multiple articles downloaded in parallel (not sequential)
  - Verify: Progress tracking works correctly with atomic counters
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs` (test module) or `tests/`

- [ ] Task 6.2: Create integration test with real download
  - Requires: Test NZB file with multiple segments (e.g., 20+ articles)
  - Verify: Download completes successfully with parallel fetches
  - Verify: Speed increase compared to sequential baseline
  - Measure: Time improvement (should approach N× speedup with N connections)
  - File: `tests/` directory (integration test)

- [ ] Task 6.3: Test cancellation during parallel download
  - Start download with many articles
  - Cancel mid-download
  - Verify: All in-flight requests stop gracefully
  - Verify: Cleanup happens (active_downloads removed, status updated to Paused)
  - File: `tests/` directory (integration test)

- [ ] Task 6.4: Test error handling with failed articles
  - Simulate: Some articles return errors (missing from server)
  - Verify: Download continues for successful articles
  - Verify: Failed articles marked correctly in database
  - Verify: Appropriate events emitted
  - File: `tests/` directory (integration test)

- [ ] Task 6.5: Stress test with large NZB
  - Use: Large NZB with 1000+ segments
  - Verify: No memory leaks (article content goes to disk)
  - Verify: Speed limiter enforces global limit correctly
  - Measure: Memory usage stays constant
  - File: `tests/` directory (integration test)

- [ ] Task 6.6: Test progress reporting accuracy
  - Verify: Progress events show accurate byte counts
  - Verify: Progress percentage calculates correctly
  - Verify: Speed calculation (bytes/sec) is reasonable
  - Verify: No race conditions in atomic counter updates
  - File: `tests/` directory (integration test)

### Phase 7: Documentation and Cleanup

- [ ] Task 7.1: Update inline comments for parallel download sections
  - Document: Why `buffer_unordered` is used
  - Document: Concurrency calculation rationale
  - Document: Atomic counter usage for progress
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs` (parallel download sections)

- [ ] Task 7.2: Update rustdoc for affected methods
  - Update: `download_nzb()` method documentation
  - Update: Queue processor task documentation
  - Mention: Parallel download behavior, concurrency limits
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs`

- [ ] Task 7.3: Update CHANGELOG.md with performance improvement
  - Add entry: "Performance: Parallel article downloads using connection pool"
  - Mention: Expected speedup (N× with N connections)
  - File: `/home/jens/Documents/source/usenet-dl/CHANGELOG.md`

- [ ] Task 7.4: Consider updating README with performance notes
  - If README exists, mention parallel download capability
  - Document: How connection count affects performance
  - File: `/home/jens/Documents/source/usenet-dl/README.md` (if exists)

### Phase 8: Optional Enhancements (Future Work)

These are mentioned in the plan but not critical for the core parallel download feature:

- [ ] Task 8.1: Multi-server failover support
  - Currently: Only first pool used (lines 3168, 3697)
  - Enhancement: Try other pools if primary fails
  - Priority-based server selection
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs`

- [ ] Task 8.2: Advanced retry logic
  - Currently: Plan mentions "TODO: Add retry logic in Tasks 8.1-8.6" (lines 3291, 3779)
  - Enhancement: Exponential backoff retry for failed articles
  - Per-article retry counters
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs`

- [ ] Task 8.3: Dynamic concurrency adjustment
  - Enhancement: Adjust buffer size based on network conditions
  - Monitor: Article download times, error rates
  - Auto-tune: Increase/decrease concurrent downloads
  - File: New module or enhancement to existing download logic

- [ ] Task 8.4: Fair token distribution in speed limiter
  - Current: First-come-first-served token allocation
  - Enhancement: Proportional token allocation per download
  - Prevents: One download monopolizing bandwidth
  - File: `/home/jens/Documents/source/usenet-dl/src/speed_limiter.rs`

- [ ] Task 8.5: Burst capacity in speed limiter
  - Current: Token bucket capacity equals limit
  - Enhancement: Allow burst capacity (e.g., 2× limit) for better throughput
  - File: `/home/jens/Documents/source/usenet-dl/src/speed_limiter.rs`

- [ ] Task 8.6: Progress reporting optimizations
  - Current approach: Periodic task reading atomic counters
  - Enhancement: Adaptive update frequency based on download speed
  - Reduces: Event spam for fast downloads
  - File: `/home/jens/Documents/source/usenet-dl/src/lib.rs`

## Task Dependencies

```
Phase 1 (Setup)
  └─> Phase 2 (Progress Infrastructure)
       └─> Phase 3 (Queue Processor Parallelization)
            └─> Phase 5 (Error Handling for Queue Processor)
                 └─> Phase 6 (Testing)
                      └─> Phase 7 (Documentation)

Phase 1 (Setup)
  └─> Phase 4 (Direct Download Parallelization)
       └─> Phase 5 (Error Handling for Direct Download)
            └─> Phase 6 (Testing)
                 └─> Phase 7 (Documentation)

Phase 8 (Optional Enhancements) - Can be done anytime after Phase 3 & 4
```

**Critical Path**: Phase 1 → 2 → 3 → 5 → 6 → 7

**Parallel Work Possible**: Phases 3 and 4 can be worked on simultaneously after Phase 2

## Completed This Iteration

- Tasks 3.2-3.7: Converted queue processor sequential loop to parallel stream
  - Replaced the sequential `for article in pending_articles` loop (lines 3221-3353) with parallel stream
  - Created stream using `stream::iter(pending_articles)` with `.map()` and `.buffer_unordered(concurrency)`
  - Moved all article download logic into async closure that runs concurrently
  - Implemented proper error handling: collect all results, count successes/failures
  - Added cancellation check at start of each article's async closure
  - Updated atomic counters (downloaded_bytes, downloaded_articles) inside map closure
  - Progress reporting task (from Task 2.2) reads these counters and emits events periodically
  - Result processing: fail entire download if any articles fail (current strategy)
  - Fixed type issues: used `std::result::Result<(i32, u64), String>` instead of type alias
  - Fixed SpeedLimiter cloning: used `.clone()` instead of `Arc::clone()`
  - Location: src/lib.rs:3220-3340 (entire parallel download implementation)
  - Validated with `cargo check` - compiles successfully with only expected warnings
  - Downloads will now use all configured connections concurrently instead of sequentially

## Previously Completed

- Task 2.2: Create progress reporting task for queue processor
  - Created dedicated tokio task that runs every 500ms to emit progress events
  - Task reads atomic counters (downloaded_bytes and downloaded_articles) and calculates progress
  - Updates database progress and emits Event::Downloading at regular intervals
  - Prevents event spam that would occur with parallel downloads completing out of order
  - Uses tokio::select! to gracefully cancel when download finishes or is cancelled
  - Added .abort() calls at all exit points: success (line 3379), failure (line 3371), cancellation (line 3220), and no-pool error (line 3247)
  - Removed inline progress reporting code (lines 3320-3354) since the dedicated task handles it
  - Only atomic counter updates remain in the article download loop
  - Location: src/lib.rs:3154-3213 (task spawn), cleanup at multiple exit points
  - Validated with `cargo check` - compiles successfully with only expected warnings (unused imports will be used in next tasks)
  - Ready for parallel stream implementation

## Notes

### Implementation Strategy

The implementation follows the plan's "buffered stream" approach using `futures::stream::iter().buffer_unordered()`. This is the idiomatic Rust async pattern for bounded concurrency and provides:

1. **Automatic backpressure** - Won't overwhelm connection pool
2. **Natural cancellation** - Drop stream to stop in-flight requests
3. **Out-of-order completion** - Faster articles don't wait for slower ones
4. **Memory efficient** - Lazy iteration, constant memory overhead

### Key Files Modified

1. `/home/jens/Documents/source/usenet-dl/Cargo.toml` - Add futures dependency
2. `/home/jens/Documents/source/usenet-dl/src/lib.rs` - Main implementation (two download loops)
3. Test files in `tests/` - New integration tests

### Estimated Complexity

- **Low Risk**: Speed limiter, pools, sync primitives all compatible
- **Medium Complexity**: Converting sequential to parallel requires careful variable cloning
- **High Value**: Expected 10-50× speedup depending on connection count

### Performance Expectations

Based on plan's analysis:

| Connections | Current (Sequential) | Expected (Parallel) | Improvement |
|-------------|---------------------|---------------------|-------------|
| 4           | ~5 MB/s             | ~20 MB/s            | 4x          |
| 20          | ~5 MB/s             | ~100 MB/s           | 20x         |
| 50          | ~5 MB/s             | ~200+ MB/s          | 40x+        |

Actual results depend on:
- Provider's per-connection speed limit
- Network bandwidth capacity
- Server latency and article sizes
- Disk I/O performance (temp file writes)

### Testing Strategy

1. **Unit tests** - Mock pools, verify concurrent behavior
2. **Integration tests** - Real NNTP provider (if available), measure speedup
3. **Stress tests** - Large NZB files, verify memory stability
4. **Cancellation tests** - Mid-download pause/cancel verification
5. **Error tests** - Failed article handling

### Rollout Considerations

- **Feature flag?** - Consider making parallel downloads optional via config
- **Gradual rollout** - Test with small connection counts first (4-10)
- **Monitoring** - Track download speeds before/after, watch for errors
- **Rollback plan** - Keep sequential code as fallback if issues arise

### Open Questions

1. **Should we expose concurrency as a separate config option?**
   - Current: Derived from sum of all server connections
   - Alternative: Explicit `max_concurrent_articles` config
   - Recommendation: Start with derived value, add config later if needed

2. **How to handle partial download failures?**
   - Option A: Fail entire download if >X% articles fail
   - Option B: Mark download as "partial" and continue
   - Option C: User-configurable threshold
   - Recommendation: Start with Option A (fail if >50% fail), make configurable later

3. **Should progress reporting be throttled?**
   - Current plan: Periodic task to prevent event spam
   - Alternative: Only emit on X% progress change
   - Recommendation: Emit every 1% progress or every 1 second, whichever is less frequent

### Success Criteria

Implementation is successful when:

1. ✓ Downloads use all configured connections concurrently
2. ✓ Speed increases proportionally to connection count
3. ✓ Progress tracking remains accurate
4. ✓ Error handling works correctly (doesn't fail on single article error)
5. ✓ Cancellation works mid-download
6. ✓ Memory usage stays constant (no leaks)
7. ✓ Speed limiter enforces global limit across parallel downloads
8. ✓ All tests pass (unit, integration, stress)
