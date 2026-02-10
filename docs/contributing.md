# Contributing to usenet-dl

Thank you for your interest in contributing to usenet-dl! This document provides guidelines and instructions for contributing to the project.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [Development Workflow](#development-workflow)
- [Testing Guidelines](#testing-guidelines)
- [Code Style](#code-style)
- [Documentation](#documentation)
- [Submitting Changes](#submitting-changes)
- [Issue Reporting](#issue-reporting)
- [Performance Considerations](#performance-considerations)
- [Security](#security)

## Code of Conduct

By participating in this project, you agree to maintain a respectful, inclusive environment. Be considerate of others, provide constructive feedback, and help create a welcoming community.

## Getting Started

### Prerequisites

- **Rust 1.93+** - Install from [rustup.rs](https://rustup.rs/)
- **SQLite 3** - Usually pre-installed on most systems
- **External archive tools** (for extraction features):
  - `unrar` - For RAR archive extraction
  - `7z` - For 7-Zip archive extraction
- **Git** - For version control

### Quick Start

> **Note**: All cargo commands should be run inside `nix-shell` to ensure correct toolchain and system dependencies are available.

```bash
# Clone the repository
git clone https://github.com/jvz-devx/usenet-dl.git
cd usenet-dl

# Build the project
nix-shell --run "cargo build"

# Run tests
nix-shell --run "cargo test"

# Run tests with output
nix-shell --run "cargo test -- --nocapture"

# Build documentation
nix-shell --run "cargo doc --no-deps --open"
```

## Development Setup

### 1. Install Dependencies

#### Linux (Debian/Ubuntu)
```bash
sudo apt-get install unrar p7zip-full sqlite3
```

#### macOS
```bash
brew install unrar p7zip sqlite
```

#### Windows
- Download and install [7-Zip](https://www.7-zip.org/)
- Download and install [WinRAR](https://www.win-rar.com/)
- SQLite is included with Windows

### 2. Configure Development Environment

```bash
# Install cargo tools for development
cargo install cargo-watch    # Auto-rebuild on file changes
cargo install cargo-tarpaulin # Code coverage

# Optional: Install clippy and rustfmt (usually included with rustup)
rustup component add clippy
rustup component add rustfmt
```

### 3. Set Up Pre-commit Hooks (Optional)

Create `.git/hooks/pre-commit`:

```bash
#!/bin/bash
set -e

# Format check
cargo fmt -- --check

# Clippy check
cargo clippy --all-targets

# Run tests
cargo test --quiet
```

Make it executable:
```bash
chmod +x .git/hooks/pre-commit
```

## Project Structure

```
usenet-dl/
├── src/
│   ├── lib.rs              # Library root, re-exports, tests
│   ├── config.rs           # Configuration types and defaults
│   ├── types.rs            # Core types (DownloadId, Status, Priority, Event, etc.)
│   ├── error.rs            # Error types and conversions
│   ├── deobfuscation.rs    # Filename deobfuscation
│   ├── folder_watcher.rs   # NZB folder watching
│   ├── rss_scheduler.rs    # RSS feed scheduling
│   ├── scheduler_task.rs   # Scheduler background task
│   ├── speed_limiter.rs    # Token bucket speed limiting
│   ├── retry.rs            # Exponential backoff retry logic
│   ├── utils.rs            # Utility functions (disk space, etc.)
│   ├── downloader/         # Main orchestration
│   │   ├── mod.rs          # UsenetDownloader struct, core coordination
│   │   ├── nzb.rs          # NZB add/import methods
│   │   ├── control.rs      # Pause/resume/cancel/reprocess
│   │   ├── queue.rs         # Priority queue management
│   │   ├── queue_processor.rs # Queue dispatch loop
│   │   ├── download_task/   # Per-download task execution
│   │   ├── background_tasks.rs # Folder watcher, RSS, scheduler spawning
│   │   ├── webhooks.rs     # Webhook dispatch
│   │   └── tests/          # Downloader tests
│   ├── db/                 # SQLite persistence layer
│   │   ├── mod.rs          # Database struct, connection management
│   │   ├── downloads.rs    # Download CRUD operations
│   │   ├── articles.rs     # Article-level tracking
│   │   ├── history.rs      # Completed download history
│   │   ├── rss.rs          # RSS feed persistence
│   │   ├── migrations.rs   # Schema migrations
│   │   └── tests/          # Database tests
│   ├── api/                # REST API layer
│   │   ├── mod.rs          # Router and middleware
│   │   ├── openapi.rs      # OpenAPI documentation
│   │   ├── auth.rs         # API key authentication
│   │   ├── rate_limit.rs   # Per-IP rate limiting
│   │   ├── state.rs        # Shared application state
│   │   ├── routes/         # HTTP endpoint handlers (per resource)
│   │   └── tests/          # API tests
│   ├── extraction/         # Archive extraction
│   │   ├── mod.rs          # Dispatcher and common logic
│   │   ├── rar.rs          # RAR extraction
│   │   ├── sevenz.rs       # 7z extraction
│   │   ├── zip.rs          # ZIP extraction
│   │   ├── password_list.rs # Password source collection
│   │   └── tests/          # Extraction tests
│   ├── post_processing/    # Post-processing pipeline
│   │   ├── mod.rs          # Pipeline orchestration
│   │   ├── verify.rs       # PAR2 verification stage
│   │   ├── repair.rs       # PAR2 repair stage
│   │   ├── cleanup.rs      # Cleanup stage
│   │   └── tests/          # Post-processing tests
│   ├── parity/             # PAR2 verification and repair
│   │   ├── mod.rs          # Module root
│   │   ├── traits.rs       # ParityHandler trait
│   │   ├── cli.rs          # CLI par2 implementation
│   │   ├── noop.rs         # No-op fallback
│   │   └── parser.rs       # PAR2 output parser
│   ├── rss_manager/        # RSS feed monitoring
│   │   ├── mod.rs          # Feed polling, filtering, auto-download
│   │   └── tests/          # RSS tests
│   └── scheduler/          # Time-based scheduler
│       ├── mod.rs          # Schedule rule evaluation
│       └── tests/          # Scheduler tests
├── examples/
│   ├── basic_download.rs       # Simple usage example
│   ├── rest_api_server.rs      # REST API example
│   ├── custom_configuration.rs # Configuration example
│   ├── multi_subscriber.rs     # Event subscription example
│   ├── speedtest.rs            # Performance testing
│   └── README.md               # Examples documentation
├── tests/
│   └── manual/                 # Manual testing guides
│       ├── api-testing.md      # API testing procedures
│       ├── server-testing.md   # NNTP server testing
│       └── rss-testing.md      # RSS feed testing
├── docs/
│   ├── getting-started.md      # Installation and basic usage
│   ├── configuration.md        # Configuration reference
│   ├── api-reference.md        # REST API documentation
│   ├── architecture.md         # System design and modules
│   ├── post-processing.md      # Extraction and cleanup
│   └── contributing.md         # This file
├── README.md               # Project overview
├── CHANGELOG.md            # Version history
└── Cargo.toml              # Project manifest
```

### Module Responsibilities

- **`lib.rs`**: Library root, public re-exports
- **`downloader/`**: Core `UsenetDownloader` struct, queue management, download orchestration, webhooks
- **`db/`**: All SQLite operations, schema migrations, article tracking, history
- **`api/`**: REST API layer (Axum), OpenAPI spec generation (utoipa), SSE, auth, rate limiting
- **`extraction/`**: Archive format handling (RAR/7z/ZIP), password trials, nested extraction
- **`post_processing/`**: Pipeline orchestration (verify → repair → extract → move → cleanup)
- **`parity/`**: PAR2 verification and repair (trait-based: CLI handler + no-op fallback)
- **`scheduler/`**: Time-based rules for speed limits and pause/resume
- **`rss_manager/`**: RSS feed polling, filtering, auto-download
- **`folder_watcher.rs`**: File system monitoring with `notify` crate

## Development Workflow

### 1. Create a Feature Branch

```bash
git checkout -b feature/your-feature-name
# or
git checkout -b fix/issue-number-description
```

### 2. Make Your Changes

Follow these principles:

- **Keep changes focused**: One feature or fix per branch
- **Write tests first**: TDD is encouraged
- **Update documentation**: Keep docs in sync with code
- **Follow existing patterns**: Maintain consistency with the codebase

### 3. Test Your Changes

> **Note**: All cargo commands should be run inside `nix-shell` to ensure correct toolchain and system dependencies.

```bash
# Run all tests
nix-shell --run "cargo test"

# Run specific test module
nix-shell --run "cargo test db::tests"

# Run specific test
nix-shell --run "cargo test test_queue_priority"

# Run with logging output
nix-shell --run "RUST_LOG=debug cargo test -- --nocapture"

# Run tests in release mode (faster)
nix-shell --run "cargo test --release"
```

### 4. Check Code Quality

```bash
# Format code
nix-shell --run "cargo fmt --all"

# Check for common mistakes
nix-shell --run "cargo clippy --all-targets"

# Check for unused dependencies
cargo udeps  # requires: cargo install cargo-udeps

# Security audit
cargo audit  # requires: cargo install cargo-audit
```

### 5. Build Documentation

```bash
# Build and view docs
nix-shell --run "cargo doc --no-deps --open"

# Check for broken links in docs
nix-shell --run "cargo doc --no-deps 2>&1 | grep warning"
```

## Testing Guidelines

### Test Organization

Tests are located inline with the code using `#[cfg(test)]` modules at the bottom of each file.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_feature() {
        // Test implementation
    }
}
```

### Writing Good Tests

1. **Use descriptive names**: `test_queue_respects_priority_ordering`
2. **Test one thing**: Each test should verify a single behavior
3. **Use AAA pattern**: Arrange, Act, Assert
4. **Include edge cases**: Empty inputs, large values, error conditions
5. **Clean up resources**: Use `tempfile` crate for temporary files/directories

### Test Categories

#### Unit Tests
Test individual functions and methods in isolation.

```rust
#[tokio::test]
async fn test_speed_limiter_unlimited() {
    let limiter = SpeedLimiter::new(None);
    let start = Instant::now();
    limiter.acquire(1_000_000).await;
    assert!(start.elapsed() < Duration::from_millis(10));
}
```

#### Integration Tests
Test interactions between components.

```rust
#[tokio::test]
async fn test_download_with_post_processing() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = Config {
        download: DownloadConfig {
            download_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        },
        ..Default::default()
    };
    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Test full download and post-processing flow
}
```

#### API Tests
Test HTTP endpoints using the Axum test helpers.

```rust
#[tokio::test]
async fn test_get_downloads_endpoint() {
    let app = create_test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/downloads")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
```

### Test Coverage

Aim for:
- **80%+ line coverage** for core functionality
- **100% coverage** for error handling paths
- **All public API** methods tested

Check coverage with:
```bash
cargo tarpaulin --out Html
```

## Code Style

### Formatting

- Use `rustfmt` with default settings
- Run `cargo fmt` before committing
- Maximum line length: 100 characters (rustfmt default)

### Naming Conventions

- **Types**: `PascalCase` (e.g., `DownloadInfo`, `PostProcess`)
- **Functions/variables**: `snake_case` (e.g., `add_download`, `download_id`)
- **Constants**: `SCREAMING_SNAKE_CASE` (e.g., `DEFAULT_SPEED_LIMIT`)
- **Type parameters**: Single uppercase letter (e.g., `T`, `E`) or descriptive `PascalCase`

### Code Organization

```rust
// 1. Imports
use std::path::PathBuf;
use tokio::sync::broadcast;

// 2. Type definitions
pub struct Config { }

// 3. Implementations
impl Config { }

// 4. Tests (at end of file)
#[cfg(test)]
mod tests { }
```

### Error Handling

- Use `thiserror` for error types
- Provide context with error messages
- Don't panic in library code (except in tests or unrecoverable situations)

```rust
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download not found: {0}")]
    NotFound(i64),

    #[error("failed to connect to server: {0}")]
    ConnectionFailed(#[from] std::io::Error),
}
```

### Comments and Documentation

- Use `///` for public API documentation
- Use `//` for inline comments
- Document **why**, not **what** (code should be self-explanatory)
- Add examples to complex public APIs

```rust
/// Adds an NZB file to the download queue.
///
/// # Arguments
///
/// * `path` - Path to the NZB file
/// * `options` - Download options (category, priority, etc.)
///
/// # Returns
///
/// The unique download ID assigned to this download.
///
/// # Errors
///
/// Returns `DownloadError::InvalidNzb` if the NZB file is malformed.
///
/// # Examples
///
/// ```no_run
/// # use usenet_dl::{UsenetDownloader, Config, DownloadOptions};
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let downloader = UsenetDownloader::new(Config::default()).await?;
/// let id = downloader.add_nzb(
///     "movie.nzb",
///     DownloadOptions::default(),
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn add_nzb(
    &self,
    path: impl AsRef<Path>,
    options: DownloadOptions,
) -> Result<DownloadId, DownloadError> {
    // Implementation
}
```

### Async Code

- Use `async/await` consistently
- Prefer `tokio::spawn` for true parallelism
- Use `tokio::select!` for cancellation
- Avoid blocking operations in async functions

```rust
// Good: Non-blocking
async fn download_article(&self, id: &str) -> Result<Vec<u8>> {
    self.client.fetch_article(id).await
}

// Bad: Blocking in async
async fn bad_example(&self) -> Result<()> {
    std::thread::sleep(Duration::from_secs(1)); // DON'T DO THIS
    Ok(())
}

// Good: Use tokio::time::sleep
async fn good_example(&self) -> Result<()> {
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(())
}
```

## Documentation

### Code Documentation

All public APIs must have rustdoc comments with:
- One-line summary
- Detailed description (if needed)
- Parameters (`# Arguments`)
- Return value (`# Returns`)
- Errors (`# Errors`)
- Examples (`# Examples`) for complex APIs
- Panics (`# Panics`) if the function can panic

### Documentation Files

When updating features, also update:
- **README.md**: Feature list, quick start
- **docs/api-reference.md**: REST API endpoints and examples
- **docs/configuration.md**: Configuration options
- **CHANGELOG.md**: Version history (follow [Keep a Changelog](https://keepachangelog.com/))

### Examples

Add runnable examples to `examples/` for:
- Common use cases
- New major features
- Integration patterns

## Submitting Changes

### Pull Request Process

1. **Update your branch**:
   ```bash
   git fetch origin
   git rebase origin/main
   ```

2. **Ensure all checks pass**:
   ```bash
   nix-shell --run "cargo fmt --all -- --check"
   nix-shell --run "cargo clippy --all-targets"
   nix-shell --run "cargo test"
   nix-shell --run "cargo doc --no-deps"
   ```

3. **Write a clear commit message**:
   ```
   feat: Add support for custom archive extraction tools

   - Allow users to specify paths to unrar/7z executables
   - Add config validation for tool paths
   - Add tests for custom tool configuration

   Closes #123
   ```

4. **Push your branch**:
   ```bash
   git push origin feature/your-feature-name
   ```

5. **Create a Pull Request** with:
   - Clear title describing the change
   - Description of what changed and why
   - Reference to related issues
   - Screenshots/examples if applicable
   - Checklist of completed items:
     - [ ] Tests pass
     - [ ] Documentation updated
     - [ ] Changelog updated
     - [ ] Code formatted and linted

### Commit Message Guidelines

Follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` New feature
- `fix:` Bug fix
- `docs:` Documentation only
- `style:` Code style changes (formatting, etc.)
- `refactor:` Code refactoring without behavior change
- `test:` Adding or updating tests
- `chore:` Maintenance tasks

Examples:
```
feat: Add webhook notification support
fix: Handle empty NZB files gracefully
docs: Update API usage examples
test: Add integration tests for RSS manager
refactor: Simplify retry logic implementation
```

## Issue Reporting

### Before Creating an Issue

- Search existing issues to avoid duplicates
- Verify the issue with the latest version
- Collect relevant information (logs, config, steps to reproduce)

### Bug Reports

Include:
- **Description**: Clear, concise summary
- **Steps to reproduce**: Numbered list
- **Expected behavior**: What should happen
- **Actual behavior**: What actually happens
- **Environment**:
  - OS and version
  - Rust version (`rustc --version`)
  - usenet-dl version
- **Logs**: Relevant error messages or logs
- **Configuration**: Sanitized config (remove credentials!)

### Feature Requests

Include:
- **Description**: What feature you'd like
- **Use case**: Why it's needed
- **Proposed solution**: How it could work
- **Alternatives**: Other approaches considered
- **Examples**: Similar features in other tools

## Performance Considerations

### When Adding Features

- **Benchmark performance impact** for hot paths
- **Avoid allocations** in tight loops
- **Use appropriate data structures** (HashMap vs BTreeMap, Vec vs VecDeque)
- **Profile with** `cargo flamegraph` for CPU-intensive code

### Database Operations

- **Batch inserts** when possible
- **Use indexes** for frequently queried columns
- **Prepare statements** for repeated queries
- **Use transactions** for multiple related operations

### Async Best Practices

- **Don't block the runtime**: No `std::thread::sleep` or blocking I/O
- **Limit concurrency**: Use semaphores to limit concurrent operations
- **Cancel gracefully**: Support tokio cancellation tokens
- **Minimize lock contention**: Prefer message passing over shared state

## Security

### Reporting Security Issues

**Do not open public issues for security vulnerabilities.**

Please report security vulnerabilities by opening a private security advisory on GitHub or by contacting the maintainers directly.

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

### Security Guidelines

- **Never log sensitive data**: Passwords, API keys, tokens
- **Validate all inputs**: Especially paths, URLs, NZB content
- **Use secure defaults**: Authentication required, localhost binding
- **Audit dependencies**: Run `cargo audit` regularly
- **Handle errors safely**: Don't leak internal state in error messages

### Code Review Checklist

Reviewers should verify:
- [ ] No SQL injection vulnerabilities
- [ ] Path traversal prevented
- [ ] Input validation present
- [ ] Sensitive data not logged
- [ ] Credentials properly redacted
- [ ] Rate limiting considered
- [ ] Error messages safe for users

## Development Tips

### Useful Commands

```bash
# Watch for changes and rebuild
nix-shell --run "cargo watch -x build"

# Watch and run tests
nix-shell --run "cargo watch -x test"

# Run a specific example
nix-shell --run "cargo run --example basic_download"

# Generate and view API docs
nix-shell --run "cargo doc --no-deps --open"

# Check what features are enabled
nix-shell --run "cargo tree --features"

# Clean build artifacts
nix-shell --run "cargo clean"

# Update dependencies
nix-shell --run "cargo update"
```

### Debugging

```bash
# Run with debug logging
nix-shell --run "RUST_LOG=debug cargo test test_name -- --nocapture"

# Run with trace logging (very verbose)
nix-shell --run "RUST_LOG=trace cargo test test_name -- --nocapture"

# Run specific module tests with logging
nix-shell --run "RUST_LOG=usenet_dl::db=debug cargo test db::tests -- --nocapture"
```

### Working with SQLite

```bash
# Open the database
sqlite3 usenet-dl.db

# View schema
.schema

# Query downloads
SELECT id, name, status FROM downloads;

# Exit
.quit
```

## Questions?

- **Documentation**: Check README.md and the `docs/` directory
- **Examples**: See `examples/` directory
- **Issues**: Search existing issues on GitHub
- **Discussions**: Open a GitHub discussion for questions

## License

By contributing to usenet-dl, you agree that your contributions will be licensed under the same license as the project (MIT OR Apache-2.0).

---

Thank you for contributing to usenet-dl!
