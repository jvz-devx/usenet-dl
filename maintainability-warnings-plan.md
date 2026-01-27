# Maintainability Warnings — Remediation Plan

Generated: 2026-01-27 | Score: 80/100 (Grade B)

---

## W1: Split `src/extraction.rs` (1,937 lines)

**Problem:** Largest file in the codebase. Handles RAR, ZIP, and 7z extraction in a single module.

**Plan:**
1. Create `src/extraction/` directory with `mod.rs`
2. Move RAR extraction logic → `src/extraction/rar.rs`
3. Move ZIP extraction logic → `src/extraction/zip.rs`
4. Move 7z extraction logic → `src/extraction/sevenz.rs`
5. Keep shared types, traits, and the public `extract()` entry point in `mod.rs`
6. Move inline `#[cfg(test)]` block → `src/extraction/tests/` (following existing pattern from `db/`, `api/`, `downloader/`)

**Acceptance criteria:** Each file under 500 lines. All existing tests pass. Public API unchanged.

---

## W2: Extract test modules from `src/scheduler.rs` (1,796 lines)

**Problem:** ~1,500 of 1,796 lines are inline tests. Production code is ~300 lines — reasonable on its own.

**Plan:**
1. Create `src/scheduler/` directory
2. Move production code → `src/scheduler/mod.rs`
3. Move `#[cfg(test)] mod tests` → `src/scheduler/tests/mod.rs`
4. If the test module is still >800 lines, split by test category (e.g., `rule_matching.rs`, `priority.rs`, `boundary.rs`)

**Acceptance criteria:** Production module under 400 lines. All 268 test `unwrap()` calls remain in test code only. All tests pass.

---

## W3: Extract test modules from `src/rss_manager.rs` (1,794 lines)

**Problem:** ~600 lines of inline test code. Production logic (~1,200 lines) is also large.

**Plan:**
1. Create `src/rss_manager/` directory
2. Move production code → `src/rss_manager/mod.rs`
3. Move `#[cfg(test)]` block → `src/rss_manager/tests/mod.rs`
4. Evaluate splitting production code further if `mod.rs` remains >800 lines (e.g., separate `filters.rs` for `matches_filters` and related logic)

**Acceptance criteria:** Test code separated. Production module under 1,000 lines. All tests pass.

---

## W4: Split `src/post_processing.rs` (1,703 lines)

**Problem:** Contains the full post-processing pipeline (verify, repair, extract, cleanup) plus inline tests.

**Plan:**
1. Create `src/post_processing/` directory with `mod.rs`
2. Move `run_verify_stage` → `src/post_processing/verify.rs`
3. Move `run_repair_stage` → `src/post_processing/repair.rs`
4. Move `cleanup` → `src/post_processing/cleanup.rs`
5. Keep the pipeline orchestration and shared types in `mod.rs`
6. Move `#[cfg(test)]` block → `src/post_processing/tests/mod.rs`

**Acceptance criteria:** Each file under 500 lines. Pipeline stages independently testable. All tests pass.

---

## W5: Reduce `src/config.rs` (1,202 lines)

**Problem:** Single file with large `Config` struct (16 fields) and all validation/defaults logic.

**Plan:**
1. Group related fields into sub-config structs:
   - `ServerConfig` — NNTP server connection settings
   - `PathConfig` — download directory, temp directory, watch folder paths
   - `SchedulerConfig` — scheduling rules
   - `PostProcessConfig` — extraction, repair, cleanup settings
2. Keep `Config` as the top-level struct composing these sub-configs
3. Move validation logic into each sub-config's `impl` block
4. Ensure `serde` deserialization flattens correctly (use `#[serde(flatten)]` if needed)

**Acceptance criteria:** `Config` struct reduced to <10 direct fields. Total config module stays under 800 lines. Deserialization backward-compatible.

---

## W6: Split `src/downloader/tasks.rs` (1,098 lines)

**Problem:** Large file handling multiple task types.

**Plan:**
1. Identify distinct task types in the file
2. Extract each task type into its own file under `src/downloader/` (e.g., `download_task.rs`, `decode_task.rs`)
3. Keep shared task infrastructure in `tasks.rs`

**Acceptance criteria:** Each file under 500 lines. All tests pass.

---

## W7: Break up long functions

**Problem:** Several functions exceed 50–100 lines, increasing cognitive complexity.

| Function | File | Lines | Action |
|----------|------|-------|--------|
| `migrate_v1` | `src/db/migrations.rs:99` | 213 | Extract SQL statements into constants or helper fns |
| `reextract` | `src/downloader/control.rs:391` | 163 | Extract validation, cleanup, and re-queue steps |
| `run` | `src/rss_scheduler.rs:87` | 109 | Extract feed-processing loop body into helper |
| `new` | `src/downloader/mod.rs:115` | 104 | Consider builder pattern (see W8) |
| `run_repair_stage` | `src/post_processing.rs:295` | 104 | Addressed by W4 split |
| `create_router` | `src/api/mod.rs:89` | 96 | Fine for a router definition — low priority |

**Acceptance criteria:** No production function exceeds 80 lines (except migration functions and router builders, which are inherently declarative).

---

## W8: Decompose `UsenetDownloader` struct (19 fields)

**Problem:** `src/downloader/mod.rs` struct `UsenetDownloader` has 19 fields — god object risk. Constructor is 104 lines.

**Plan:**
1. Group related fields into sub-structs:
   - Connection/server state
   - Queue/download state
   - Configuration references
   - Notification/webhook channels
2. Introduce a builder pattern for construction (replacing the 104-line `new()`)
3. Keep `UsenetDownloader` as the facade composing these components

**Acceptance criteria:** `UsenetDownloader` has <12 direct fields. Builder provides clear construction API. All tests pass.

---

## W9: `Download` struct has 19 fields (`src/db/mod.rs`)

**Problem:** Database model struct with many columns.

**Assessment:** This maps directly to a database table schema. Splitting a DB row struct often creates more complexity than it solves. **Low priority** — only address if fields naturally group into separate tables (normalization).

**Plan (if pursued):**
1. Evaluate if status/progress fields can become a `DownloadProgress` sub-struct
2. Evaluate if path-related fields can become a `DownloadPaths` sub-struct
3. Ensure `sqlx::FromRow` derivation still works with nested structs

**Acceptance criteria:** Only refactor if it simplifies query code. Do not introduce joins where flat queries currently suffice.

---

## W10: Complete TODO items in `src/post_processing.rs:456-458`

**Problem:** Three incomplete features:
```rust
None, // TODO: Add per-download password config
None, // TODO: Add NZB metadata password extraction
None, // TODO: Add global password file config
```

**Plan:**
1. Add `password: Option<String>` field to download config or `Download` struct
2. Parse `<head><meta type="password">` from NZB XML metadata
3. Add `password_file: Option<PathBuf>` to `Config` — reads one password per line
4. Pass resolved password list to extraction functions

**Acceptance criteria:** All three `None` values replaced with functional implementations. Tests cover each password source.

---

## Priority Order

| Priority | Items | Impact |
|----------|-------|--------|
| High | W1, W4 | Largest files, most complexity reduction |
| Medium | W2, W3, W5, W7 | Test separation and config clarity |
| Low | W6, W8, W9, W10 | Incremental improvements |

Each warning is independent and can be addressed in any order without blocking others.
