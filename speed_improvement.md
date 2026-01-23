# Speed Improvement Plan for usenet-dl

## Current Performance
- **Hardware capability:** ~2.5 Gbit/s (~310 MB/s) per speedtest.net
- **Current performance:** 55-80 MB/s with 50 connections (release mode)
- **Target:** 150+ MB/s

## Benchmark Results

| Build Mode | Peak Speed | End Speed | Total Time |
|------------|------------|-----------|------------|
| Debug (original) | ~60 MB/s | ~35 MB/s | - |
| Debug (with optimizations) | ~75 MB/s | ~37 MB/s | 369s |
| **Release (with optimizations)** | **~80 MB/s** | **~45 MB/s** | **363s** |

Test file: ~700MB NZB with 50 connections

## What Has Already Been Tried

### 1. Increased BufReader Buffer (DONE - Minor improvement)
- **File:** `nntp-rs/src/client.rs:240`
- **Change:** Increased from 8KB to 256KB
- **Result:** Helped slightly with syscall reduction

### 2. Binary Response API (DONE - ~20% improvement at peak)
- **Files:** `nntp-rs/src/client.rs`, `nntp-rs/src/response.rs`
- **Change:** Added `NntpBinaryResponse` that returns `Vec<u8>` instead of `Vec<String>`
- **Result:** Peak improved from ~60 MB/s to ~75 MB/s (debug), ~80 MB/s (release)

### 3. Direct Byte Collection (DONE)
- **File:** `nntp-rs/src/client.rs` - `read_multiline_response_binary_with_timeout()`
- **Change:** Uses `read_until` to collect bytes directly without string conversion
- **Result:** Avoids UTF-8 validation overhead

### 4. Release Mode Testing (DONE - ~7% improvement)
- **Change:** Run with `cargo test --release` instead of debug mode
- **Result:** Peak improved from ~75 MB/s to ~80 MB/s, sustained from ~35 MB/s to ~55-60 MB/s

---

## Remaining Bottlenecks to Address

### Priority 1: Connection Pipelining (HIGH IMPACT)

**Problem:** Each article fetch is sequential: send command → wait for response → process. With 50 connections doing this independently, we're limited by round-trip latency.

**Solution:** Implement NNTP command pipelining - send multiple ARTICLE commands before waiting for responses.

**Files to modify:**
- `nntp-rs/src/client.rs`

**Implementation:**
```rust
/// Fetch multiple articles with pipelining
pub async fn fetch_articles_pipelined(
    &mut self,
    ids: &[&str],
    max_pipeline: usize,  // e.g., 10 commands at once
) -> Result<Vec<NntpBinaryResponse>> {
    let mut results = Vec::with_capacity(ids.len());

    for chunk in ids.chunks(max_pipeline) {
        // Send all commands first
        for id in chunk {
            let cmd = commands::article(id);
            self.send_command(&cmd).await?;
        }

        // Then read all responses
        for id in chunk {
            let response = self.read_multiline_response_binary().await?;
            results.push(response);
        }
    }

    Ok(results)
}
```

**Test command:**
```bash
TEST_NZB_PATH="./Fallout.S02E06.The.Other.Player.2160p.AMZN.WEB-DL.DDP5.1.Atmos.DV.HDR10H.265-Kitsune.nzb" NNTP_CONNECTIONS=50 cargo test --test e2e_real_nzb test_real_nzb_download -- --ignored --nocapture
```

---

### Priority 2: Reduce Per-Article Overhead (HIGH IMPACT)

**Problem:** Each article incurs overhead:
- Database status update (SQLite write)
- File write (individual file per article)
- Atomic counter updates

**Solution:** Batch operations.

**Files to modify:**
- `usenet-dl/src/lib.rs` (around line 3322-3360)

**Implementation:**
1. Batch database updates - update status for 100 articles at once instead of 1
2. Use memory-mapped files or write to a single large file with offsets
3. Reduce progress reporting frequency

**Changes needed in lib.rs:**
```rust
// Instead of writing each article to a separate file:
// let article_file = download_temp_dir.join(format!("article_{}.dat", article.segment_number));
// tokio::fs::write(&article_file, &response.data).await?;

// Write to a single pre-allocated file at specific offsets:
// This requires knowing article sizes upfront from NZB
```

---

### Priority 3: Parallel yEnc Decoding (MEDIUM IMPACT)

**Problem:** yEnc decoding happens after download, serializing CPU work.

**Solution:** Decode yEnc in parallel with downloading using a separate task pool.

**Files to modify:**
- `usenet-dl/src/lib.rs`
- Possibly create `usenet-dl/src/decoder.rs`

