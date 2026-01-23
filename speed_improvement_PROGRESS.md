# Progress: speed_improvement

Started: vr 23 jan 2026 20:04:46 CET

## Status

IN_PROGRESS

## Analysis

### What Already Exists (Strong Foundation)

The usenet-dl codebase has excellent infrastructure for high-speed downloads:

1. **Parallel Downloading** - FULLY IMPLEMENTED
   - Location: `src/lib.rs:3256-3361`
   - Uses `futures::stream::buffer_unordered(concurrency)` for concurrent article fetching
   - All configured server connections used simultaneously (typically 50)
   - Out-of-order completion handling
   - Currently achieving 55-80 MB/s sustained throughput

2. **Binary Response API** - IMPLEMENTED
   - Location: `nntp-rs/src/client.rs:2715`, `nntp-rs/src/response.rs:14-43`
   - `fetch_article_binary()` returns `Vec<u8>` directly
   - Avoids UTF-8 validation overhead (~20% performance gain)

3. **Large Buffer Configuration** - IMPLEMENTED
   - Location: `nntp-rs/src/client.rs:240`
   - 256KB BufReader (up from 8KB default)
   - Reduces syscall overhead

4. **TCP_NODELAY** - IMPLEMENTED
   - Location: `nntp-rs/src/client.rs:200`
   - Nagle's algorithm disabled for low latency

5. **Connection Pooling** - IMPLEMENTED
   - Location: `nntp-rs/src/pool.rs`
   - Uses bb8 connection pool library
   - Automatic connection reuse and validation

6. **Progress Reporting** - OPTIMIZED
   - Location: `src/lib.rs:3177-3246`
   - Atomic counters updated per article (minimal overhead)
   - Separate task reports every 500ms (batched reporting)
   - Prevents event spam

7. **Batch Article Insertion** - IMPLEMENTED
   - Location: `src/db.rs:763`
   - `insert_articles_batch()` for NZB parsing
   - Inserts multiple articles in single transaction

8. **yEnc Decoding Implementation** - COMPLETE
   - Location: `nntp-rs/src/yenc.rs` (1422 lines)
   - Full encoder/decoder with CRC32 verification
   - Multi-part assembly support
   - Pure Rust (not SIMD optimized)

**Tip:** Use the DeepWiki MCP tool to explore the SABnzbd codebase in detail:
```
mcp__deepwiki__ask_question(repoName: "sabnzbd/sabnzbd", question: "How does SABnzbd implement article pipelining and prefetching?")
```
This can provide deeper insights into their download optimization strategies. They are a mature fast useenet downloader and you should take inspiration from that.



### What's Missing (Optimization Opportunities)

**HIGH IMPACT:**

1. **Connection Pipelining** - NOT IMPLEMENTED
   - Currently: Each article fetch is request→wait→response
   - Needed: Send multiple ARTICLE commands before reading responses
   - Expected gain: +30-50% throughput
   - Location to implement: `nntp-rs/src/client.rs`

2. **TCP Socket Buffer Tuning** - NOT IMPLEMENTED
   - Currently: OS default socket buffers (typically 128KB-256KB)
   - Needed: Set SO_RCVBUF=4MB, SO_SNDBUF=1MB via socket2 crate
   - Expected gain: +5-10% throughput
   - Location to implement: `nntp-rs/src/client.rs:194-200`
   - Requires: Add `socket2 = "0.5"` to `nntp-rs/Cargo.toml`

**MEDIUM IMPACT:**

3. **Batch Database Status Updates** - PARTIAL
   - Currently: Each article triggers `db.update_article_status()` immediately
   - Location: `src/lib.rs:3327, 3341-3347`
   - Needed: Buffer 100 status updates, write in single transaction
   - Expected gain: +10-20% throughput (reduces SQLite write contention)

4. **Parallel yEnc Decoding** - NOT IMPLEMENTED
   - Currently: Articles written as raw binary to temp files
   - Post-processing stage not yet implemented (src/post_processing.rs:214)
   - Needed: Decode yEnc in parallel with downloads via tokio task pool
   - Expected gain: +10-15% throughput
   - Location: Extend `src/lib.rs:3335-3338` or create `src/decoder.rs`

5. **Article Prefetching** - NOT IMPLEMENTED
   - Currently: Connection may sit idle during article processing
   - Needed: Keep queue of prefetched articles per connection
   - Expected gain: +10-20% throughput
   - Location: `src/lib.rs:3256-3361`

