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
- [ ] Move `MANUAL_SERVER_TESTING.md` to `tests/manual/server-testing.md`
- [ ] Move `RSS_MANUAL_TESTING.md` to `tests/manual/rss-testing.md`

### 3. Create New Documentation
- [ ] Create `docs/getting-started.md`
- [ ] Create `docs/architecture.md`
- [ ] Create `docs/post-processing.md`
- [ ] Create `tests/manual/README.md`

### 4. Clean Documentation Files
- [ ] Clean `README.md` (remove status sections, emoji, placeholders)
- [ ] Clean `CHANGELOG.md` (remove task references, test counts)
- [ ] Clean `docs/contributing.md` (after move)
- [ ] Update all documentation links to new paths

### 5. Remove Development Artifacts
- [ ] Delete `.codemachine/` directory
- [ ] Delete `.ralph/` directory
- [ ] Delete `plan.md`
- [ ] Delete `plan_PROGRESS.md`
- [ ] Delete `CLAUDE.md` (keep in `.claude/` if exists)

### 6. Code Comment Audit
- [ ] Search for and review TODO/FIXME comments
- [ ] Remove Phase/Task references from code
- [ ] Clean overly verbose comments

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

- Moved `API_TESTING.md` to `tests/manual/api-testing.md` using git mv

## Notes

- Starting with directory structure and file moves first
- Will verify each step before moving to next