**Implementation:**
```rust
// Create a decoder channel
let (decode_tx, decode_rx) = tokio::sync::mpsc::channel(100);

// Spawn decoder workers
for _ in 0..num_cpus::get() {
    let rx = decode_rx.clone();
    tokio::spawn(async move {
        while let Some((data, segment_num)) = rx.recv().await {
            // Decode yEnc in parallel
            let decoded = nntp_rs::yenc_decode(&data);
            // Write to assembler
        }
    });
}

// In download loop, send to decoder instead of writing directly
decode_tx.send((response.data, article.segment_number)).await?;
```

---

### Priority 4: TCP Socket Tuning (MEDIUM IMPACT)

**Problem:** Default TCP buffer sizes may be too small for high-bandwidth connections.

**Solution:** Increase TCP buffer sizes before connecting.

**Files to modify:**
- `nntp-rs/src/client.rs` (in `connect()` method, around line 194)

**Implementation:**
```rust
// After TcpStream::connect, before TLS:
use socket2::{Socket, Domain, Type};

let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;
socket.set_recv_buffer_size(4 * 1024 * 1024)?;  // 4MB receive buffer
socket.set_send_buffer_size(1024 * 1024)?;       // 1MB send buffer
socket.set_nodelay(true)?;
socket.connect(&addr.parse::<std::net::SocketAddr>()?.into())?;
let tcp_stream = TcpStream::from_std(socket.into())?;
```

Add to `nntp-rs/Cargo.toml`:
```toml
socket2 = "0.5"
```

---

### Priority 5: Article Prefetching (MEDIUM IMPACT)

**Problem:** Connection sits idle while processing downloaded article.

**Solution:** Prefetch next articles while current ones are being processed.

**Files to modify:**
- `usenet-dl/src/lib.rs`

**Implementation:**
Keep a queue of prefetched articles per connection. While the main loop processes article N, the connection is already fetching article N+1.

---

### Priority 6: SIMD yEnc Decoding (LOW IMPACT but significant)

**Problem:** yEnc decoding is CPU-bound and processes byte-by-byte.

**Solution:** Use SIMD instructions for parallel byte processing.

**Reference:** SABnzbd's `sabctools` uses C with SIMD for this.

**Files to modify:**
- `nntp-rs/src/yenc.rs`

This would require significant work - consider using an existing optimized crate or writing assembly.

---

## Testing Instructions

After each change, run the performance test:

```bash
# Clean build to ensure changes are compiled
cargo clean -p nntp-rs -p usenet-dl

# Run with 50 connections
TEST_NZB_PATH="./Fallout.S02E06.The.Other.Player.2160p.AMZN.WEB-DL.DDP5.1.Atmos.DV.HDR10H.265-Kitsune.nzb" \
NNTP_CONNECTIONS=50 \
cargo test --release --test e2e_real_nzb test_real_nzb_download -- --ignored --nocapture
```

**Note:** Use `--release` for accurate performance testing!

### Expected Results by Priority

| Change | Expected Improvement |
|--------|---------------------|
| Pipelining | +30-50% |
| Batch operations | +10-20% |
| Parallel yEnc | +10-15% |
| TCP tuning | +5-10% |
| Prefetching | +10-20% |
| SIMD yEnc | +5-10% |

---

## SABnzbd Reference Architecture

SABnzbd achieves high speeds through:

1. **Multi-threaded downloaders** with idle/busy connection pools
2. **Pre-allocated buffers** using `bytearray` with `memoryview`
3. **C-extension (`sabctools`)** for:
   - Optimized SSL recv (`ssl_recv_into`)
   - SIMD yEnc decoding
4. **Article prefetching** (`_ARTICLE_PREFETCH` constant)
5. **Assembler queue monitoring** to prevent overwhelming disk I/O
6. **Adaptive connection management** (backs off on errors)

Source: https://github.com/sabnzbd/sabnzbd

**Tip:** Use the DeepWiki MCP tool to explore the SABnzbd codebase in detail:
```
mcp__deepwiki__ask_question(repoName: "sabnzbd/sabnzbd", question: "How does SABnzbd implement article pipelining and prefetching?")
```
This can provide deeper insights into their download optimization strategies.

---

## Quick Wins to Try First

1. **Run in release mode** - Debug builds are 10x slower:
   ```bash
   cargo test --release --test e2e_real_nzb ...
   ```

2. **Increase connection count** - Try 100 connections if provider allows:
   ```bash
   NNTP_CONNECTIONS=100 cargo test ...
   ```

3. **Check provider limits** - Some providers throttle per-connection or total bandwidth

4. **Verify network isn't the bottleneck**:
   ```bash
   # Test raw download speed
   curl -o /dev/null https://speed.hetzner.de/1GB.bin
   ```
