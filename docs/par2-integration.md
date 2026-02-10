# PAR2 Integration

usenet-dl has built-in support for PAR2 verification and repair through its `CliParityHandler`, which shells out to the `par2` binary. Without it, downloads still work — the post-processing pipeline gracefully skips the verify/repair stages. With it, damaged or incomplete downloads can be automatically detected and repaired.

## What PAR2 Does

Usenet posts often include PAR2 (Parity Archive 2.0) recovery files alongside the actual content. These files contain Reed-Solomon error correction data that can:

- **Verify** that all downloaded files are intact and uncorrupted
- **Repair** damaged or missing files using recovery blocks (up to the redundancy percentage the uploader chose)

In the post-processing pipeline, PAR2 runs as the first two stages (before extraction):

```
Download → [Verify] → [Repair] → Extract → Move → Cleanup
            ^^^^^^     ^^^^^^
            PAR2        PAR2
```

## Installing par2cmdline

### Linux

**Debian / Ubuntu:**
```bash
sudo apt install par2
```

**Arch Linux:**
```bash
sudo pacman -S par2cmdline
```

**Fedora:**
```bash
sudo dnf install par2cmdline
```

**NixOS / nix-shell:**
```nix
# In your shell.nix or configuration.nix
environment.systemPackages = [ pkgs.par2cmdline ];
```

### macOS

```bash
brew install par2
```

### Docker

If bundling in a container image:

```dockerfile
# Debian-based
RUN apt-get update && apt-get install -y par2 && rm -rf /var/lib/apt/lists/*

# Alpine
RUN apk add --no-cache par2cmdline
```

### Verify Installation

```bash
par2 --version
# Should output something like: par2cmdline version 0.8.1
```

usenet-dl looks for a binary named `par2` in your PATH. You can confirm it's discoverable:

```bash
which par2
# /usr/bin/par2
```

## Configuration

### Automatic (default)

With default settings, usenet-dl automatically searches your PATH for `par2` at startup. No configuration needed — just install it and it works.

```toml
# These are the defaults — you don't need to set them
search_path = true
# par2_path is unset (auto-detect)
```

### Explicit Path

Point to a specific binary location:

```toml
par2_path = "/usr/local/bin/par2"
```

```json
{
  "par2_path": "/usr/local/bin/par2"
}
```

This is useful when:
- `par2` is installed in a non-standard location
- You're bundling it as a sidecar binary (e.g., in a Tauri app)
- You want to use a specific version or fork (like `par2cmdline-turbo`)

### Disable PAR2

To explicitly skip PAR2 even if the binary is available:

```toml
search_path = false
# Don't set par2_path
```

This forces the `NoOpParityHandler`, which skips verify/repair stages.

## Behavior

### With par2 Available

| Stage | Behavior |
|-------|----------|
| Verify | Runs `par2 v <file.par2>` — checks all files against PAR2 checksums |
| Repair | Runs `par2 r <file.par2>` — reconstructs damaged/missing blocks |

If verification finds damage but enough recovery blocks exist, repair runs automatically. If there aren't enough recovery blocks, the download is marked as failed.

### Without par2

| Stage | Behavior |
|-------|----------|
| Verify | Skipped (emits `VerifySkipped` event) |
| Repair | Skipped (emits `RepairSkipped` event) |

The pipeline continues to extraction. This means corrupted files may cause extraction failures, but the download itself won't be blocked.

### Checking Capabilities

**REST API:**
```bash
curl http://localhost:6789/api/v1/capabilities
```

```json
{
  "parity": {
    "can_verify": true,
    "can_repair": true,
    "handler": "cli-par2"
  }
}
```

**Rust API:**
```rust
let downloader = UsenetDownloader::new(config).await?;
let caps = downloader.capabilities();
println!("PAR2 handler: {}", caps.parity.handler);
println!("Can verify: {}", caps.parity.can_verify);
println!("Can repair: {}", caps.parity.can_repair);
```

## par2cmdline-turbo

[par2cmdline-turbo](https://github.com/animetosho/par2cmdline-turbo) is a performance-optimized fork with SIMD acceleration. It's a drop-in replacement — same CLI interface, same output format. usenet-dl works with it without any code changes.

```bash
# Point to the turbo binary
par2_path = "/usr/local/bin/par2cmdline-turbo"
```

Or just name it `par2` and put it in your PATH.

## Events

Subscribe to post-processing events for PAR2 status:

| Event | When |
|-------|------|
| `Verifying { id }` | Verification started |
| `VerifyComplete { id, damaged }` | Verification finished |
| `VerifySkipped { id }` | PAR2 not available, skipped |
| `Repairing { id, blocks_needed, blocks_available }` | Repair started |
| `RepairComplete { id, success }` | Repair finished |
| `RepairSkipped { id }` | PAR2 not available, skipped |

## See Also

- [Post-Processing](post-processing.md) — Full pipeline documentation
- [Configuration](configuration.md) — All configuration options including `par2_path` and `search_path`
