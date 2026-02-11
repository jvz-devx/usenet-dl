# Configuration Guide

This document describes the complete configuration file format for usenet-dl. Configuration can be provided in either TOML or JSON format.

## Quick Start

The minimal configuration requires only NNTP server settings:

### TOML (config.toml)
```toml
[[servers]]
host = "news.example.com"
port = 563
tls = true
username = "myuser"
password = "mypass"
connections = 10
priority = 0
pipeline_depth = 10
```

### JSON (config.json)
```json
{
  "servers": [
    {
      "host": "news.example.com",
      "port": 563,
      "tls": true,
      "username": "myuser",
      "password": "mypass",
      "connections": 10,
      "priority": 0,
      "pipeline_depth": 10
    }
  ]
}
```

All other settings use sensible defaults and are optional.

---

## Complete Configuration Reference

### Top-Level Settings

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `servers` | Array of `ServerConfig` | `[]` | NNTP server configurations (at least one required) |
| `download_dir` | String (path) | `"downloads"` | Directory for completed downloads |
| `temp_dir` | String (path) | `"temp"` | Temporary directory for work files |
| `max_concurrent_downloads` | Integer | `3` | Maximum number of concurrent downloads |
| `speed_limit_bps` | Integer (optional) | `null` | Global speed limit in bytes per second (null = unlimited) |
| `retry` | `RetryConfig` | See below | Retry configuration for transient failures |
| `default_post_process` | String | `"unpack_and_cleanup"` | Default post-processing mode |
| `delete_samples` | Boolean | `true` | Delete sample files/folders during cleanup |
| `extraction` | `ExtractionConfig` | See below | Archive extraction settings |
| `file_collision` | String | `"rename"` | How to handle filename collisions |
| `deobfuscation` | `DeobfuscationConfig` | See below | Filename deobfuscation settings |
| `duplicate` | `DuplicateConfig` | See below | Duplicate detection settings |
| `disk_space` | `DiskSpaceConfig` | See below | Disk space checking settings |
| `cleanup` | `CleanupConfig` | See below | Cleanup configuration |
| `direct_unpack` | `DirectUnpackConfig` | See below | DirectUnpack configuration (extract during download) |
| `password_file` | String (path, optional) | `null` | Path to file with passwords (one per line) |
| `try_empty_password` | Boolean | `true` | Try empty password as fallback for archives |
| `unrar_path` | String (path, optional) | `null` | Path to unrar executable (auto-detected if null) |
| `sevenzip_path` | String (path, optional) | `null` | Path to 7z executable (auto-detected if null) |
| `par2_path` | String (path, optional) | `null` | Path to par2 binary for repair operations (auto-detected if null) |
| `search_path` | Boolean | `true` | Search system PATH for external binaries if explicit paths not set |
| `persistence.database_path` | String (path) | `"usenet-dl.db"` | SQLite database path (nested under `persistence`) |
| `api` | `ApiConfig` | See below | REST API configuration |
| `persistence.schedule_rules` | Array of `ScheduleRule` | `[]` | Time-based speed limit rules (nested under `persistence`) |
| `watch_folders` | Array of `WatchFolderConfig` | `[]` | Folders to watch for NZB imports |
| `rss_feeds` | Array of `RssFeedConfig` | `[]` | RSS feed configurations |
| `webhooks` | Array of `WebhookConfig` | `[]` | Webhook configurations |
| `scripts` | Array of `ScriptConfig` | `[]` | Script execution configurations |
| `persistence.categories` | Object (string → `CategoryConfig`) | `{}` | Category-specific configurations (nested under `persistence`) |

---

## ServerConfig

NNTP server connection settings. Multiple servers can be configured for load distribution or backup.

### TOML
```toml
[[servers]]
host = "news.primary.com"
port = 563
tls = true
username = "myuser"
password = "mypass"
connections = 10
priority = 0  # Lower = tried first
pipeline_depth = 10

[[servers]]
host = "news.backup.com"
port = 119
tls = false
connections = 5
priority = 10  # Higher = backup server
pipeline_depth = 10
```

