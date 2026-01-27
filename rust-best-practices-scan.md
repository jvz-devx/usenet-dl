# Rust Best Practices Scan Report

## Overview

The codebase is well-structured with ~80+ Rust source files across 10 modules. Overall code quality is **good**, with strong error handling and proper async patterns. Below are the findings grouped by category and severity.

---

## 1. Error Handling — Grade: A

**Strengths:**
- Uses `thiserror` throughout (correct for a library crate); `anyhow` is not a dependency
- Comprehensive error hierarchy in `src/error.rs` with domain-specific types (`DatabaseError`, `DownloadError`, `PostProcessError`)
- Proper HTTP status code mapping via `ToHttpStatus` trait
- Zero `.unwrap()` calls in production code (all 110+ are in test code)
- Zero `panic!`, `todo!`, `unimplemented!` in production code
- Intentional `let _ = ...` pattern used explicitly where Results are discarded (shutdown paths, cleanup)

**No issues found.**

---

## 2. Async Patterns — Grade: A-

**Strengths:**
- Proper use of `tokio::fs` for async file operations (`post_processing/verify.rs`, `post_processing/cleanup.rs`)
- Correct `spawn_blocking()` wrapping for archive extraction (`extraction/shared.rs`)
- No `std::thread::sleep` in async code
- No `block_on` inside async code
- No `.await` inside `Mutex` lock guard scopes

**1 issue found:**

| Severity | File | Line | Issue |
|----------|------|------|-------|
| Medium | `src/extraction/password_list.rs` | 42 | `std::fs::read_to_string()` (blocking I/O) called from sync fn that is invoked in async context (`post_processing/mod.rs:224`). Should use `tokio::fs` or wrap in `spawn_blocking`. |

---

## 3. Unnecessary Cloning — Grade: B

**High impact:**

| File | Lines | Issue |
|------|-------|-------|
| `src/downloader/config_ops.rs` | 125, 136 | `get_categories()` and `get_schedule_rules()` clone entire `HashMap`/`Vec` on every call. Callers that only need to read pay an allocation cost. |
| `src/rss_scheduler.rs` | 102 | Entire `Vec<RssFeedConfig>` cloned every RSS check cycle inside a loop. Should iterate by reference. |
| `src/downloader/tasks.rs` | 112–116 | Arc clones inside per-article inner loop (potentially thousands of iterations). Use `Arc::clone()` for clarity and consider hoisting. |
| `src/downloader/webhooks.rs` | 195, 206, 222 | `HashMap` of env vars cloned per-script in loops. Build once and pass by reference. |

**Medium impact:**

| File | Lines | Issue |
|------|-------|-------|
| `src/api/routes/downloads.rs` | 248, 442 | `serde_json::Value::clone()` before `from_value()`. `from_value` consumes, so this is necessary, but could restructure to avoid the clone. |
| `src/downloader/queue_processor.rs` | 30–38 | Multiple Arc clones use `.clone()` syntax inconsistently — some later use `Arc::clone()`. Should be consistent. |

---

## 4. Type Safety — Grade: B-

**High priority:**

| File | Lines | Issue |
|------|-------|-------|
| `src/types.rs` | 12 | `pub type DownloadId = i64` is a type alias, not a newtype. Provides zero compile-time safety — a `RuleId` can be passed where `DownloadId` is expected. Should be `pub struct DownloadId(i64)`. |
| `src/scheduler/mod.rs` | 43 | Same issue: `pub type RuleId = i64`. |
| `src/db/downloads.rs` | 130, 174 | `update_status(id, status: i32)` and `update_priority(id, priority: i32)` accept raw `i32` instead of typed enums. |

**Medium priority:**

| File | Lines | Issue |
|------|-------|-------|
| `src/downloader/mod.rs` | 88 | `pub db: Arc<Database>` on `UsenetDownloader` exposes the database directly. Should be `pub(crate)` with test-only accessors. |
| Multiple files | — | Missing `#[must_use]` on `add_nzb_content()`, `add_nzb_url()`, `insert_download()`, `is_obfuscated()`, `get_unique_path()`, `is_sample()`, `get_available_space()`. |

---

## 5. Iterator & Collection Patterns — Grade: B

| Severity | File | Lines | Issue |
|----------|------|-------|-------|
| Medium | `src/downloader/server.rs` | 101–108 | `test_all_servers()` manually pushes to `Vec::new()`. Should use `Vec::with_capacity(servers.len())`. Cannot easily use `.collect()` due to sequential `.await`. |
| Medium | `src/downloader/nzb.rs` | 194 | `articles.chunks(199)` — magic number. Define `const SQLITE_BATCH_SIZE: usize = 199;`. |
| Medium | `src/deobfuscation.rs` | 56–100 | Magic threshold constants (`24`, `0.31`, `0.38`, `0.28`) in `is_high_entropy()`. Should be named constants. |
| Medium | `src/downloader/nzb.rs` | 260, 273 | Timeout of `30` seconds hardcoded in two places. Define `const NZB_FETCH_TIMEOUT_SECS: u64 = 30;`. |
| Low | `src/extraction/zip.rs` | 94–130 | 5+ levels of nesting with duplicated error-mapping logic in both `by_index` and `by_index_decrypt` branches. Extract to helper. |
| Low | `src/downloader/control.rs` | 436–550 | Deep nesting in spawned async block. Extract `handle_reextract_success()` / `handle_reextract_error()` helpers. |

---

## 6. Formatting & Linting

`cargo fmt` and `cargo clippy` are **not installed** in the current toolchain. These should be added:

```bash
rustup component add rustfmt clippy
```

---

## Summary

| Category | Grade | Critical Issues | Actionable Items |
|----------|-------|-----------------|------------------|
| Error Handling | A | 0 | 0 |
| Async Patterns | A- | 0 | 1 (blocking I/O in async path) |
| Unnecessary Cloning | B | 0 | 4 high, 2 medium |
| Type Safety | B- | 2 (ID aliases) | 5 total |
| Iterators & Collections | B | 0 | 6 (magic numbers, nesting) |
| Tooling | — | — | Install `rustfmt` + `clippy` |

### Top 5 Recommended Actions

1. **Convert `DownloadId` and `RuleId`** from type aliases to newtype structs for compile-time safety
2. **Fix blocking I/O** in `extraction/password_list.rs` — wrap in `spawn_blocking` or make async
3. **Reduce cloning** in `config_ops.rs` (HashMap/Vec cloned on every access) and `rss_scheduler.rs` (Vec cloned every cycle)
4. **Replace magic numbers** with named constants (`SQLITE_BATCH_SIZE`, `NZB_FETCH_TIMEOUT_SECS`, entropy thresholds)
5. **Add `#[must_use]`** to functions returning IDs and boolean checks (`add_nzb_content`, `is_obfuscated`, etc.)
