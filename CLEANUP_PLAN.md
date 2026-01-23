# Production Cleanup Plan

This document outlines all tasks needed to make usenet-dl production-ready with clean, professional documentation.

---

## 1. Remove Development Artifacts

### Files and Directories to Delete

| Path | Reason |
|------|--------|
| `.codemachine/` | Development tooling directory |
| `.ralph/` | Task management tooling directory |
| `plan.md` | Development notes |
| `plan_PROGRESS.md` | Development progress tracking |
| `CLAUDE.md` | Development instructions (keep as `.claude/CLAUDE.md` if needed for dev) |

### Cleanup Commands

```bash
rm -rf .codemachine/
rm -rf .ralph/
rm plan.md
rm plan_PROGRESS.md
```

---

## 2. Documentation Restructure

### Proposed Directory Structure

```
usenet-dl/
├── README.md                    # Project overview, quick start, badges
├── CHANGELOG.md                 # Version history (cleaned)
├── LICENSE-MIT                  # MIT license
├── LICENSE-APACHE               # Apache 2.0 license
├── docs/
│   ├── getting-started.md       # Installation, basic usage
│   ├── configuration.md         # All config options (from CONFIGURATION.md)
│   ├── api-reference.md         # REST API documentation (from API_USAGE.md)
│   ├── architecture.md          # System design, module overview
│   ├── post-processing.md       # Extraction, deobfuscation, cleanup
│   └── contributing.md          # Development guidelines (from CONTRIBUTING.md)
├── examples/
│   ├── README.md                # Examples index
│   ├── basic_download.rs
│   ├── custom_configuration.rs
│   ├── multi_subscriber.rs
│   ├── rest_api_server.rs
│   └── speedtest.rs
└── tests/
    └── manual/
        ├── api-testing.md       # API testing guide (from API_TESTING.md)
        ├── server-testing.md    # NNTP server testing (from MANUAL_SERVER_TESTING.md)
        └── rss-testing.md       # RSS testing (from RSS_MANUAL_TESTING.md)
```

### Files to Move/Rename

| Current | New Location |
|---------|--------------|
| `CONFIGURATION.md` | `docs/configuration.md` |
| `API_USAGE.md` | `docs/api-reference.md` |
| `CONTRIBUTING.md` | `docs/contributing.md` |
| `API_TESTING.md` | `tests/manual/api-testing.md` |
| `MANUAL_SERVER_TESTING.md` | `tests/manual/server-testing.md` |
| `RSS_MANUAL_TESTING.md` | `tests/manual/rss-testing.md` |

---

## 3. README.md Cleanup

### Remove

- Status section with percentages and task counts
- Phase completion checkmarks
- Roadmap section with checkboxes
- References to `implementation_1.md` and `implementation_1_PROGRESS.md`
- Emoji in headers

### Update

- Replace `yourusername` placeholder with actual GitHub username
- Update documentation links to new `docs/` paths
- Simplify feature lists (remove verbose task references)
- Add proper installation instructions for crates.io (when published)
- Clean up "Built with heart in Rust" footer

### Keep

- Feature overview
- Quick start code examples
- Architecture diagram
- Configuration examples
- API endpoint listing
- License information

---

## 4. CHANGELOG.md Cleanup

### Remove

- Task references (Tasks X.Y)
- Test count annotations (137 tests, etc.)
- Phase numbering in headers
- Implementation details (should only describe user-facing changes)
- References to non-existent files

### Update

- Consolidate into proper semantic versioning sections
- Focus on features, not implementation tasks
- Remove internal test counts

---

## 5. CONTRIBUTING.md Cleanup

### Remove

- Emoji at end of file
- References to non-existent example files in project structure
- `implementation_1.md` and `implementation_1_PROGRESS.md` references
- TODO placeholder for security email

### Update

- Project structure to match actual filesystem
- Example filenames in `examples/` section
- Security contact information

---

## 6. Code Comment Audit

### Search Patterns to Review

```bash
# Find TODO comments that reference AI/development
grep -rn "TODO" src/
grep -rn "FIXME" src/
grep -rn "Phase" src/
grep -rn "Task" src/

# Find overly verbose comments
grep -rn "This function" src/
grep -rn "This method" src/
```

### Guidelines

- Remove comments explaining obvious code
- Keep comments explaining "why", not "what"
- Remove any references to development phases or tasks
- Ensure rustdoc comments are professional and complete

---

## 7. Other Files to Clean

### `examples/README.md`

- Review for professional tone
- Ensure all referenced examples exist

### `postman_collection.json`

- Verify all endpoints are correct
- Remove any test/debug endpoints

### `.env.example` (if exists)

- Ensure no real credentials
- Professional variable names

### `docker/` directory

- Review for production readiness

---

## 8. Git Cleanup

### Update `.gitignore`

Ensure these are ignored:
```
# Development artifacts
.codemachine/
.ralph/
plan.md
plan_PROGRESS.md

# IDE
.idea/
.vscode/
*.swp

# Build
/target/
*.db

# Environment
.env
```

### Final Commit Message

```
chore: Production cleanup and documentation restructure

- Remove development tooling directories
- Reorganize documentation into docs/
- Clean README, CHANGELOG, CONTRIBUTING
- Remove task tracking references
- Professional code comments
```

---

## 9. Verification Checklist

- [ ] No `.codemachine/` or `.ralph/` directories
- [ ] No `plan.md` or `plan_PROGRESS.md` files
- [ ] No references to `implementation_1.md` anywhere
- [ ] No percentage completions or task counts in docs
- [ ] No emoji in documentation (except badges)
- [ ] All documentation links work
- [ ] `cargo doc` builds without warnings
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes
- [ ] Professional, consistent tone throughout

---

## 10. Optional Enhancements

### After Cleanup

1. **Add GitHub templates**
   - `.github/ISSUE_TEMPLATE/bug_report.md`
   - `.github/ISSUE_TEMPLATE/feature_request.md`
   - `.github/PULL_REQUEST_TEMPLATE.md`

2. **CI/CD configuration**
   - `.github/workflows/ci.yml` for automated testing

3. **Badges**
   - Build status
   - Code coverage
   - Documentation

4. **Security policy**
   - `SECURITY.md` with vulnerability reporting instructions

---

## Execution Order

1. Create `docs/` and `tests/manual/` directories
2. Move documentation files to new locations
3. Clean each documentation file
4. Delete development artifacts
5. Update `.gitignore`
6. Run verification checklist
7. Commit changes
