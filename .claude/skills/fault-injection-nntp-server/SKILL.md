---
name: fault-injection-nntp-server
description: Start a fault injection NNTP server for testing Usenet download clients. Use this to simulate network failures, protocol violations, timeouts, encoding issues, and compression errors.
argument-hint: "[--config <path>] [--port <port>] [--daemon|--stop]"
allowed-tools: Bash, Read, Write
---

# Fault Injection NNTP Server

A Tokio-based NNTP server that injects configurable faults for testing download client resilience.

## Quick Start

```bash
# Build the server (first time only)
cd .claude/skills/fault-injection-nntp-server && cargo build --release

# Start in foreground with protocol faults
.claude/skills/fault-injection-nntp-server/target/release/fault-nntp-server \
  --config .claude/skills/fault-injection-nntp-server/configs/protocol-faults.toml

# Or start as daemon
.claude/skills/fault-injection-nntp-server/target/release/fault-nntp-server \
  --daemon --config .claude/skills/fault-injection-nntp-server/configs/stress-test.toml

# Stop daemon
.claude/skills/fault-injection-nntp-server/target/release/fault-nntp-server --stop
```

## Usage

### Arguments

| Argument | Description |
|----------|-------------|
| `--config <path>` | Path to TOML configuration file |
| `--port <port>` | Override port (default: 1190) |
| `--daemon` | Run as background daemon |
| `--stop` | Stop running daemon |
| `--pid-file <path>` | PID file location (default: /tmp/fault-nntp.pid) |

### Running Tests Against It

```bash
# Set environment variables for your test suite
export NNTP_HOST=127.0.0.1
export NNTP_PORT=1190

# Run your tests
cargo test
```

## Configuration Presets

| Preset | Description |
|--------|-------------|
| `connection-faults.toml` | Connection drops, hangs, resets |
| `protocol-faults.toml` | Malformed responses, invalid codes |
| `encoding-faults.toml` | UTF-8 errors, NUL bytes, BOM |
| `stress-test.toml` | High fault rate chaos testing |

## Configuration Format

```toml
[server]
port = 1190
max_connections = 10

[faults.connection]
reject_prob = 0.0           # Probability to reject connection
hang_on_connect_ms = 0      # Delay before greeting
close_after_greeting = false

[faults.response]
malformed_status_prob = 0.0 # Invalid status line
invalid_code_prob = 0.0     # Non-standard response codes
truncate_prob = 0.0         # Cut response mid-stream
missing_terminator_prob = 0.0 # No dot terminator on multiline

[faults.encoding]
invalid_utf8_prob = 0.0     # Inject invalid UTF-8 sequences
nul_bytes_prob = 0.0        # Inject NUL bytes
wrong_line_endings_prob = 0.0 # Mix \r\n and \n
bom_prefix_prob = 0.0       # Add BOM to responses

[faults.timing]
slow_drip_bytes_per_sec = 0 # Slow response (0 = disabled)
freeze_mid_response_prob = 0.0
freeze_duration_ms = 5000

[faults.compression]
corrupt_gzip_prob = 0.0     # Corrupt GZIP data
truncate_compressed_prob = 0.0
fake_marker_prob = 0.0      # [COMPRESS=GZIP] but not compressed
```

## Fault Categories

See [faults.md](faults.md) for the complete catalog of 50+ injectable fault scenarios.

## Supported NNTP Commands

- `CAPABILITIES` - Returns capability list
- `AUTHINFO USER/PASS` - Always succeeds
- `GROUP <name>` - Selects group (mock data)
- `ARTICLE/HEAD/BODY/STAT <id>` - Returns mock article
- `XOVER <range>` - Returns mock overview
- `XHDR <header> <range>` - Returns mock headers
- `QUIT` - Closes connection

## Logging

The server logs all injected faults with tracing. Set `RUST_LOG` for verbosity:

```bash
RUST_LOG=debug ./fault-nntp-server --config config.toml
```
