---
name: rust-project-init
description: Scaffold a new Rust project with correct structure, dependencies, and configuration. Handles workspace, library, and binary crate setup.
user-invocable: true
---

# Rust Project Initializer

Set up a new Rust project with the right structure from the start. Avoids the "generate code, then fix 50 import errors" cycle.

## When Invoked

The user wants to create a new Rust project, add a crate to a workspace, or set up a project structure.

## Process

### Step 1: Clarify Project Type

Ask if not obvious:
- **Binary** (`src/main.rs`) — an executable
- **Library** (`src/lib.rs`) — reusable code for other crates
- **Both** (`src/main.rs` + `src/lib.rs`) — binary that uses its own library
- **Workspace** — multiple crates in one repository

### Step 2: Initialize

```bash
# Binary
cargo init my-project

# Library
cargo init --lib my-project

# Workspace: create Cargo.toml at root
```

Workspace `Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = [
    "crates/*",
]

[workspace.dependencies]
# Shared dependency versions here
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
thiserror = "2"
```

### Step 3: Project Structure

```
my-project/
├── Cargo.toml
├── src/
│   ├── lib.rs          # Library root (pub mod declarations)
│   ├── main.rs          # Binary entry (if applicable)
│   ├── error.rs         # Error types
│   └── {module}/
│       ├── mod.rs       # Module root
│       └── ...
├── tests/               # Integration tests
│   └── integration.rs
├── benches/             # Benchmarks
├── examples/            # Example programs
└── .cargo/
    └── config.toml      # Cargo configuration
```

### Step 4: Essential Cargo.toml Configuration

```toml
[package]
name = "my-project"
version = "0.1.0"
edition = "2024"          # Use latest stable edition
rust-version = "1.85"     # Minimum supported Rust version

[dependencies]
# Add based on project needs

[dev-dependencies]
# Testing dependencies

[lints.rust]
unsafe_code = "forbid"    # Unless unsafe is intentionally needed

[lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
```

### Step 5: Common Dependency Sets

**CLI application:**
```toml
clap = { version = "4", features = ["derive"] }
anyhow = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
```

**Web API (Axum):**
```toml
axum = "0.8"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

**Library crate:**
```toml
thiserror = "2"
serde = { version = "1", features = ["derive"], optional = true }

[features]
default = []
serde = ["dep:serde"]
```

### Step 6: CI / Tooling Setup

Recommend these in `Cargo.toml` or as config:
- `cargo fmt` — formatting (add `rustfmt.toml` if custom rules needed)
- `cargo clippy` — linting
- `cargo test` — testing
- `cargo doc --no-deps` — documentation

Minimal `rustfmt.toml`:
```toml
edition = "2024"
```

Minimal `.cargo/config.toml`:
```toml
[alias]
xtask = "run --package xtask --"
```

## After Initialization

1. Run `cargo check` to verify everything compiles
2. Run `cargo clippy` to catch any issues
3. Verify module structure: every `mod` declaration has a matching file
4. Verify dependency versions are current (check crates.io)
