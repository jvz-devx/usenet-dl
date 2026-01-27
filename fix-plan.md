# Must-Fix Plan — usenet-dl

Score: **86/100 (B)**. These fixes target the remaining issues blocking A-grade (90+).

---

## 1. Remove dead code in `src/downloader/queue.rs`

**Lines 79-101** — Three `pub(crate)` methods are never called anywhere:
- `get_next_download`
- `peek_next_download`
- `queue_size`

**Action:** Delete all three methods. They're superseded by the queue processor.

---

## 2. Break up `spawn_download_task` (281 lines, nesting depth 11)

**File:** `src/downloader/tasks.rs:27`

This is a single function containing the entire download lifecycle inside a `tokio::spawn` closure. Extract into these focused helpers:

| New function | Lines to extract | Responsibility |
|---|---|---|
| `fetch_article(pool, article, temp_dir, db)` | 118-198 | Single article fetch, write, status update |
| `tally_results(results) -> (successes, failures, first_error)` | 205-219 | Count successes/failures from stream results |
| `handle_download_failure(db, event_tx, id, failures, successes, total, error)` | 230-257 | Failure threshold check, status update, event emit |
| `emit_final_progress(db, event_tx, id, bytes, articles, total_size, total_articles, start)` | 259-283 | Final progress calculation and event |

After extraction, `spawn_download_task` becomes ~60 lines of orchestration that reads top-to-bottom.

---

## 3. Break up remaining >100-line functions

Six functions exceed 100 lines. Priority order:

### 3a. `reextract` — 164 lines at `src/downloader/control.rs:391`
Extract the archive-discovery logic and the re-extraction loop into separate helpers.

### 3b. `add_nzb_content` — 161 lines at `src/downloader/nzb.rs:62`
Split NZB XML parsing, article insertion, and download record creation into separate steps.

### 3c. `run_extract_stage` — 147 lines at `src/post_processing/mod.rs:169`
Extract the per-archive extraction loop and password-retry logic.

### 3d. `try_extract` (zip) — 139 lines at `src/extraction/zip.rs:56`
Extract password attempt loop and entry extraction into helpers.

### 3e. `try_extract` (rar) — 116 lines at `src/extraction/rar.rs:65`
Same pattern as zip — extract password loop.

### 3f. `download_articles` — 116 lines at `src/downloader/download_task.rs:181`
Extract the per-batch processing and progress-reporting logic.

---

## 4. Fix too-many-arguments functions (7 functions)

Clippy flags 7 functions exceeding the 7-parameter limit (8-10 params). For each, create a params struct:

```rust
// Before
fn do_thing(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H) { ... }

// After
struct DoThingParams { a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H }
fn do_thing(params: DoThingParams) { ... }
```

---

## 5. Add missing doc comments (111 clippy warnings)

Mostly struct fields in two files:
- `src/db/mod.rs` — `Download` struct fields (lines 33-42, ~10 fields)
- `src/error.rs` — Error variant struct fields (lines 26, 89, 137, 141, 145, 150-151)

Add `///` doc comments to each undocumented field.

---

## 6. Add module doc to `src/rss_manager/mod.rs`

Add a `//!` doc comment block at the top of the file describing the RSS feed management module.

---

## Execution Order

1. **Item 1** (dead code) — trivial, do first
2. **Item 6** (module doc) — trivial
3. **Item 5** (field docs) — mechanical, low risk
4. **Item 4** (params structs) — moderate, changes function signatures
5. **Item 2** (spawn_download_task) — largest refactor, highest impact
6. **Item 3** (remaining long functions) — do incrementally

After all items: run `cargo clippy --lib -- -D warnings` to confirm zero warnings.
