# Progress: CLEANUP_PLAN

Started: za 24 jan 2026  0:47:13 CET

## Status

IN_PROGRESS

## Task List

### 1. Documentation Structure Setup
- [x] Create `docs/` directory
- [x] Create `tests/manual/` directory

### 2. Move Documentation Files
- [x] Move `CONFIGURATION.md` to `docs/configuration.md`
- [x] Move `API_USAGE.md` to `docs/api-reference.md`
- [x] Move `CONTRIBUTING.md` to `docs/contributing.md`
- [x] Move `API_TESTING.md` to `tests/manual/api-testing.md`
- [x] Move `MANUAL_SERVER_TESTING.md` to `tests/manual/server-testing.md`
- [x] Move `RSS_MANUAL_TESTING.md` to `tests/manual/rss-testing.md`

### 3. Create New Documentation
- [x] Create `docs/getting-started.md`
- [x] Create `docs/architecture.md`
- [x] Create `docs/post-processing.md`
- [x] Create `tests/manual/README.md`

### 4. Clean Documentation Files
- [x] Clean `README.md` (remove status sections, emoji, placeholders)
- [x] Clean `CHANGELOG.md` (remove task references, test counts)
- [x] Clean `docs/contributing.md` (after move)
- [x] Update all documentation links to new paths

### 5. Remove Development Artifacts
- [x] Delete `.codemachine/` directory
- [x] Delete `.ralph/` directory
- [x] Delete `plan.md`
- [x] Delete `plan_PROGRESS.md`
- [x] Delete `CLAUDE.md` (keep in `.claude/` if exists)

### 6. Code Comment Audit
- [x] Search for and review TODO/FIXME comments
- [x] Remove Phase/Task references from code
- [x] Clean overly verbose comments

### 7. Git Configuration
- [ ] Update `.gitignore` with development artifacts
- [ ] Verify `.env.example` is clean (if exists)

### 8. Verification
- [ ] Run `cargo doc` (no warnings)
- [ ] Run `cargo test` (all pass)
- [ ] Run `cargo clippy` (no issues)
- [ ] Verify all documentation links work
- [ ] Verify professional tone throughout

## Completed This Iteration

- Code Comment Audit (Task 6):
  - Cleaned all TODO/FIXME comments from development style to production style
  - Removed all "Task X.Y" references from test files
  - Removed all "Phase X" references from documentation
  - Changed "TODO: Add multi-server failover in future tasks" → "Note: Multi-server failover not yet implemented"
  - Changed "TODO: Track actual stage" → "Default to Extract stage"
  - Cleaned PAR2 and extraction TODOs to be more professional
  - Cleaned API route TODO about delete_files parameter
  - Changed "Phase 1", "Phase 4" references to "current implementation" and "future extensions"
  - Verified with cargo check - all changes compile successfully

## Notes

- Starting with directory structure and file moves first
- Will verify each step before moving to next
