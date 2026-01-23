# Parallel Download Optimization Plan

## Current Problem

The download loop in `src/lib.rs` (lines 3153 and 3694) processes articles **sequentially**:

```rust
// Download each article
for article in pending_articles {
    // ... fetch one article at a time
}
```

This means only 1 connection is used regardless of connection pool size. With 50 connections configured, we're still getting ~5MB/s instead of potentially 50x that.

---

## How SABnzbd Does It

SABnzbd uses a **multi-threaded worker pool architecture**:

```
┌─────────────────────────────────────────────────────────────┐
│                      Downloader (main thread)               │
│  - Manages servers and connection pools                     │
│  - Distributes articles to idle workers                     │
│  - Coordinates completion                                   │
└─────────────────────────────────────────────────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          ▼                   ▼                   ▼
   ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
   │ NewsWrapper │     │ NewsWrapper │     │ NewsWrapper │
   │  Thread 1   │     │  Thread 2   │     │  Thread N   │
   │             │     │             │     │             │
   │ - 1 NNTP    │     │ - 1 NNTP    │     │ - 1 NNTP    │
   │   connection│     │   connection│     │   connection│
   └─────────────┘     └─────────────┘     └─────────────┘
          │                   │                   │
          └───────────────────┼───────────────────┘
                              ▼
                    ┌─────────────────┐
                    │  ArticleCache   │
                    │  (in-memory)    │
                    └─────────────────┘
                              │
                              ▼
                    ┌─────────────────┐
                    │   Assembler     │
                    └─────────────────┘
```

Key points:
- Each NewsWrapper thread holds one NNTP connection
- Articles distributed to idle workers from a queue
- ArticleCache stores downloaded articles before assembly
- Assembler writes completed files to disk

---

## Implementation: Buffered Stream

Use `futures::stream::iter().buffer_unordered()` for concurrent downloads.

### Before (Sequential)

```rust
for article in pending_articles {
    let mut conn = pool.get().await?;
    let response = conn.fetch_article(&message_id).await?;
    // write to temp file...
}
```

### After (Parallel)

```rust
use futures::stream::{self, StreamExt};

let concurrency = config.servers.iter()
    .map(|s| s.connections)
    .sum::<usize>();

let results: Vec<_> = stream::iter(pending_articles)
    .map(|article| {
        let pool = pool.clone();
        async move {
            let mut conn = pool.get().await?;
            let message_id = if article.message_id.starts_with('<') {
                article.message_id.clone()
            } else {
                format!("<{}>", article.message_id)
            };
            let response = conn.fetch_article(&message_id).await?;
            // write to temp file...
            Ok::<_, Error>((article.segment_number, article.size_bytes))
        }
    })
    .buffer_unordered(concurrency)
    .collect()
    .await;

// Process results, update progress
for result in results {
    match result {
        Ok((segment, size)) => { /* update progress */ }
        Err(e) => { /* handle failure */ }
    }
}
```

### How buffer_unordered Works

```
pending_articles: [A1, A2, A3, A4, A5, A6, A7, A8, ...]
                   │
                   ▼
            stream::iter()
                   │
                   ▼
         .map(|a| async { fetch(a) })
                   │
                   ▼
        .buffer_unordered(4)  ◄── Only 4 concurrent fetches
                   │
         ┌────┬────┼────┬────┐
         ▼    ▼    ▼    ▼    │
        [A1] [A2] [A3] [A4]  │  (in-flight)
         │    │    │    │    │
         ▼    ▼    ▼    ▼    │
        Done  │   Done  │    │
         │    │    │    │    │
         ▼    │    ▼    │    │
        [A5] [A2] [A6] [A4]  │  (A1,A3 done, A5,A6 started)
              │         │    │
              ▼         ▼    │
             ...       ...   │
                             ▼
                      .collect()
                             │
                             ▼
                    Vec<Result<...>>
```

- Stream lazily pulls articles as slots free up
- At most `concurrency` futures active at once
- Results collected in completion order (not input order)
- Natural backpressure - won't overwhelm the connection pool

---

## Implementation Steps

### 1. Locate the download loops

Two places need changes:
- `src/lib.rs:3153-3350` - queue processor download loop
- `src/lib.rs:3694-3800` - direct `download_nzb` loop

### 2. Calculate concurrency

```rust
let concurrency: usize = self.nntp_pools.iter()
    .map(|pool| pool.max_size())
    .sum();
```

Or from config:
```rust
let concurrency: usize = self.config.servers.iter()
    .map(|s| s.connections)
    .sum();
```

### 3. Replace sequential loop with stream

See "After (Parallel)" code above.

### 4. Progress tracking

Articles complete out of order. Options:

**A) Atomic counter (simple)**
```rust
let downloaded_bytes = Arc::new(AtomicU64::new(0));
let downloaded_count = Arc::new(AtomicU64::new(0));

// Inside each future:
downloaded_bytes.fetch_add(article.size_bytes as u64, Ordering::Relaxed);
downloaded_count.fetch_add(1, Ordering::Relaxed);
```

**B) Channel for progress events**
```rust
let (progress_tx, mut progress_rx) = mpsc::channel(100);

// Spawn progress reporter
tokio::spawn(async move {
    while let Some((bytes, count)) = progress_rx.recv().await {
        // emit Event::Downloading
    }
});
```

### 5. Error handling

Don't abort entire download on single article failure:

```rust
.map(|article| {
    async move {
        match fetch_article(&pool, &article).await {
            Ok(data) => Ok((article, data)),
            Err(e) => {
                // Log error, mark article as failed
                tracing::warn!(article_id = article.id, "Article fetch failed: {}", e);
                Err((article, e))
            }
        }
    }
})
```

After collection, retry failed articles or mark download as partial failure.

### 6. Cancellation support

Check cancel token inside each future:

```rust
.map(|article| {
    let cancel_token = cancel_token.clone();
    async move {
        if cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }
        // ... fetch article
    }
})
```

---

## Memory Usage

For reference, with a large NZB (7,873 segments):

| What | Size |
|------|------|
| Article metadata (already loaded) | ~1 MB |
| In-flight futures (50 connections) | ~50 KB |
| Temp files on disk | ~5.8 GB |
| **Total RAM overhead** | **~1 MB** |

The article content goes to temp files, not memory. Memory usage is identical to current sequential approach.

---

## Expected Performance

| Connections | Current | Expected |
|-------------|---------|----------|
| 4 | ~5 MB/s | ~20 MB/s |
| 20 | ~5 MB/s | ~100 MB/s |
| 50 | ~5 MB/s | ~200+ MB/s |

Actual speed depends on:
- Provider's per-connection speed limit
- Total network bandwidth
- Server latency
- Article size (larger = more efficient)

---

## Testing

1. **Unit test**: Mock pool, verify concurrent fetches
2. **Integration test**: Real provider, measure actual speedup
3. **Stress test**: Large NZB, verify no memory leaks
4. **Cancel test**: Pause mid-download, verify cleanup