### JSON
```json
{
  "servers": [
    {
      "host": "news.primary.com",
      "port": 563,
      "tls": true,
      "username": "myuser",
      "password": "mypass",
      "connections": 10,
      "priority": 0,
      "pipeline_depth": 10
    },
    {
      "host": "news.backup.com",
      "port": 119,
      "tls": false,
      "connections": 5,
      "priority": 10,
      "pipeline_depth": 10
    }
  ]
}
```

### Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `host` | String | Yes | - | Server hostname or IP address |
| `port` | Integer | Yes | - | Server port (119 for plain, 563 for TLS) |
| `tls` | Boolean | Yes | - | Enable implicit TLS (not STARTTLS) |
| `username` | String | No | `null` | Authentication username |
| `password` | String | No | `null` | Authentication password |
| `connections` | Integer | No | `10` | Number of concurrent connections to maintain |
| `priority` | Integer | No | `0` | Server priority (lower values tried first, use for backups) |
| `pipeline_depth` | Integer | No | `10` | Number of pipelined NNTP commands per connection |

---

## RetryConfig

Configuration for automatic retry on transient failures (network timeouts, server busy, etc.).

### TOML
```toml
[retry]
max_attempts = 5
initial_delay = 1        # Seconds
max_delay = 60           # Seconds
backoff_multiplier = 2.0
jitter = true
```

### JSON
```json
{
  "retry": {
    "max_attempts": 5,
    "initial_delay": 1,
    "max_delay": 60,
    "backoff_multiplier": 2.0,
    "jitter": true
  }
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_attempts` | Integer | `5` | Maximum number of retry attempts |
| `initial_delay` | Integer (seconds) | `1` | Initial delay before first retry |
| `max_delay` | Integer (seconds) | `60` | Maximum delay between retries |
| `backoff_multiplier` | Float | `2.0` | Multiplier for exponential backoff (1s, 2s, 4s, 8s, ...) |
| `jitter` | Boolean | `true` | Add random jitter to delays (prevents thundering herd) |

---

## PostProcess Mode

Post-processing mode controls what happens after download completes. Default: `"unpack_and_cleanup"`

### Values

| Value | Description |
|-------|-------------|
| `"none"` | Just download, no post-processing |
| `"verify"` | Download + PAR2 verify |
| `"repair"` | Download + PAR2 verify/repair |
| `"unpack"` | Above + extract archives |
| `"unpack_and_cleanup"` | Above + remove intermediate files (default) |

### Example

```toml
default_post_process = "unpack_and_cleanup"
```

```json
{
  "default_post_process": "unpack_and_cleanup"
}
```

---

## ExtractionConfig

Archive extraction settings, including nested archive handling.

### TOML
```toml
[extraction]
max_recursion_depth = 2
archive_extensions = ["rar", "zip", "7z", "tar", "gz", "bz2"]
```

### JSON
```json
{
  "extraction": {
    "max_recursion_depth": 2,
    "archive_extensions": ["rar", "zip", "7z", "tar", "gz", "bz2"]
  }
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_recursion_depth` | Integer | `2` | Maximum depth for nested archive extraction (0 = only outer archives) |
| `archive_extensions` | Array of strings | `["rar", "zip", "7z", "tar", "gz", "bz2"]` | File extensions to treat as archives for recursion |

---

## PAR2 Configuration

Configure PAR2 verification and repair capabilities. By default, usenet-dl searches the system PATH for the `par2` binary. You can override this with explicit paths or disable PATH searching.

### Automatic Detection (Default)

```toml
# No configuration needed - will search PATH automatically
search_path = true  # This is the default
```

```json
{
  "search_path": true
}
```

When `search_path` is `true` (default) and `par2_path` is not set, usenet-dl will:
1. Search for `par2` in the system PATH
2. Use it for verification and repair if found
3. Fall back to verification-only mode if not found (no repair capability)

### Explicit Path

For Tauri apps bundling binaries as sidecars, or when `par2` is in a non-standard location:

```toml
par2_path = "/usr/local/bin/par2"
# Or for Tauri:
# par2_path = "/path/to/sidecar/binaries/par2"
```

```json
{
  "par2_path": "/usr/local/bin/par2"
}
```

### Disable PATH Search

To only use explicitly configured binaries (useful for sandboxed environments):

```toml
search_path = false
par2_path = "/opt/binaries/par2"
```