**LOW PRIORITY:**

6. **SIMD yEnc Decoding** - NOT IMPLEMENTED
   - Currently: Byte-by-byte processing in `nntp-rs/src/yenc.rs`
   - Needed: Use SIMD instructions for parallel byte operations
   - Expected gain: +5-10% throughput
   - Significant implementation effort (consider C extension like SABnzbd's sabctools)

### Dependencies Between Tasks

```
Priority 1 (Independent - can run in parallel):
├─ Task 1.1: Implement connection pipelining
└─ Task 1.2: Add TCP socket buffer tuning

Priority 2 (Depends on Priority 1 completing):
├─ Task 2.1: Batch database status updates
├─ Task 2.2: Implement parallel yEnc decoding
└─ Task 2.3: Add article prefetching

Priority 3 (Future optimization):
└─ Task 3.1: SIMD yEnc decoding (deferred)
```

### Key Architecture Decisions

1. **Pipelining Strategy**: Chunk-based (send N commands, read N responses) rather than full async
   - Safer: Maintains request/response correlation
   - Configurable: Pipeline depth parameter (e.g., 10 commands)
   - Compatible: Works with existing connection pool

2. **Database Batching**: Use mpsc channel with periodic flush
   - Non-blocking: Don't slow down download loop
   - Configurable: Flush every 100 updates OR every 1 second
   - Resilient: Flush remaining updates on completion

3. **yEnc Decoding**: Separate tokio task pool with bounded channel
   - Parallel: Decode while downloading continues
   - Bounded: Prevent memory explosion (channel capacity 100)
   - Ordered: Preserve segment ordering for assembly

4. **Socket Tuning**: Apply during connection, not post-connect
   - Use socket2 crate to set buffers before bind
   - Fallback gracefully if OS limits buffer size
   - Log configured sizes for debugging

### Contingencies & Edge Cases

1. **Pipelining Failures**:
   - Some servers may not support pipelining → detect via capability check
   - Add feature flag to disable pipelining if issues occur
   - Fallback to sequential mode on errors

2. **Memory Pressure**:
   - Parallel decoding could consume significant RAM
   - Monitor channel backpressure
   - Add configurable limits for in-flight decoded articles

3. **Database Lock Contention**:
   - Batching may increase transaction time
   - Use WAL mode for concurrent readers (may already be enabled)
   - Add timeout handling for batch writes

4. **Socket Buffer Limits**:
   - OS may restrict maximum buffer size (/proc/sys/net/core/rmem_max)
   - Gracefully handle failures to set requested sizes
   - Log actual vs requested buffer sizes

5. **Testing Challenges**:
   - Pipelining behavior may differ across NNTP servers
   - Need real-world testing with production server
   - Add configurable pipeline depth for tuning

## Task List

### Phase 1: Connection Pipelining (HIGH IMPACT - Expected +30-50%)

- [x] Task 1.1: Add fetch_articles_pipelined() method to NntpClient
  - File: `nntp-rs/src/client.rs`
  - Add public async method that accepts slice of message IDs and pipeline depth
  - Send N commands, then read N responses in order
  - Return Vec<NntpBinaryResponse>
  - Handle errors gracefully (abort pipeline on first error)
  - COMPLETED: Added at line 2757-2866 with full documentation and error handling

- [x] Task 1.2: Add pipelining tests
  - File: `nntp-rs/tests/pipelining_test.rs` (CREATED)
  - Test with 2, 5, 10 article pipeline depths ✓
  - Test error handling (invalid message ID in middle of pipeline) ✓
  - Verify response ordering matches request ordering ✓
  - COMPLETED: Created comprehensive test suite with 10 test cases

- [x] Task 1.3: Integrate pipelining into download loop
  - File: `src/lib.rs:3250-3420`
  - Modify article fetch to batch N articles per connection
  - Use fetch_articles_pipelined() instead of fetch_article_binary()
  - Add PIPELINE_DEPTH constant (start with 10)
  - Handle partial batch at end of article list
  - COMPLETED: Refactored download loop to batch articles in groups of PIPELINE_DEPTH (10)
    - Articles are chunked before being sent to stream processing
    - Each batch uses fetch_articles_pipelined() for reduced latency
    - Error handling updated to track batch sizes for accurate failure counting
    - Speed limiter acquires tokens for entire batch before fetching
    - Progress tracking updated to count individual articles in batches

- [x] Task 1.4: Add pipelining configuration
  - File: `src/config.rs`
  - Add pipeline_depth field to server config
  - Default to 10, allow 0 to disable pipelining
  - Document in README.md
  - COMPLETED: Added pipeline_depth field to ServerConfig with default of 10
    - Updated download loop to use configurable pipeline_depth from server config
    - Added documentation in README.md configuration section
    - Minimum depth enforced (clamped to 1 if 0 is configured)

- [ ] Task 1.5: Run performance test with pipelining
  - Run: TEST_NZB_PATH="./Fallout.S02E06.nzb" NNTP_CONNECTIONS=50 cargo test --release --test e2e_real_nzb test_real_nzb_download -- --ignored --nocapture
  - Record peak and sustained speeds
  - Compare against baseline (55-80 MB/s)
  - Target: 75-120 MB/s sustained

### Phase 2: TCP Socket Buffer Tuning (HIGH IMPACT - Expected +5-10%)

- [ ] Task 2.1: Add socket2 dependency
  - File: `nntp-rs/Cargo.toml`
  - Add: socket2 = "0.5"

- [ ] Task 2.2: Implement socket buffer configuration
  - File: `nntp-rs/src/client.rs:189-200`
  - Use socket2::Socket instead of TcpStream::connect()
  - Set SO_RCVBUF to 4MB (4 * 1024 * 1024)
  - Set SO_SNDBUF to 1MB (1024 * 1024)
  - Convert socket2::Socket to tokio::net::TcpStream
  - Log actual buffer sizes (may differ from requested)
  - Handle errors gracefully (continue with defaults)

- [ ] Task 2.3: Add socket tuning tests
  - File: `nntp-rs/tests/` (add to existing test file)
  - Verify socket buffers are set correctly
  - Test graceful fallback if OS limits buffer size

- [ ] Task 2.4: Run performance test with socket tuning
  - Run: TEST_NZB_PATH="./Fallout.S02E06.nzb" NNTP_CONNECTIONS=50 cargo test --release --test e2e_real_nzb test_real_nzb_download -- --ignored --nocapture
  - Record peak and sustained speeds
  - Compare against pipelining baseline

### Phase 3: Batch Database Status Updates (MEDIUM IMPACT - Expected +10-20%)

- [ ] Task 3.1: Create database update batcher
  - File: `src/db.rs`
  - Add update_articles_status_batch(Vec<(article_id, status)>) method
  - Use single BEGIN/COMMIT transaction for all updates
  - Add batch_update_article_status() that queues updates for batching

- [ ] Task 3.2: Add update batching channel
  - File: `src/lib.rs:3150-3170` (before download loop)
  - Create mpsc channel for status updates: (tx, rx) with capacity 500
  - Spawn background task to consume channel
  - Batch updates: flush every 100 updates OR every 1 second
  - Flush remaining updates when download completes

- [ ] Task 3.3: Replace direct DB calls with batched updates
  - File: `src/lib.rs:3327, 3341-3347`
  - Replace db.update_article_status() with batch_tx.send()
  - Handle channel send errors (log warning, continue)

- [ ] Task 3.4: Add batching tests
  - File: `tests/` (add to existing test file)
  - Verify all updates eventually written to DB
  - Test flush on completion
  - Test flush on timeout

- [ ] Task 3.5: Run performance test with batched updates
  - Run: TEST_NZB_PATH="./Fallout.S02E06.nzb" NNTP_CONNECTIONS=50 cargo test --release --test e2e_real_nzb test_real_nzb_download -- --ignored --nocapture
  - Record peak and sustained speeds
  - Check for SQLite lock contention in logs

### Phase 4: Parallel yEnc Decoding (MEDIUM IMPACT - Expected +10-15%)

- [ ] Task 4.1: Create yEnc decoder task pool
  - File: `src/lib.rs:3150-3170` (before download loop)
  - Create mpsc channel: (decode_tx, decode_rx) with capacity 100
  - Spawn N decoder tasks (N = num_cpus::get())
  - Each task: receive (raw_data, article_id, segment_num) → decode → write to temp file

- [ ] Task 4.2: Integrate decoder into download loop
  - File: `src/lib.rs:3335-3338`
  - Instead of writing raw data, send to decoder channel
  - Decoder writes decoded data to temp files
  - Keep same file naming: article_{segment_number}.dat

- [ ] Task 4.3: Add decoder shutdown handling
  - File: `src/lib.rs:3385-3390` (after download loop)
  - Close decoder channel
  - Wait for all decoder tasks to complete
  - Verify all articles decoded before continuing

- [ ] Task 4.4: Add decoding tests
  - File: `tests/` (add to existing test file)
  - Verify parallel decoding produces correct output
  - Test CRC32 validation
  - Test multi-part assembly

- [ ] Task 4.5: Run performance test with parallel decoding
  - Run: TEST_NZB_PATH="./Fallout.S02E06.nzb" NNTP_CONNECTIONS=50 cargo test --release --test e2e_real_nzb test_real_nzb_download -- --ignored --nocapture
  - Record peak and sustained speeds
  - Monitor CPU usage (should see multi-core utilization)

### Phase 5: Article Prefetching (MEDIUM IMPACT - Expected +10-20%)

- [ ] Task 5.1: Design prefetch architecture
  - Create design document for prefetch queue per connection
  - Decide: prefetch depth (e.g., 3 articles ahead)
  - Consider: memory limits for prefetched data

- [ ] Task 5.2: Implement prefetch queue
  - File: `src/lib.rs:3256-3361`
  - Modify download loop to maintain prefetch buffer
  - Fetch article N+1 while processing article N
  - Use tokio::select! to fetch and process concurrently

- [ ] Task 5.3: Add prefetch configuration
  - File: `src/config.rs`
  - Add prefetch_depth field (default 3)
  - Allow 0 to disable prefetching

- [ ] Task 5.4: Add prefetch tests
  - File: `tests/` (add to existing test file)
  - Verify prefetching doesn't affect correctness
  - Test with various prefetch depths

- [ ] Task 5.5: Run performance test with prefetching
  - Run: TEST_NZB_PATH="./Fallout.S02E06.nzb" NNTP_CONNECTIONS=50 cargo test --release --test e2e_real_nzb test_real_nzb_download -- --ignored --nocapture
  - Record peak and sustained speeds
  - Target: 100+ MB/s sustained

### Phase 6: Integration & Tuning

- [ ] Task 6.1: Run full performance test suite
  - Test with all optimizations enabled
  - Test with various connection counts (25, 50, 100)
  - Test with different pipeline depths (5, 10, 20)
  - Record results in performance matrix

- [ ] Task 6.2: Tune configuration defaults
  - Based on test results, set optimal defaults
  - Document tuning parameters in README.md
  - Add performance troubleshooting guide

- [ ] Task 6.3: Add performance monitoring
  - Add metrics for pipeline depth utilization
  - Add metrics for decoder queue depth
  - Add metrics for database batch sizes
  - Log performance stats at end of download

- [ ] Task 6.4: Update documentation
  - File: `README.md`
  - Document all new configuration options
  - Add performance tuning section
  - Include benchmark results

- [ ] Task 6.5: Update CHANGELOG
  - File: `CHANGELOG.md`
  - Document all performance improvements
  - Include before/after benchmarks
  - Credit optimization techniques and references

### Phase 7: SIMD yEnc Decoding (DEFERRED - Future Optimization)

- [ ] Task 7.1: Research SIMD implementations
  - Investigate existing Rust SIMD crates
  - Review SABnzbd's sabctools C implementation
  - Evaluate portable_simd or other options

- [ ] Task 7.2: Prototype SIMD decoder
  - File: `nntp-rs/src/yenc_simd.rs` (new file)
  - Implement SIMD version of yenc::decode()
  - Benchmark against current implementation

- [ ] Task 7.3: Integrate SIMD decoder
  - Add feature flag for SIMD support
  - Use cfg to select SIMD vs scalar implementation
  - Maintain compatibility with non-x86 platforms

## Notes

### Performance Testing Protocol

All performance tests should follow this protocol:

```bash
# Clean build
cargo clean -p nntp-rs -p usenet-dl

# Run test in release mode
TEST_NZB_PATH="./Fallout.S02E06.The.Other.Player.2160p.AMZN.WEB-DL.DDP5.1.Atmos.DV.HDR10H.265-Kitsune.nzb" \
NNTP_CONNECTIONS=50 \
cargo test --release --test e2e_real_nzb test_real_nzb_download -- --ignored --nocapture
```

Record:
- Peak speed (MB/s)
- Sustained speed (MB/s)
- Total download time
- Any errors or warnings

### Current Performance Baseline

- Hardware capability: 1.4 Gbit/s (175 MB/s)
- Current performance: 55-80 MB/s with 50 connections
- Target: 100+ MB/s sustained

### Risk Assessment

**Low Risk:**
- Tasks 1.x (Pipelining) - Well-understood pattern, easy to test
- Tasks 2.x (Socket tuning) - OS handles gracefully, easy rollback
- Tasks 3.x (DB batching) - Common pattern, good test coverage

**Medium Risk:**
- Tasks 4.x (Parallel decoding) - Memory pressure, needs monitoring
- Tasks 5.x (Prefetching) - Complexity in coordination logic

**High Risk:**
- Tasks 7.x (SIMD) - Platform-specific, significant effort

### Implementation Order Rationale

Phase 1 & 2 first because:
- Independent (can develop/test in parallel)
- Highest impact (combined +35-60% gain)
- Low risk
- Foundation for later optimizations

Phase 3-5 next because:
- Build on stable pipelining foundation
- Each adds incremental value
- Can be tested independently

Phase 6 for integration:
- Ensure all optimizations work together
- Tune for optimal configuration
- Complete documentation

Phase 7 deferred because:
- Lower impact relative to effort
- Requires specialized knowledge
- Can be added later without architectural changes

### Key Files Reference

Core download logic:
- `/home/jens/Documents/source/usenet-dl/src/lib.rs:3256-3361` - Download loop
- `/home/jens/Documents/source/usenet-dl/src/lib.rs:3177-3246` - Progress reporting
- `/home/jens/Documents/source/usenet-dl/src/db.rs:763` - Database operations
- `/home/jens/Documents/source/nntp-rs/src/client.rs:189-240` - Connection setup
- `/home/jens/Documents/source/nntp-rs/src/client.rs:2590-2700` - Binary response reading
- `/home/jens/Documents/source/nntp-rs/src/yenc.rs` - yEnc implementation

Test files:
- `/home/jens/Documents/source/usenet-dl/tests/e2e_real_nzb.rs` - Performance test
- `/home/jens/Documents/source/usenet-dl/tests/parallel_downloads.rs` - Parallel tests

## Completed This Iteration

### Task 1.1: Add fetch_articles_pipelined() method to NntpClient ✓

**Location:** `nntp-rs/src/client.rs:2757-2866`

**Implementation Details:**
- Added public async method `fetch_articles_pipelined(&mut self, ids: &[&str], max_pipeline: usize)`
- Implements two-phase pipelining:
  - Phase 1: Send all commands in chunk without waiting for responses
  - Phase 2: Read all responses in the same order as commands were sent
- Processes articles in chunks based on configurable pipeline depth
- Returns `Vec<NntpBinaryResponse>` matching the order of input message IDs
- Full error handling:
  - Detects NO_SUCH_ARTICLE errors (codes 423, 430)
  - Detects protocol errors (non-2xx responses)
  - Aborts pipeline on first error to prevent mismatched responses
- Comprehensive documentation with performance notes and usage example
- Validates edge cases (empty input, minimum pipeline depth)

**Build Status:** ✓ Compiles cleanly with `cargo check -p nntp-rs`

**Next Steps:**
- Task 1.3: Integrate pipelining into download loop
- Task 1.4: Add pipelining configuration
- Task 1.5: Run performance test with pipelining

### Task 1.2: Add pipelining tests ✓

**Location:** `/home/jens/Documents/source/nntp-rs/tests/pipelining_test.rs`

**Implementation Details:**
- Created comprehensive test suite with 10 test cases for `fetch_articles_pipelined()`
- All tests gated behind `live-tests` feature flag for real NNTP server testing
- Test coverage includes:
  - **Empty input handling**: Validates empty article list returns empty results
  - **Single article fetch**: Tests pipelining with single article
  - **Pipeline depths 2, 5, 10**: Tests multiple pipeline depths
  - **Partial batch handling**: Tests 7 articles with depth 5 (5+2 batches)
  - **Response ordering**: Verifies responses match request order
  - **Error handling**: Tests invalid article ID detection and pipeline abort
  - **Minimum depth validation**: Tests depth 0 clamped to 1
  - **Performance comparison**: Compares sequential vs pipelined performance
- All tests follow existing nntp-rs test patterns (use `get_test_config()`, handle empty groups)
- Tests can be run with: `cargo test --test pipelining_test --features live-tests`

**Build Status:** ✓ Compiles cleanly with and without `live-tests` feature

**Test List:**
1. `test_pipelining_empty_input`
2. `test_pipelining_single_article`
3. `test_pipelining_depth_2`
4. `test_pipelining_depth_5`
5. `test_pipelining_depth_10`
6. `test_pipelining_partial_batch`
7. `test_pipelining_response_ordering`
8. `test_pipelining_invalid_article_id`
9. `test_pipelining_minimum_depth`
10. `test_pipelining_vs_sequential_performance`

### Task 1.3: Integrate pipelining into download loop ✓

**Location:** `src/lib.rs:3250-3420`

**Implementation Details:**
- Added `PIPELINE_DEPTH` constant set to 10 articles per batch
- Refactored download loop from single-article processing to batch processing:
  - Pre-chunk articles into groups of PIPELINE_DEPTH before streaming
  - Each batch processed as a unit by buffer_unordered
  - Convert Articles to message IDs and call `fetch_articles_pipelined()`
- Updated error handling:
  - Errors now return `(String, usize)` tuple with error message and batch size
  - Failed batch marks all articles in batch as FAILED in database
  - Result counting properly accumulates individual article counts from batches
- Speed limiter updated to acquire tokens for entire batch before fetching
- Progress counters updated to track individual articles within batches
- Architecture preserves existing concurrency model:
  - Still uses `buffer_unordered(concurrency)` for parallel batch processing
  - Connection pool manages NNTP connections as before
  - Each connection now fetches multiple articles per round-trip

**Key Changes:**
1. Line 3264: Added PIPELINE_DEPTH constant and documentation
2. Lines 3273-3279: Chunk articles before creating stream
3. Lines 3319-3340: Prepare message IDs and call pipelined fetch
4. Lines 3350-3358: Error path marks all batch articles as failed
5. Lines 3363-3393: Process batch responses and write article files
6. Lines 3410-3428: Update result counting to handle batches

**Build Status:** ✓ Compiles cleanly with `cargo check -p usenet-dl`

**Next Steps:**
- Task 1.5: Run performance test to measure improvement

### Task 1.4: Add pipelining configuration ✓

**Location:** `src/config.rs`, `src/lib.rs`, `README.md`

**Implementation Details:**
- Added `pipeline_depth` field to `ServerConfig` struct (line 189-193 in config.rs)
- Field has comprehensive documentation explaining pipelining benefits
- Default value set to 10 via `default_pipeline_depth()` function
- Updated download loop to use configurable value instead of hardcoded constant
  - Reads pipeline_depth from first server config (line 3268-3271 in lib.rs)
  - Enforces minimum depth of 1 (disables pipelining if set to 0 by clamping to 1)
  - Falls back to 10 if no servers configured
- Variable copied into async closure for use in pipelined fetch call
- Updated README.md:
  - Added pipeline_depth to Default Settings table
  - Added pipeline_depth to ServerConfig example with explanatory comment

**Key Changes:**
1. `src/config.rs:189-193`: Added pipeline_depth field with documentation
2. `src/config.rs:790-792`: Added default_pipeline_depth() function
3. `src/lib.rs:3268-3271`: Read pipeline_depth from server config with validation
4. `src/lib.rs:3294`: Use pipeline_depth for chunking articles
5. `src/lib.rs:3307`: Copy pipeline_depth into async closure
6. `src/lib.rs:3358`: Use pipeline_depth in fetch_articles_pipelined call
7. `README.md:296`: Added to Default Settings table
8. `README.md:319`: Added to ServerConfig example

**Build Status:** ✓ Compiles cleanly with `cargo build -p usenet-dl`

**Configuration Example:**
```rust
ServerConfig {
    host: "news.example.com".to_string(),
    port: 563,
    tls: true,
    username: Some("user".to_string()),
    password: Some("pass".to_string()),
    connections: 10,
    priority: 0,
    pipeline_depth: 10,  // Can be tuned: 1 (disabled) to 20 (aggressive)
}
```