```json
{
  "search_path": false,
  "par2_path": "/opt/binaries/par2"
}
```

### Capabilities

PAR2 handler capabilities depend on configuration:

| Configuration | Verification | Repair | Handler |
|---------------|--------------|--------|---------|
| `par2` in PATH or `par2_path` set | ✅ Full | ✅ Full | `cli-par2` |
| No `par2` binary available | ✅ Basic* | ❌ Not supported | `noop` |

\* Basic verification assumes files are intact when PAR2 binary is unavailable. Full verification requires the `par2` binary.

### Query Capabilities

Use the `/api/v1/capabilities` endpoint to check current capabilities:

```bash
curl http://localhost:8080/api/v1/capabilities
```

Response:
```json
{
  "parity": {
    "can_verify": true,
    "can_repair": true,
    "handler": "cli-par2"
  }
}
```

Or query from Rust:

```rust
let downloader = UsenetDownloader::new(config).await?;
let caps = downloader.capabilities();
println!("PAR2 repair available: {}", caps.parity.can_repair);
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `par2_path` | String (path, optional) | `null` | Explicit path to par2 binary. If set, this path is used instead of searching PATH. |
| `search_path` | Boolean | `true` | Whether to search system PATH for binaries when explicit paths are not set. Set to `false` for sandboxed or controlled environments. |

---

## FileCollisionAction

How to handle filename collisions at destination. Default: `"rename"`

### Values

| Value | Description |
|-------|-------------|
| `"rename"` | Append (1), (2), etc. to filename (default - never lose data) |
| `"overwrite"` | Overwrite existing file |
| `"skip"` | Skip file, keep existing |

### Example

```toml
file_collision = "rename"
```

```json
{
  "file_collision": "rename"
}
```

---

## DeobfuscationConfig

Automatic detection and renaming of obfuscated (random) filenames.

### TOML
```toml
[deobfuscation]
enabled = true
min_length = 12
```

### JSON
```json
{
  "deobfuscation": {
    "enabled": true,
    "min_length": 12
  }
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | Boolean | `true` | Enable automatic deobfuscation |
| `min_length` | Integer | `12` | Minimum filename length to consider for deobfuscation |

---

## DuplicateConfig

Duplicate detection prevents re-downloading the same content.

### TOML
```toml
[duplicate]
enabled = true
action = "warn"
methods = ["nzb_hash", "job_name"]
```

### JSON
```json
{
  "duplicate": {
    "enabled": true,
    "action": "warn",
    "methods": ["nzb_hash", "job_name"]
  }
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | Boolean | `true` | Enable duplicate detection |
| `action` | String | `"warn"` | What to do when duplicate detected: `"block"`, `"warn"`, or `"allow"` |
| `methods` | Array of strings | `["nzb_hash", "job_name"]` | Detection methods (checked in order) |

### Action Values

| Value | Description |
|-------|-------------|
| `"block"` | Block the download entirely |
| `"warn"` | Allow but emit warning event (default) |
| `"allow"` | Allow silently |

### Method Values

| Value | Description |
|-------|-------------|
| `"nzb_hash"` | NZB content hash (most reliable) |
| `"nzb_name"` | NZB filename |
| `"job_name"` | Extracted job name (catches renamed NZBs) |

---

## DiskSpaceConfig

Pre-download disk space validation to prevent failed extractions.

### TOML
```toml
[disk_space]
enabled = true
min_free_space = 1073741824  # 1 GB in bytes
size_multiplier = 2.5
```

### JSON
```json
{
  "disk_space": {
    "enabled": true,
    "min_free_space": 1073741824,
    "size_multiplier": 2.5
  }
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | Boolean | `true` | Enable disk space checking |
| `min_free_space` | Integer (bytes) | `1073741824` (1 GB) | Minimum free space to maintain |
| `size_multiplier` | Float | `2.5` | Multiplier for download size (accounts for extraction: compressed + extracted + headroom) |

---

## DirectUnpackConfig

Extract archives while downloads are still in progress. When enabled and post-processing includes `unpack` or `unpack_and_cleanup`, a background coordinator polls for completed files and extracts RAR archives as they finish downloading. If all articles succeed and extraction completes, the post-processing pipeline skips verify/repair/extract and runs only move + cleanup.

### TOML
```toml
[direct_unpack]
enabled = true
direct_rename = true
poll_interval_ms = 200
```

### JSON
```json
{
  "direct_unpack": {
    "enabled": true,
    "direct_rename": true,
    "poll_interval_ms": 200
  }
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | Boolean | `false` | Enable DirectUnpack (extract during download) |
| `direct_rename` | Boolean | `false` | Enable DirectRename (use PAR2 metadata to fix obfuscated filenames mid-download) |
| `poll_interval_ms` | Integer (milliseconds) | `200` | How often to poll for newly completed files |

### Behavior

- **Zero-tolerance for failures**: If any article download fails, DirectUnpack cancels immediately and the normal post-processing pipeline (verify/repair/extract) runs instead
- **DirectRename**: When enabled, PAR2 files are prioritized for early download. Once a PAR2 file completes, its metadata maps 16KB MD5 hashes to real filenames. Files are renamed as they complete, before DirectUnpack processes them
- **Cancellation token**: DirectUnpack respects download pause/cancel operations
- **Post-processing shortcut**: If DirectUnpack succeeds with zero failures, only move + cleanup stages run

---

## CleanupConfig

Automatic cleanup of intermediate files after successful extraction.

### TOML
```toml
[cleanup]
enabled = true
target_extensions = ["par2", "PAR2", "nzb", "NZB", "sfv", "SFV", "srr", "SRR", "nfo", "NFO"]
archive_extensions = ["rar", "zip", "7z", "tar", "gz", "bz2"]
delete_samples = true
sample_folder_names = ["sample", "Sample", "SAMPLE", "samples", "Samples", "SAMPLES"]
```

### JSON
```json
{
  "cleanup": {
    "enabled": true,
    "target_extensions": ["par2", "PAR2", "nzb", "NZB", "sfv", "SFV", "srr", "SRR", "nfo", "NFO"],
    "archive_extensions": ["rar", "zip", "7z", "tar", "gz", "bz2"],
    "delete_samples": true,
    "sample_folder_names": ["sample", "Sample", "SAMPLE", "samples", "Samples", "SAMPLES"]
  }
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | Boolean | `true` | Enable cleanup of intermediate files |
| `target_extensions` | Array of strings | See above | File extensions to remove |
| `archive_extensions` | Array of strings | `["rar", "zip", "7z", "tar", "gz", "bz2"]` | Archive extensions to remove after extraction |
| `delete_samples` | Boolean | `true` | Delete sample folders |
| `sample_folder_names` | Array of strings | `["sample", "Sample", "SAMPLE", ...]` | Sample folder names (case-sensitive match) |

---

## ApiConfig

REST API server configuration.

### TOML
```toml
[api]
bind_address = "127.0.0.1:6789"
api_key = "secret123"  # Optional authentication
cors_enabled = true
cors_origins = ["*"]
swagger_ui = true

[api.rate_limit]
enabled = false
requests_per_second = 100
burst_size = 200
exempt_paths = ["/api/v1/events", "/api/v1/health"]
exempt_ips = ["127.0.0.1", "::1"]
```

### JSON
```json
{
  "api": {
    "bind_address": "127.0.0.1:6789",
    "api_key": "secret123",
    "cors_enabled": true,
    "cors_origins": ["*"],
    "swagger_ui": true,
    "rate_limit": {
      "enabled": false,
      "requests_per_second": 100,
      "burst_size": 200,
      "exempt_paths": ["/api/v1/events", "/api/v1/health"],
      "exempt_ips": ["127.0.0.1", "::1"]
    }
  }
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `bind_address` | String | `"127.0.0.1:6789"` | Address to bind API server (localhost only for security) |
| `api_key` | String (optional) | `null` | Optional API key for authentication (sent as `X-Api-Key` header) |
| `cors_enabled` | Boolean | `true` | Enable CORS for browser access |
| `cors_origins` | Array of strings | `["*"]` | Allowed CORS origins |
| `swagger_ui` | Boolean | `true` | Enable Swagger UI at `/swagger-ui` |
| `rate_limit` | `RateLimitConfig` | See below | Rate limiting configuration |

### RateLimitConfig Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | Boolean | `false` | Enable rate limiting (disabled by default - trust local network) |
| `requests_per_second` | Integer | `100` | Requests per second per IP |
| `burst_size` | Integer | `200` | Burst size (allows UI refresh bursts) |
| `exempt_paths` | Array of strings | `["/api/v1/events", "/api/v1/health"]` | Endpoints exempt from rate limiting |
| `exempt_ips` | Array of strings | `["127.0.0.1", "::1"]` | IP addresses exempt from rate limiting |

---

## ScheduleRule

Time-based rules for speed limits or pausing (e.g., unlimited at night, limited during work hours).

### TOML
```toml
[[schedule_rules]]
name = "Night owl"
days = []  # Empty = all days
start_time = "00:00"
end_time = "06:00"
enabled = true

[schedule_rules.action]
type = "unlimited"

[[schedule_rules]]
name = "Work hours"
days = ["monday", "tuesday", "wednesday", "thursday", "friday"]
start_time = "09:00"
end_time = "17:00"
enabled = true

[schedule_rules.action]
type = "speed_limit"
limit_bps = 1000000  # 1 MB/s
```

### JSON
```json
{
  "schedule_rules": [
    {
      "name": "Night owl",
      "days": [],
      "start_time": "00:00",
      "end_time": "06:00",
      "action": {
        "type": "unlimited"
      },
      "enabled": true
    },
    {
      "name": "Work hours",
      "days": ["monday", "tuesday", "wednesday", "thursday", "friday"],
      "start_time": "09:00",
      "end_time": "17:00",
      "action": {
        "type": "speed_limit",
        "limit_bps": 1000000
      },
      "enabled": true
    }
  ]
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | String | Required | Human-readable name for the rule |
| `days` | Array of strings | `[]` (all days) | Days this rule applies: `"monday"`, `"tuesday"`, etc. (empty = all days) |
| `start_time` | String | Required | Start time in HH:MM format (24-hour) |
| `end_time` | String | Required | End time in HH:MM format (24-hour) |
| `action` | `ScheduleAction` | Required | Action to take during this window |
| `enabled` | Boolean | `true` | Whether rule is active |

### ScheduleAction Types

| Type | Fields | Description |
|------|--------|-------------|
| `"speed_limit"` | `limit_bps` (integer) | Set speed limit in bytes per second |
| `"unlimited"` | None | Unlimited speed |
| `"pause"` | None | Pause all downloads |

---

## WatchFolderConfig

Automatic NZB import from monitored folders.

### TOML
```toml
[[watch_folders]]
path = "/path/to/nzb/folder"
after_import = "move_to_processed"
category = "movies"
scan_interval = 5  # Seconds
```

### JSON
```json
{
  "watch_folders": [
    {
      "path": "/path/to/nzb/folder",
      "after_import": "move_to_processed",
      "category": "movies",
      "scan_interval": 5
    }
  ]
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `path` | String (path) | Required | Directory to watch for NZB files |
| `after_import` | String | `"move_to_processed"` | What to do with NZB after import: `"delete"`, `"move_to_processed"`, or `"keep"` |
| `category` | String (optional) | `null` | Category to assign (null = use default) |
| `scan_interval` | Integer (seconds) | `5` | How often to scan for new files |

---

## RssFeedConfig

Automatic NZB discovery from RSS feeds.

### TOML
```toml
[[rss_feeds]]
url = "https://indexer.example.com/rss"
check_interval = 900  # 15 minutes in seconds
category = "tv"
auto_download = true
priority = "normal"
enabled = true

[[rss_feeds.filters]]
name = "HD TV Shows"
include = ["1080p", "720p"]
exclude = ["CAM", "TS"]
min_size = 1073741824  # 1 GB
max_size = 10737418240  # 10 GB
# max_age = 86400  # 24 hours (optional)
```

### JSON
```json
{
  "rss_feeds": [
    {
      "url": "https://indexer.example.com/rss",
      "check_interval": 900,
      "category": "tv",
      "filters": [
        {
          "name": "HD TV Shows",
          "include": ["1080p", "720p"],
          "exclude": ["CAM", "TS"],
          "min_size": 1073741824,
          "max_size": 10737418240,
          "max_age": null
        }
      ],
      "auto_download": true,
      "priority": "normal",
      "enabled": true
    }
  ]
}
```

### RssFeedConfig Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `url` | String | Required | Feed URL (RSS or Atom format supported) |
| `check_interval` | Integer (seconds) | `900` (15 min) | How often to check feed |
| `category` | String (optional) | `null` | Category to assign to downloads |
| `filters` | Array of `RssFilter` | `[]` | Only download items matching filters |
| `auto_download` | Boolean | `true` | Automatically download matches vs just notify |
| `priority` | String | `"normal"` | Priority for auto-downloaded items: `"low"`, `"normal"`, `"high"`, or `"force"` |
| `enabled` | Boolean | `true` | Whether feed is active |

### RssFilter Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | String | Required | Filter name (for UI/logging) |
| `include` | Array of strings | `[]` | Patterns to include (regex) |
| `exclude` | Array of strings | `[]` | Patterns to exclude (regex) |
| `min_size` | Integer (bytes, optional) | `null` | Minimum size |
| `max_size` | Integer (bytes, optional) | `null` | Maximum size |
| `max_age` | Integer (seconds, optional) | `null` | Maximum age from publish date |

---

## WebhookConfig

HTTP webhooks for external notifications.

### TOML
```toml
[[webhooks]]
url = "https://example.com/webhook"
events = ["on_complete", "on_failed"]
auth_header = "Bearer secret-token"
timeout = 30  # Seconds
```

### JSON
```json
{
  "webhooks": [
    {
      "url": "https://example.com/webhook",
      "events": ["on_complete", "on_failed"],
      "auth_header": "Bearer secret-token",
      "timeout": 30
    }
  ]
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `url` | String | Required | URL to POST webhook payload to |
| `events` | Array of strings | Required | Events that trigger this webhook: `"on_complete"`, `"on_failed"`, `"on_queued"` |
| `auth_header` | String (optional) | `null` | Optional authentication header value (sent as `Authorization` header) |
| `timeout` | Integer (seconds) | `30` | Timeout for webhook requests |

---

## ScriptConfig

External script execution on events.

### TOML
```toml
[[scripts]]
path = "/usr/local/bin/notify.sh"
events = ["on_complete", "on_failed"]
timeout = 300  # 5 minutes
```

### JSON
```json
{
  "scripts": [
    {
      "path": "/usr/local/bin/notify.sh",
      "events": ["on_complete", "on_failed"],
      "timeout": 300
    }
  ]
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `path` | String (path) | Required | Path to script/executable |
| `events` | Array of strings | Required | Events that trigger this script: `"on_complete"`, `"on_failed"`, `"on_post_process_complete"` |
| `timeout` | Integer (seconds) | `300` (5 min) | Timeout for script execution |

### Script Environment Variables

Scripts receive these environment variables:

| Variable | Description |
|----------|-------------|
| `USENET_DL_ID` | Download ID |
| `USENET_DL_NAME` | Download name |
| `USENET_DL_CATEGORY` | Category |
| `USENET_DL_STATUS` | Status (complete/failed) |
| `USENET_DL_DESTINATION` | Final destination path |
| `USENET_DL_ERROR` | Error message (if failed) |
| `USENET_DL_SIZE` | Total size in bytes |
| `USENET_DL_IS_CATEGORY_SCRIPT` | "true" if category script |
| `USENET_DL_CATEGORY_DESTINATION` | Category destination path (if category script) |

---

## CategoryConfig

Category-specific settings override global defaults.

### TOML
```toml
[persistence.categories.movies]
destination = "/media/movies"
post_process = "unpack_and_cleanup"

[[persistence.categories.movies.scripts]]
path = "/usr/local/bin/movie-indexer.sh"
events = ["on_complete"]
timeout = 60

[persistence.categories.tv]
destination = "/media/tv"
```

### JSON
```json
{
  "persistence": {
    "categories": {
      "movies": {
        "destination": "/media/movies",
        "post_process": "unpack_and_cleanup",
        "scripts": [
          {
            "path": "/usr/local/bin/movie-indexer.sh",
            "events": ["on_complete"],
            "timeout": 60
          }
        ]
      },
      "tv": {
        "destination": "/media/tv",
        "post_process": null,
        "scripts": []
      }
    }
  }
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `destination` | String (path) | Required | Destination directory for this category |
| `post_process` | String (optional) | `null` | Override default post-processing mode (null = use global default) |
| `scripts` | Array of `ScriptConfig` | `[]` | Category-specific scripts (run before global scripts) |

---

## Complete Example Configuration

### TOML (config.toml)

```toml
# NNTP Servers
[[servers]]
host = "news.primary.com"
port = 563
tls = true
username = "user"
password = "pass"
connections = 10
priority = 0
pipeline_depth = 10

[[servers]]
host = "news.backup.com"
port = 119
tls = false
connections = 5
priority = 10
pipeline_depth = 10

# Directories
download_dir = "/downloads"
temp_dir = "/temp"

# Download Settings
max_concurrent_downloads = 3
speed_limit_bps = null  # Unlimited

# Retry Configuration
[retry]
max_attempts = 5
initial_delay = 1
max_delay = 60
backoff_multiplier = 2.0
jitter = true

# Post-Processing
default_post_process = "unpack_and_cleanup"
delete_samples = true

# Extraction
[extraction]
max_recursion_depth = 2
archive_extensions = ["rar", "zip", "7z"]

# File Handling
file_collision = "rename"

[deobfuscation]
enabled = true
min_length = 12

# Duplicate Detection
[duplicate]
enabled = true
action = "warn"
methods = ["nzb_hash", "job_name"]

# Disk Space
[disk_space]
enabled = true
min_free_space = 1073741824  # 1 GB
size_multiplier = 2.5

# Cleanup
[cleanup]
enabled = true
target_extensions = ["par2", "nzb", "sfv"]
archive_extensions = ["rar", "zip", "7z"]
delete_samples = true

# DirectUnpack (extract during download)
[direct_unpack]
enabled = false
direct_rename = false
poll_interval_ms = 200

# Passwords
password_file = "/config/passwords.txt"
try_empty_password = true

# Persistence (not flattened — fields are nested)
[persistence]
database_path = "/config/usenet-dl.db"

[persistence.categories.movies]
destination = "/media/movies"
post_process = "unpack_and_cleanup"

[persistence.categories.tv]
destination = "/media/tv"

# REST API
[api]
bind_address = "0.0.0.0:6789"
api_key = "secret123"
cors_enabled = true
swagger_ui = true

[api.rate_limit]
enabled = false

# Schedule: Unlimited at night
[[schedule_rules]]
name = "Night unlimited"
days = []
start_time = "00:00"
end_time = "06:00"
enabled = true

[schedule_rules.action]
type = "unlimited"

# Schedule: Limited during work hours
[[schedule_rules]]
name = "Work hours"
days = ["monday", "tuesday", "wednesday", "thursday", "friday"]
start_time = "09:00"
end_time = "17:00"
enabled = true

[schedule_rules.action]
type = "speed_limit"
limit_bps = 1000000

# Watch Folder
[[watch_folders]]
path = "/nzb"
after_import = "move_to_processed"
scan_interval = 5

# RSS Feed
[[rss_feeds]]
url = "https://indexer.com/rss"
check_interval = 900
category = "tv"
auto_download = true
priority = "normal"
enabled = true

[[rss_feeds.filters]]
name = "HD Shows"
include = ["1080p", "720p"]
exclude = ["CAM"]
min_size = 1073741824

# Webhook
[[webhooks]]
url = "https://example.com/webhook"
events = ["on_complete"]
auth_header = "Bearer token"
timeout = 30

# Script
[[scripts]]
path = "/scripts/notify.sh"
events = ["on_complete", "on_failed"]
timeout = 300
```

### JSON (config.json)

```json
{
  "servers": [
    {
      "host": "news.primary.com",
      "port": 563,
      "tls": true,
      "username": "user",
      "password": "pass",
      "connections": 10,
      "priority": 0,
      "pipeline_depth": 10
    }
  ],
  "download_dir": "/downloads",
  "temp_dir": "/temp",
  "max_concurrent_downloads": 3,
  "speed_limit_bps": null,
  "retry": {
    "max_attempts": 5,
    "initial_delay": 1,
    "max_delay": 60,
    "backoff_multiplier": 2.0,
    "jitter": true
  },
  "default_post_process": "unpack_and_cleanup",
  "delete_samples": true,
  "extraction": {
    "max_recursion_depth": 2,
    "archive_extensions": ["rar", "zip", "7z"]
  },
  "file_collision": "rename",
  "deobfuscation": {
    "enabled": true,
    "min_length": 12
  },
  "duplicate": {
    "enabled": true,
    "action": "warn",
    "methods": ["nzb_hash", "job_name"]
  },
  "disk_space": {
    "enabled": true,
    "min_free_space": 1073741824,
    "size_multiplier": 2.5
  },
  "cleanup": {
    "enabled": true,
    "target_extensions": ["par2", "nzb", "sfv"],
    "archive_extensions": ["rar", "zip", "7z"],
    "delete_samples": true
  },
  "direct_unpack": {
    "enabled": false,
    "direct_rename": false,
    "poll_interval_ms": 200
  },
  "password_file": "/config/passwords.txt",
  "try_empty_password": true,
  "persistence": {
    "database_path": "/config/usenet-dl.db",
    "schedule_rules": [],
    "categories": {
      "movies": {
        "destination": "/media/movies",
        "post_process": "unpack_and_cleanup",
        "scripts": []
      },
      "tv": {
        "destination": "/media/tv",
        "post_process": null,
        "scripts": []
      }
    }
  },
  "api": {
    "bind_address": "0.0.0.0:6789",
    "api_key": "secret123",
    "cors_enabled": true,
    "swagger_ui": true,
    "rate_limit": {
      "enabled": false
    }
  },
  "schedule_rules": [
    {
      "name": "Night unlimited",
      "days": [],
      "start_time": "00:00",
      "end_time": "06:00",
      "action": {
        "type": "unlimited"
      },
      "enabled": true
    }
  ],
  "watch_folders": [
    {
      "path": "/nzb",
      "after_import": "move_to_processed",
      "scan_interval": 5
    }
  ],
  "rss_feeds": [
    {
      "url": "https://indexer.com/rss",
      "check_interval": 900,
      "category": "tv",
      "filters": [
        {
          "name": "HD Shows",
          "include": ["1080p", "720p"],
          "exclude": ["CAM"],
          "min_size": 1073741824,
          "max_size": null,
          "max_age": null
        }
      ],
      "auto_download": true,
      "priority": "normal",
      "enabled": true
    }
  ],
  "webhooks": [
    {
      "url": "https://example.com/webhook",
      "events": ["on_complete"],
      "auth_header": "Bearer token",
      "timeout": 30
    }
  ],
  "scripts": [
    {
      "path": "/scripts/notify.sh",
      "events": ["on_complete", "on_failed"],
      "timeout": 300
    }
  ]
}
```

---

## Configuration Notes

### Data Type Conversions

- **Durations**: All durations are specified in **seconds** as integers (e.g., `timeout = 30` for 30 seconds)
- **Sizes**: All sizes are in **bytes** as integers (e.g., `min_free_space = 1073741824` for 1 GB)
- **Enums**: All enum values use **snake_case** strings (e.g., `"unpack_and_cleanup"`, `"move_to_processed"`)
- **IPs**: IP addresses are strings (e.g., `"127.0.0.1"`, `"::1"`)

### Security Considerations

- **API Key**: Set `api.api_key` if exposing API beyond localhost
- **Bind Address**: Default `127.0.0.1:6789` only accepts local connections. Use `0.0.0.0:6789` to accept remote connections (ensure firewall is configured)
- **Passwords**: Stored in plain text in config file - ensure file permissions are restrictive (e.g., `chmod 600 config.toml`)
- **Webhook Auth**: Use `auth_header` for webhook authentication

### Path Handling

- **Relative paths**: Interpreted relative to current working directory
- **Absolute paths**: Recommended for production deployments
- **Windows paths**: Use forward slashes or escaped backslashes in JSON (e.g., `"C:/downloads"` or `"C:\\downloads"`)

### Minimal vs Complete Configuration

You only need to specify settings that differ from defaults. A minimal production configuration might be:

```toml
[[servers]]
host = "news.example.com"
port = 563
tls = true
username = "myuser"
password = "mypass"

download_dir = "/media/downloads"

[persistence.categories.movies]
destination = "/media/movies"

[persistence.categories.tv]
destination = "/media/tv"
```

All other settings will use their defaults.
