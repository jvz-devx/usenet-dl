# usenet-dl Implementation Design v1

## Design Philosophy

**usenet-dl is a backend library for SABnzbd/NZBGet-like applications.**

It is designed to be:
- **Highly configurable** - Almost every behavior can be customized
- **Sensible defaults** - Works out of the box with zero configuration
- **Library-first** - No CLI or UI, purely a Rust crate for embedding
- **Event-driven** - Consumers subscribe to events, no polling required

The goal is to provide a solid foundation that frontend applications (web UI, desktop app, Spotnet integration) can build upon without reimplementing download management logic.

---

## Defaults Summary

Everything works out of the box. Users only need to configure their NNTP server(s).

| Setting | Default | Rationale |
|---------|---------|-----------|
| **Download directory** | `./downloads` | Current directory, easy to find |
| **Temp directory** | `./temp` | Separate from final downloads |
| **Concurrent downloads** | 3 | Balanced throughput without overwhelming |
| **Speed limit** | Unlimited | Users expect full speed by default |
| **Post-processing** | Unpack + Cleanup | Most users want ready-to-use files |
| **Failed download action** | Keep files | Don't delete potentially recoverable data |
| **File collision** | Rename (add number) | Never lose data silently |
| **Nested extraction depth** | 2 levels | Handle common archive-in-archive |
| **Deobfuscation** | Enabled | Most users want readable names |
| **Duplicate detection** | Warn only | Alert but don't block |
| **Try empty password** | Yes | Common for public releases |
| **Delete samples** | Yes | Usually unwanted |
| **Disk space check** | Enabled, 1GB buffer | Prevent failed extractions |
| **Retry attempts** | 5 with exponential backoff | Resilient to transient failures |
| **API bind address** | 127.0.0.1:6789 | Localhost only for security |
| **API authentication** | None (localhost) | Easy local development |
| **CORS** | Enabled for all origins | Easy frontend development |
| **Swagger UI** | Enabled | Self-documenting API |
| **Rate limiting** | Disabled | Trust local network |
| **Watch folder action** | Move to processed | Keep originals accessible |
| **RSS check interval** | 15 minutes | Balance freshness vs load |
| **Script execution** | Async (non-blocking) | Don't slow down pipeline |
| **Graceful shutdown timeout** | 30 seconds | Complete current work |

---

## Architecture Overview

```
┌─────────────────────────────────────────┐
│  Spotnet App    │  SABnzbd Alternative  │
├─────────────────┴───────────────────────┤
│              usenet-dl                  │
├─────────────────────────────────────────┤
│              nntp-rs                    │
└─────────────────────────────────────────┘
```

### Responsibility Split

**usenet-dl handles:**
- Download queue management
- Post-processing pipeline
- RAR/7z/ZIP extraction
- File renaming and organization
- Cleanup (remove .par2, .nzb, samples)
- Progress and event callbacks
- Password management for archives
- Persistence (SQLite)
- NZB folder watching and URL fetching
- External notifications (webhooks, scripts)

**nntp-rs handles:**
- NNTP protocol
- NZB parsing
- yEnc decoding
- PAR2 verification and repair
- Connection pooling

---

## Event System

### Design Choice: `tokio::broadcast` Channel

Using broadcast channels instead of closures for event delivery:

**Rationale:**
- Multiple subscribers (UI, logging, notifications) can listen independently
- No lifetime complexity with closures
- Async-friendly
- Subscribers can join/leave dynamically

### Event Types

```rust
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub enum Event {
    // Queue events
    Queued { id: DownloadId, name: String },
    Removed { id: DownloadId },

    // Download progress
    Downloading { id: DownloadId, percent: f32, speed_bps: u64 },
    DownloadComplete { id: DownloadId },
    DownloadFailed { id: DownloadId, error: String },

    // Post-processing stages
    Verifying { id: DownloadId },
    VerifyComplete { id: DownloadId, damaged: bool },
    Repairing { id: DownloadId, blocks_needed: u32, blocks_available: u32 },
    RepairComplete { id: DownloadId, success: bool },
    Extracting { id: DownloadId, archive: String, percent: f32 },
    ExtractComplete { id: DownloadId },
    Moving { id: DownloadId, destination: PathBuf },
    Cleaning { id: DownloadId },

    // Final states
    Complete { id: DownloadId, path: PathBuf },
    Failed { id: DownloadId, stage: Stage, error: String, files_kept: bool },

    // Global events
    SpeedLimitChanged { limit_bps: Option<u64> },
    QueuePaused,
    QueueResumed,

    // Notifications
    WebhookFailed { url: String, error: String },
    ScriptFailed { script: PathBuf, exit_code: Option<i32> },
}

#[derive(Clone, Debug)]
pub enum Stage {
    Download,
    Verify,
    Repair,
    Extract,
    Move,
    Cleanup,
}

pub struct UsenetDownloader {
    event_tx: broadcast::Sender<Event>,
    // ...
}

impl UsenetDownloader {
    /// Subscribe to events. Multiple subscribers supported.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.event_tx.subscribe()
    }
}
```

### Usage Example

```rust
let downloader = UsenetDownloader::new(config).await?;

// UI subscriber
let mut ui_events = downloader.subscribe();
tokio::spawn(async move {
    while let Ok(event) = ui_events.recv().await {
        update_ui(event);
    }
});

// Logging subscriber
let mut log_events = downloader.subscribe();
tokio::spawn(async move {
    while let Ok(event) = log_events.recv().await {
        tracing::info!(?event, "download event");
    }
});
```

---

## Retry Logic

### Design: Configurable Exponential Backoff

Transient failures (network timeouts, server busy, connection reset) are retried automatically.

### Configuration

```rust
pub struct RetryConfig {
    /// Maximum number of retry attempts (default: 5)
    pub max_attempts: u32,

    /// Initial delay before first retry (default: 1 second)
    pub initial_delay: Duration,

    /// Maximum delay between retries (default: 60 seconds)
    pub max_delay: Duration,

    /// Multiplier for exponential backoff (default: 2.0)
    pub backoff_multiplier: f64,

    /// Add random jitter to delays (default: true)
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,           // Enough for transient issues
            initial_delay: Duration::from_secs(1),  // Quick first retry
            max_delay: Duration::from_secs(60),     // Cap wait time
            backoff_multiplier: 2.0,   // 1s, 2s, 4s, 8s, 16s...
            jitter: true,              // Prevent thundering herd
        }
    }
}
```

### Retry Behavior

| Error Type | Retryable | Notes |
|------------|-----------|-------|
| Network timeout | Yes | Exponential backoff |
| Connection refused | Yes | Server may be restarting |
| Server busy (400) | Yes | With longer initial delay |
| Authentication failed | No | Fail immediately |
| Article not found (430) | No | Try backup server if configured |
| Disk full | No | Fail immediately |
| Corrupt data | No | Fail immediately |

### Implementation

```rust
async fn download_with_retry<F, T, E>(
    &self,
    operation: F,
    config: &RetryConfig,
) -> Result<T, E>
where
    F: Fn() -> Future<Output = Result<T, E>>,
    E: IsRetryable,
{
    let mut attempt = 0;
    let mut delay = config.initial_delay;

    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) if e.is_retryable() && attempt < config.max_attempts => {
                attempt += 1;
                let jittered_delay = if config.jitter {
                    add_jitter(delay)
                } else {
                    delay
                };
                tokio::time::sleep(jittered_delay).await;
                delay = (delay.mul_f64(config.backoff_multiplier)).min(config.max_delay);
            }
            Err(e) => return Err(e),
        }
    }
}
```

---

## File Collision Handling

### Design: Configurable Action

When extracting or moving files would overwrite existing files.

### Configuration

```rust
#[derive(Clone, Copy, Debug, Default)]
pub enum FileCollisionAction {
    /// Append (1), (2), etc. to filename (default)
    #[default]
    Rename,
    /// Overwrite existing file
    Overwrite,
    /// Skip the file, keep existing
    Skip,
}

pub struct Config {
    // ...

    /// How to handle file collisions at destination
    pub file_collision: FileCollisionAction,
}
```

### Rename Strategy

```rust
fn get_unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let stem = path.file_stem().unwrap_or_default();
    let ext = path.extension();
    let parent = path.parent().unwrap_or(Path::new("."));

    for i in 1.. {
        let new_name = match ext {
            Some(ext) => format!("{} ({}).{}", stem.to_string_lossy(), i, ext.to_string_lossy()),
            None => format!("{} ({})", stem.to_string_lossy(), i),
        };
        let new_path = parent.join(new_name);
        if !new_path.exists() {
            return new_path;
        }
    }
    unreachable!()
}
```

---

## NZB Sources

### Design: Multiple Acquisition Methods

NZB files can be added via API, folder watching, or URL fetching.

### Folder Watching

```rust
pub struct WatchFolderConfig {
    /// Directory to watch for NZB files
    pub path: PathBuf,

    /// What to do with NZB after adding to queue
    pub after_import: WatchFolderAction,

    /// Category to assign (None = use default)
    pub category: Option<String>,

    /// Scan interval (default: 5 seconds)
    pub scan_interval: Duration,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum WatchFolderAction {
    /// Delete NZB file after successfully adding to queue
    Delete,
    /// Move to a 'processed' subfolder
    #[default]
    MoveToProcessed,
    /// Keep in place, track to avoid re-adding
    Keep,
}
```

### URL Fetching

```rust
impl UsenetDownloader {
    /// Fetch NZB from URL and add to queue
    pub async fn add_nzb_url(
        &self,
        url: &str,
        options: DownloadOptions,
    ) -> Result<DownloadId> {
        let response = self.http_client.get(url).send().await?;
        let content = response.bytes().await?;
        let name = extract_filename_from_url(url);
        self.add_nzb_content(&content, &name, options).await
    }
}
```

### Category-Specific Watch Folders

```rust
pub struct CategoryConfig {
    pub destination: PathBuf,
    pub post_process: Option<PostProcess>,

    /// Watch folder specific to this category
    pub watch_folder: Option<WatchFolderConfig>,
}
```

---

## Download Resume

### Design: Article-Level Progress Tracking

Downloads are resumable after crash or restart with article-level granularity.

### Database Schema Addition

```sql
-- Track individual article download status
CREATE TABLE download_articles (
    id INTEGER PRIMARY KEY,
    download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    segment_number INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL,
    status INTEGER NOT NULL DEFAULT 0,  -- 0=pending, 1=downloaded, 2=failed
    downloaded_at INTEGER,
    UNIQUE(download_id, message_id)
);

CREATE INDEX idx_articles_download ON download_articles(download_id);
CREATE INDEX idx_articles_status ON download_articles(download_id, status);
```

### Resume Flow

```rust
impl UsenetDownloader {
    /// Resume a partially downloaded job
    async fn resume_download(&self, id: DownloadId) -> Result<()> {
        let pending_articles = self.db.get_pending_articles(id).await?;

        if pending_articles.is_empty() {
            // All articles downloaded, proceed to post-processing
            self.start_post_processing(id).await
        } else {
            // Resume downloading remaining articles
            self.download_articles(id, pending_articles).await
        }
    }

    /// On startup, restore in-progress downloads
    async fn restore_queue(&self) -> Result<()> {
        let downloads = self.db.get_incomplete_downloads().await?;
        for download in downloads {
            match download.status {
                Status::Downloading => self.resume_download(download.id).await?,
                Status::Processing => self.start_post_processing(download.id).await?,
                _ => {}
            }
        }
        Ok(())
    }
}
```

---

## Nested Archive Extraction

### Design: Configurable Recursion Depth

Archives inside archives can be extracted automatically.

### Configuration

```rust
pub struct ExtractionConfig {
    /// Maximum depth for nested archive extraction (default: 2)
    /// 0 = only extract outer archives
    /// 1 = extract archives found inside (one level)
    /// 2+ = continue recursively
    pub max_recursion_depth: u32,

    /// File extensions to treat as archives for recursion
    pub archive_extensions: Vec<String>,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            max_recursion_depth: 2,  // Handle archive-in-archive (common)
            archive_extensions: vec![
                "rar".into(), "zip".into(), "7z".into(),
                "tar".into(), "gz".into(), "bz2".into(),
            ],
        }
    }
}
```

### Extraction Flow

```rust
async fn extract_recursive(
    &self,
    path: &Path,
    depth: u32,
    config: &ExtractionConfig,
) -> Result<Vec<PathBuf>> {
    let extracted = self.extract_archive(path).await?;

    if depth >= config.max_recursion_depth {
        return Ok(extracted);
    }

    let mut all_files = extracted.clone();
    for file in &extracted {
        if is_archive(file, &config.archive_extensions) {
            let nested = self.extract_recursive(file, depth + 1, config).await?;
            all_files.extend(nested);
        }
    }

    Ok(all_files)
}
```

---

## Obfuscated Filename Handling

### Design: SABnzbd-Style Detection and Renaming

Usenet releases often use obfuscated (random) filenames. These are detected and renamed.

### Detection Heuristics

```rust
fn is_obfuscated(filename: &str) -> bool {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);

    // Check for common obfuscation patterns
    let checks = [
        // Mostly random alphanumeric (high entropy)
        is_high_entropy(stem),
        // UUID-like patterns
        looks_like_uuid(stem),
        // Pure hex strings
        is_hex_string(stem) && stem.len() > 16,
        // Random with no vowels (unlikely in real names)
        has_no_vowels(stem) && stem.len() > 8,
    ];

    checks.iter().any(|&c| c)
}
```

### Rename Sources (SABnzbd-style, priority order)

1. **Job name** - NZB filename without extension
2. **NZB meta title** - `<meta type="name">` from NZB
3. **Archive comment** - RAR/ZIP comment field
4. **Largest file** - Name of the largest extracted file (if not obfuscated)

### Implementation

```rust
pub struct DeobfuscationConfig {
    /// Enable automatic deobfuscation (default: true)
    pub enabled: bool,

    /// Minimum filename length to consider for deobfuscation (default: 12)
    pub min_length: usize,
}

impl UsenetDownloader {
    fn determine_final_name(&self, download: &Download, extracted: &[PathBuf]) -> String {
        // 1. Job name (NZB filename)
        let job_name = Path::new(&download.nzb_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&download.name);

        if !is_obfuscated(job_name) {
            return job_name.to_string();
        }

        // 2. NZB meta title
        if let Some(meta_name) = &download.nzb_meta_name {
            if !is_obfuscated(meta_name) {
                return meta_name.clone();
            }
        }

        // 3. Largest non-obfuscated file
        if let Some(largest) = find_largest_file(extracted) {
            let name = largest.file_stem().and_then(|s| s.to_str());
            if let Some(name) = name {
                if !is_obfuscated(name) {
                    return name.to_string();
                }
            }
        }

        // Fallback to job name even if obfuscated
        job_name.to_string()
    }
}
```

---

## External Notifications

### Design: Webhooks and Script Execution

Notify external systems on download events.

### Webhook Configuration

```rust
pub struct WebhookConfig {
    /// URL to POST to
    pub url: String,

    /// Events that trigger this webhook
    pub events: Vec<WebhookEvent>,

    /// Optional authentication header
    pub auth_header: Option<String>,

    /// Timeout for webhook requests (default: 30 seconds)
    pub timeout: Duration,
}

#[derive(Clone, Copy, Debug)]
pub enum WebhookEvent {
    OnComplete,
    OnFailed,
    OnQueued,
}
```

### Webhook Payload

```rust
#[derive(Serialize)]
struct WebhookPayload {
    event: String,
    download_id: i64,
    name: String,
    category: Option<String>,
    status: String,
    destination: Option<PathBuf>,
    error: Option<String>,
    timestamp: i64,
}
```

### Script Execution

```rust
pub struct ScriptConfig {
    /// Path to script/executable
    pub path: PathBuf,

    /// Events that trigger this script
    pub events: Vec<ScriptEvent>,

    /// Timeout for script execution (default: 5 minutes)
    pub timeout: Duration,
}

#[derive(Clone, Copy, Debug)]
pub enum ScriptEvent {
    OnComplete,
    OnFailed,
    OnPostProcessComplete,
}
```

### Script Environment Variables

Scripts receive context via environment variables (SABnzbd-compatible):

| Variable | Description |
|----------|-------------|
| `USENET_DL_ID` | Download ID |
| `USENET_DL_NAME` | Download name |
| `USENET_DL_CATEGORY` | Category |
| `USENET_DL_STATUS` | Status (complete/failed) |
| `USENET_DL_DESTINATION` | Final destination path |
| `USENET_DL_ERROR` | Error message (if failed) |
| `USENET_DL_SIZE` | Total size in bytes |

### Async Execution

```rust
impl UsenetDownloader {
    /// Execute notification scripts asynchronously (fire and forget)
    fn trigger_scripts(&self, event: ScriptEvent, download: &Download) {
        for script in &self.config.scripts {
            if script.events.contains(&event) {
                let script_path = script.path.clone();
                let timeout = script.timeout;
                let env_vars = build_env_vars(download);

                tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        timeout,
                        Command::new(&script_path)
                            .envs(env_vars)
                            .output()
                    ).await;

                    match result {
                        Ok(Ok(output)) if !output.status.success() => {
                            tracing::warn!(
                                script = ?script_path,
                                code = output.status.code(),
                                "notification script failed"
                            );
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(script = ?script_path, error = %e, "failed to run script");
                        }
                        Err(_) => {
                            tracing::warn!(script = ?script_path, "script timed out");
                        }
                        _ => {}
                    }
                });
            }
        }
    }
}
```

---

## Disk Space Checking

### Design: Pre-Download Validation

Check available disk space before starting downloads.

### Configuration

```rust
pub struct DiskSpaceConfig {
    /// Enable disk space checking (default: true)
    pub enabled: bool,

    /// Minimum free space to maintain (default: 1 GB)
    pub min_free_space: u64,

    /// Multiplier for estimated size to account for extraction overhead (default: 2.5)
    /// A 1GB download might need 2.5GB during extraction (compressed + extracted)
    pub size_multiplier: f64,
}

impl Default for DiskSpaceConfig {
    fn default() -> Self {
        Self {
            enabled: true,                       // Prevent failed extractions
            min_free_space: 1024 * 1024 * 1024,  // 1 GB buffer
            size_multiplier: 2.5,                // Compressed + extracted + headroom
        }
    }
}
```

### Implementation

```rust
impl UsenetDownloader {
    async fn check_disk_space(&self, download: &Download) -> Result<(), DiskSpaceError> {
        if !self.config.disk_space.enabled {
            return Ok(());
        }

        let required = (download.size_bytes as f64 * self.config.disk_space.size_multiplier) as u64;
        let required_with_buffer = required + self.config.disk_space.min_free_space;

        let available = get_available_space(&self.config.download_dir)?;

        if available < required_with_buffer {
            return Err(DiskSpaceError::InsufficientSpace {
                required: required_with_buffer,
                available,
            });
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DiskSpaceError {
    #[error("insufficient disk space: need {required} bytes, have {available} bytes")]
    InsufficientSpace { required: u64, available: u64 },

    #[error("failed to check disk space: {0}")]
    CheckFailed(#[from] std::io::Error),
}
```

---

## Server Health Check

### Design: Connectivity and Authentication Testing

API to verify server configuration before adding to production use.

### Implementation

```rust
impl UsenetDownloader {
    /// Test connectivity and authentication for a server configuration
    pub async fn test_server(&self, server: &ServerConfig) -> ServerTestResult {
        let start = Instant::now();

        // 1. TCP connection test
        let connect_result = TcpStream::connect((&*server.host, server.port)).await;
        if let Err(e) = connect_result {
            return ServerTestResult {
                success: false,
                latency: None,
                error: Some(format!("Connection failed: {}", e)),
                capabilities: None,
            };
        }

        // 2. TLS handshake (if enabled)
        // 3. NNTP greeting
        // 4. Authentication (if credentials provided)
        // 5. Capability check

        let latency = start.elapsed();

        match self.nntp_client.test_connection(server).await {
            Ok(capabilities) => ServerTestResult {
                success: true,
                latency: Some(latency),
                error: None,
                capabilities: Some(capabilities),
            },
            Err(e) => ServerTestResult {
                success: false,
                latency: Some(latency),
                error: Some(e.to_string()),
                capabilities: None,
            },
        }
    }

    /// Test all configured servers
    pub async fn test_all_servers(&self) -> Vec<(String, ServerTestResult)> {
        let mut results = Vec::new();
        for server in &self.config.servers {
            let result = self.test_server(server).await;
            results.push((server.host.clone(), result));
        }
        results
    }
}

#[derive(Debug)]
pub struct ServerTestResult {
    pub success: bool,
    pub latency: Option<Duration>,
    pub error: Option<String>,
    pub capabilities: Option<ServerCapabilities>,
}

#[derive(Debug)]
pub struct ServerCapabilities {
    pub posting_allowed: bool,
    pub max_connections: Option<u32>,
    pub compression: bool,
}
```

---

## Re-Processing API

### Design: Manual Post-Processing Trigger

Allow re-running post-processing on completed or failed downloads.

### Implementation

```rust
impl UsenetDownloader {
    /// Re-run post-processing on a download
    ///
    /// Useful when:
    /// - Extraction failed due to missing password (now added)
    /// - Post-processing settings changed
    /// - Files were manually repaired
    pub async fn reprocess(&self, id: DownloadId) -> Result<()> {
        let download = self.db.get_download(id).await?
            .ok_or(Error::NotFound)?;

        // Verify download files still exist
        let download_path = self.get_download_path(&download);
        if !download_path.exists() {
            return Err(Error::FilesNotFound);
        }

        // Reset status and re-queue for post-processing
        self.db.update_status(id, Status::Processing).await?;
        self.event_tx.send(Event::Verifying { id }).ok();

        // Start post-processing pipeline
        self.start_post_processing(id).await
    }

    /// Re-run only extraction (skip verify/repair)
    pub async fn reextract(&self, id: DownloadId) -> Result<()> {
        let download = self.db.get_download(id).await?
            .ok_or(Error::NotFound)?;

        self.db.update_status(id, Status::Processing).await?;
        self.event_tx.send(Event::Extracting {
            id,
            archive: "".into(),
            percent: 0.0
        }).ok();

        self.run_extraction(id).await
    }
}
```

---

## Failed Download Handling

### Design Choice: Configurable Action with File Preservation

When a download succeeds but post-processing fails, files are preserved by default.

### Configuration

```rust
#[derive(Clone, Debug, Default)]
pub enum FailedDownloadAction {
    /// Keep files in the download directory (default)
    #[default]
    Keep,
    /// Delete all downloaded files
    Delete,
    /// Move to a dedicated failed downloads directory
    MoveToFailed,
}

pub struct Config {
    // ...

    /// What to do with files when post-processing fails
    pub failed_action: FailedDownloadAction,

    /// Directory for failed downloads (when action is MoveToFailed)
    pub failed_directory: Option<PathBuf>,
}
```

### Pipeline Rollback Points

```
Download → PAR2 Verify → PAR2 Repair → Extract → Rename → Move → Cleanup
   │           │             │           │         │        │
   └───────────┴─────────────┴───────────┴─────────┴────────┘
                    Files preserved on failure
                    (based on FailedDownloadAction)
```

**Behavior by stage:**

| Stage | On Failure | Files State |
|-------|------------|-------------|
| Download | Mark failed | Partial files deleted |
| Verify | Mark failed | Downloaded files preserved |
| Repair | Mark failed | Downloaded files preserved |
| Extract | Mark failed | Downloaded + extracted files preserved |
| Move | Mark failed | Extracted files preserved in temp location |
| Cleanup | Log warning, continue | Considered successful |

---

## Password Management

### Design: Multi-Source with Caching

Inspired by SABnzbd's comprehensive approach with NZBGet's simplicity.

### Password Sources (Priority Order)

1. **Cached correct password** - From previous successful extraction of same archive
2. **Per-download password** - User-specified for this download
3. **NZB metadata password** - Embedded in NZB file
4. **Global password file** - One password per line
5. **Empty password** - Optional fallback

### Configuration

```rust
pub struct PasswordConfig {
    /// Path to global password file (one password per line)
    pub password_file: Option<PathBuf>,

    /// Try empty password as last resort
    pub try_empty_password: bool,
}

pub struct DownloadOptions {
    pub category: Option<String>,
    pub destination: Option<PathBuf>,
    pub post_process: Option<PostProcess>,
    pub priority: Priority,

    /// Password for this specific download (high priority)
    pub password: Option<String>,
}
```

### Internal Implementation

```rust
struct PasswordList {
    passwords: Vec<String>,
}

impl PasswordList {
    /// Collect passwords from all sources, de-duplicated, in priority order
    fn collect(
        cached_correct: Option<&str>,
        download_password: Option<&str>,
        nzb_meta_password: Option<&str>,
        global_file: Option<&Path>,
        try_empty: bool,
    ) -> Self {
        let mut seen = HashSet::new();
        let mut passwords = Vec::new();

        // Add in priority order, skip duplicates
        for pw in [cached_correct, download_password, nzb_meta_password].into_iter().flatten() {
            if seen.insert(pw.to_string()) {
                passwords.push(pw.to_string());
            }
        }

        // Add from file
        if let Some(path) = global_file {
            if let Ok(content) = std::fs::read_to_string(path) {
                for line in content.lines() {
                    let pw = line.trim();
                    if !pw.is_empty() && seen.insert(pw.to_string()) {
                        passwords.push(pw.to_string());
                    }
                }
            }
        }

        // Empty password last
        if try_empty && seen.insert(String::new()) {
            passwords.push(String::new());
        }

        Self { passwords }
    }
}
```

### Extraction Flow

```rust
async fn extract_with_passwords(
    &self,
    download_id: DownloadId,
    archive: &Path,
    passwords: &PasswordList,
) -> Result<(), ExtractError> {
    for (i, password) in passwords.iter().enumerate() {
        match self.try_extract(archive, password).await {
            Ok(()) => {
                // Cache successful password in SQLite
                self.db.set_correct_password(download_id, password).await?;
                return Ok(());
            }
            Err(ExtractError::WrongPassword) => {
                // Try next password
                continue;
            }
            Err(e) => {
                // Other error (corrupt archive, disk full, etc.)
                return Err(e);
            }
        }
    }

    Err(ExtractError::AllPasswordsFailed)
}
```

---

## Speed Limiting

### Design: Global Limit Distributed Across Downloads

The speed limit applies to total bandwidth across all concurrent downloads.

### Implementation Strategy

```rust
pub struct UsenetDownloader {
    /// Global speed limiter (shared across all downloads)
    speed_limiter: Arc<SpeedLimiter>,
    // ...
}

impl UsenetDownloader {
    /// Set global speed limit in bytes per second. None = unlimited.
    pub async fn set_speed_limit(&self, limit_bps: Option<u64>) {
        self.speed_limiter.set_limit(limit_bps);
        self.event_tx.send(Event::SpeedLimitChanged { limit_bps }).ok();
    }
}

struct SpeedLimiter {
    limit_bps: AtomicU64,  // 0 = unlimited
    tokens: AtomicU64,
    last_refill: AtomicU64,
}

impl SpeedLimiter {
    /// Request permission to transfer `bytes`. Returns immediately if unlimited,
    /// otherwise waits until bandwidth is available.
    async fn acquire(&self, bytes: u64) {
        // Token bucket algorithm
        // All concurrent downloads share the same bucket
    }
}
```

### Bandwidth Distribution

With 3 concurrent downloads and 10 MB/s limit:
- Each download requests tokens from shared bucket
- Natural distribution based on demand
- Fast downloads get throttled, slow downloads proceed normally
- Total never exceeds 10 MB/s

---

## Persistence (SQLite)

### Schema

```sql
-- Download queue
CREATE TABLE downloads (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    nzb_path TEXT NOT NULL,
    nzb_meta_name TEXT,  -- Title from NZB metadata
    category TEXT,
    destination TEXT NOT NULL,
    post_process INTEGER NOT NULL,  -- PostProcess enum
    priority INTEGER NOT NULL DEFAULT 0,
    status INTEGER NOT NULL DEFAULT 0,  -- pending, downloading, processing, complete, failed
    progress REAL DEFAULT 0.0,
    speed_bps INTEGER DEFAULT 0,
    size_bytes INTEGER DEFAULT 0,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    completed_at INTEGER
);

-- Track individual article download status (for resume)
CREATE TABLE download_articles (
    id INTEGER PRIMARY KEY,
    download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    segment_number INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL,
    status INTEGER NOT NULL DEFAULT 0,  -- 0=pending, 1=downloaded, 2=failed
    downloaded_at INTEGER,
    UNIQUE(download_id, message_id)
);

-- Password cache
CREATE TABLE passwords (
    download_id INTEGER PRIMARY KEY REFERENCES downloads(id),
    correct_password TEXT NOT NULL
);

-- Processed NZB files (for watch folder with Keep action)
CREATE TABLE processed_nzbs (
    path TEXT PRIMARY KEY,
    processed_at INTEGER NOT NULL
);

-- Download history (optional, for statistics)
CREATE TABLE history (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    category TEXT,
    destination TEXT,
    status INTEGER NOT NULL,
    size_bytes INTEGER,
    download_time_secs INTEGER,
    completed_at INTEGER NOT NULL
);

-- Indexes
CREATE INDEX idx_downloads_status ON downloads(status);
CREATE INDEX idx_downloads_priority ON downloads(priority DESC, created_at ASC);
CREATE INDEX idx_articles_download ON download_articles(download_id);
CREATE INDEX idx_articles_status ON download_articles(download_id, status);
CREATE INDEX idx_history_completed ON history(completed_at DESC);
```

### Queue Persistence

```rust
impl UsenetDownloader {
    /// Load queue from database on startup
    async fn restore_queue(&self) -> Result<()> {
        let downloads = self.db.get_incomplete_downloads().await?;
        for download in downloads {
            match download.status {
                Status::Downloading => self.resume_download(download.id).await?,
                Status::Processing => self.start_post_processing(download.id).await?,
                _ => self.queue.push(download),
            }
        }
        Ok(())
    }

    /// Save queue state periodically and on changes
    async fn persist_download(&self, download: &Download) -> Result<()> {
        self.db.upsert_download(download).await
    }
}
```

---

## Post-Processing Pipeline

### Stages

```
Download → PAR2 Verify → PAR2 Repair (if needed) → Extract → Rename → Move → Cleanup
```

### Configuration

```rust
#[derive(Clone, Copy, Debug, Default)]
pub enum PostProcess {
    /// Just download, no post-processing
    None,
    /// Download + PAR2 verify
    Verify,
    /// Download + PAR2 verify/repair
    Repair,
    /// Above + extract archives
    Unpack,
    /// Above + remove intermediate files (default)
    #[default]
    UnpackAndCleanup,
}
```

### Cleanup Targets

Files removed after successful extraction (when `PostProcess::UnpackAndCleanup`):

- `.par2` files
- `.nzb` files
- `.sfv` files
- `.srr` files
- Archive files (`.rar`, `.r00`, `.7z`, `.zip`)
- Sample folders (configurable)

---

## Queue Management

### Priority System

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low = -1,
    Normal = 0,
    High = 1,
    Force = 2,  // Start immediately, ignore concurrent limit
}
```

### Queue Operations

```rust
impl UsenetDownloader {
    // Add to queue
    pub async fn add_nzb(&self, path: &Path, options: DownloadOptions) -> Result<DownloadId>;
    pub async fn add_nzb_content(&self, content: &[u8], name: &str, options: DownloadOptions) -> Result<DownloadId>;
    pub async fn add_nzb_url(&self, url: &str, options: DownloadOptions) -> Result<DownloadId>;

    // Individual download control
    pub async fn pause(&self, id: DownloadId) -> Result<()>;
    pub async fn resume(&self, id: DownloadId) -> Result<()>;
    pub async fn cancel(&self, id: DownloadId) -> Result<()>;
    pub async fn set_priority(&self, id: DownloadId, priority: Priority) -> Result<()>;

    // Queue-wide control
    pub async fn pause_all(&self) -> Result<()>;
    pub async fn resume_all(&self) -> Result<()>;

    // Re-processing
    pub async fn reprocess(&self, id: DownloadId) -> Result<()>;
    pub async fn reextract(&self, id: DownloadId) -> Result<()>;

    // Query
    pub async fn get_download(&self, id: DownloadId) -> Option<DownloadInfo>;
    pub async fn list_downloads(&self) -> Vec<DownloadInfo>;
    pub async fn get_history(&self, limit: usize) -> Vec<HistoryEntry>;

    // Server health
    pub async fn test_server(&self, server: &ServerConfig) -> ServerTestResult;
    pub async fn test_all_servers(&self) -> Vec<(String, ServerTestResult)>;
}
```

---

## Data Types

### Download Info

```rust
#[derive(Clone, Debug)]
pub struct DownloadInfo {
    pub id: DownloadId,
    pub name: String,
    pub category: Option<String>,
    pub status: Status,
    pub progress: f32,
    pub speed_bps: u64,
    pub size_bytes: u64,
    pub downloaded_bytes: u64,
    pub eta_seconds: Option<u64>,
    pub priority: Priority,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Queued,
    Downloading,
    Paused,
    Processing,
    Complete,
    Failed,
}
```

### History Entry

```rust
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub id: i64,
    pub name: String,
    pub category: Option<String>,
    pub destination: Option<PathBuf>,
    pub status: Status,
    pub size_bytes: u64,
    pub download_time: Duration,
    pub completed_at: DateTime<Utc>,
}
```

---

## Configuration

### Full Config Structure

```rust
pub struct Config {
    // Connection (passed to nntp-rs)
    pub servers: Vec<ServerConfig>,

    // Download settings
    pub download_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub max_concurrent_downloads: usize,  // default: 3
    pub speed_limit_bps: Option<u64>,     // default: None (unlimited)

    // Retry settings
    pub retry: RetryConfig,

    // Post-processing
    pub default_post_process: PostProcess,  // default: UnpackAndCleanup
    pub failed_action: FailedDownloadAction,  // default: Keep
    pub failed_directory: Option<PathBuf>,
    pub delete_samples: bool,  // default: true

    // Extraction
    pub extraction: ExtractionConfig,

    // File handling
    pub file_collision: FileCollisionAction,  // default: Rename
    pub deobfuscation: DeobfuscationConfig,

    // Duplicate detection
    pub duplicate: DuplicateConfig,

    // Disk space
    pub disk_space: DiskSpaceConfig,

    // Passwords
    pub password_file: Option<PathBuf>,
    pub try_empty_password: bool,  // default: true

    // Archive tools (auto-detected if not specified)
    pub unrar_path: Option<PathBuf>,
    pub sevenzip_path: Option<PathBuf>,

    // Database
    pub database_path: PathBuf,

    // REST API
    pub api: ApiConfig,

    // Scheduler
    pub schedule_rules: Vec<ScheduleRule>,

    // Watch folders (global)
    pub watch_folders: Vec<WatchFolderConfig>,

    // RSS feeds
    pub rss_feeds: Vec<RssFeedConfig>,

    // Notifications
    pub webhooks: Vec<WebhookConfig>,
    pub scripts: Vec<ScriptConfig>,

    // Categories
    pub categories: HashMap<String, CategoryConfig>,
}

pub struct CategoryConfig {
    pub destination: PathBuf,
    pub post_process: Option<PostProcess>,  // Override default
    pub watch_folder: Option<WatchFolderConfig>,  // Category-specific watch
}

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    pub connections: usize,
    pub priority: i32,  // Lower = tried first, for backup servers
}
```

### Defaults

```rust
impl Default for Config {
    fn default() -> Self {
        Self {
            // User must configure at least one server
            servers: vec![],

            // Directories - sensible local paths
            download_dir: PathBuf::from("downloads"),
            temp_dir: PathBuf::from("temp"),

            // Download behavior
            max_concurrent_downloads: 3,  // Good balance of speed vs resources
            speed_limit_bps: None,        // Full speed by default

            // Resilience
            retry: RetryConfig::default(),

            // Post-processing - most users want ready-to-use files
            default_post_process: PostProcess::UnpackAndCleanup,
            failed_action: FailedDownloadAction::Keep,  // Don't lose data
            failed_directory: None,
            delete_samples: true,  // Usually unwanted

            // Extraction
            extraction: ExtractionConfig::default(),

            // File handling - never lose data
            file_collision: FileCollisionAction::Rename,
            deobfuscation: DeobfuscationConfig { enabled: true, min_length: 12 },
            duplicate: DuplicateConfig::default(),

            // Safety
            disk_space: DiskSpaceConfig::default(),

            // Passwords
            password_file: None,
            try_empty_password: true,  // Common for public releases

            // Archive tools - auto-detect from PATH
            unrar_path: None,
            sevenzip_path: None,

            // Database
            database_path: PathBuf::from("usenet-dl.db"),

            // API - secure localhost defaults
            api: ApiConfig::default(),

            // Automation - empty by default, user configures as needed
            schedule_rules: vec![],
            watch_folders: vec![],
            rss_feeds: vec![],
            webhooks: vec![],
            scripts: vec![],
            categories: HashMap::new(),
        }
    }
}
```

---

## REST API

### Design: OpenAPI 3.1 Compliant

The library exposes an HTTP REST API following OpenAPI standards for easy integration with any frontend or tooling.

**Rationale:**
- Language-agnostic client generation
- Standard tooling (Swagger UI, Postman, etc.)
- Self-documenting via OpenAPI spec
- Easy to test and debug

### API Structure

```
Base URL: http://localhost:6789/api/v1

Authentication: Optional API key via X-Api-Key header (configurable)
Content-Type: application/json
```

### Endpoints

#### Queue Management

```yaml
# List all downloads
GET /downloads
  Response: DownloadInfo[]

# Get single download
GET /downloads/{id}
  Response: DownloadInfo

# Add NZB from file upload
POST /downloads
  Content-Type: multipart/form-data
  Body: { file: binary, options?: DownloadOptions }
  Response: { id: DownloadId }

# Add NZB from URL
POST /downloads/url
  Body: { url: string, options?: DownloadOptions }
  Response: { id: DownloadId }

# Pause download
POST /downloads/{id}/pause
  Response: 204 No Content

# Resume download
POST /downloads/{id}/resume
  Response: 204 No Content

# Cancel/remove download
DELETE /downloads/{id}
  Query: ?delete_files=true|false
  Response: 204 No Content

# Set priority
PATCH /downloads/{id}/priority
  Body: { priority: "low" | "normal" | "high" | "force" }
  Response: 204 No Content

# Re-process download
POST /downloads/{id}/reprocess
  Response: 204 No Content

# Re-extract only
POST /downloads/{id}/reextract
  Response: 204 No Content
```

#### Queue-Wide Operations

```yaml
# Pause all downloads
POST /queue/pause
  Response: 204 No Content

# Resume all downloads
POST /queue/resume
  Response: 204 No Content

# Get queue statistics
GET /queue/stats
  Response: QueueStats
```

#### History

```yaml
# Get download history
GET /history
  Query: ?limit=50&offset=0&status=complete|failed
  Response: { items: HistoryEntry[], total: number }

# Clear history
DELETE /history
  Query: ?before=timestamp&status=complete|failed
  Response: { deleted: number }
```

#### Server Management

```yaml
# Test server connection
POST /servers/test
  Body: ServerConfig
  Response: ServerTestResult

# Test all configured servers
GET /servers/test
  Response: { server: string, result: ServerTestResult }[]
```

#### Configuration

```yaml
# Get current config (sensitive fields redacted)
GET /config
  Response: Config

# Update config
PATCH /config
  Body: Partial<Config>
  Response: Config

# Get speed limit
GET /config/speed-limit
  Response: { limit_bps: number | null }

# Set speed limit
PUT /config/speed-limit
  Body: { limit_bps: number | null }
  Response: 204 No Content
```

#### Categories

```yaml
# List categories
GET /categories
  Response: { [name: string]: CategoryConfig }

# Create/update category
PUT /categories/{name}
  Body: CategoryConfig
  Response: 204 No Content

# Delete category
DELETE /categories/{name}
  Response: 204 No Content
```

#### System

```yaml
# Health check
GET /health
  Response: { status: "ok", version: string }

# Get OpenAPI spec
GET /openapi.json
  Response: OpenAPI 3.1 specification

# Server-sent events for real-time updates
GET /events
  Response: text/event-stream

# Graceful shutdown
POST /shutdown
  Response: 202 Accepted
```

#### RSS Feeds

```yaml
# List RSS feeds
GET /rss
  Response: RssFeed[]

# Add RSS feed
POST /rss
  Body: RssFeedConfig
  Response: { id: RssFeedId }

# Update RSS feed
PUT /rss/{id}
  Body: RssFeedConfig
  Response: 204 No Content

# Delete RSS feed
DELETE /rss/{id}
  Response: 204 No Content

# Force check feed now
POST /rss/{id}/check
  Response: { queued: number }
```

#### Scheduler

```yaml
# Get schedule rules
GET /scheduler
  Response: ScheduleRule[]

# Add schedule rule
POST /scheduler
  Body: ScheduleRule
  Response: { id: RuleId }

# Update schedule rule
PUT /scheduler/{id}
  Body: ScheduleRule
  Response: 204 No Content

# Delete schedule rule
DELETE /scheduler/{id}
  Response: 204 No Content
```

### Request/Response Schemas

```rust
// All responses wrapped in standard envelope for errors
#[derive(Serialize)]
#[serde(untagged)]
pub enum ApiResponse<T> {
    Success(T),
    Error(ApiError),
}

#[derive(Serialize)]
pub struct ApiError {
    pub error: ErrorDetail,
}

#[derive(Serialize)]
pub struct ErrorDetail {
    pub code: String,           // Machine-readable: "not_found", "validation_error"
    pub message: String,        // Human-readable description
    pub details: Option<Value>, // Additional context (validation errors, etc.)
}
```

### HTTP Status Codes

| Code | Usage |
|------|-------|
| 200 | Success with body |
| 201 | Created (new download added) |
| 204 | Success, no body |
| 400 | Bad request / validation error |
| 401 | Unauthorized (invalid API key) |
| 404 | Resource not found |
| 409 | Conflict (e.g., download already paused) |
| 422 | Unprocessable (e.g., invalid NZB file) |
| 500 | Internal server error |
| 503 | Service unavailable (shutting down) |

### Server-Sent Events

Real-time event stream for UI updates:

```
GET /api/v1/events
Accept: text/event-stream

event: download_progress
data: {"id":123,"percent":45.2,"speed_bps":5242880}

event: download_complete
data: {"id":123,"path":"/downloads/Movie.Name"}

event: queue_paused
data: {}
```

Event types map directly to the internal `Event` enum.

### OpenAPI Generation

Using `utoipa` for compile-time OpenAPI spec generation:

```rust
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        routes::list_downloads,
        routes::get_download,
        routes::add_download,
        routes::add_download_url,
        routes::pause_download,
        routes::resume_download,
        routes::delete_download,
        // ... all routes
    ),
    components(schemas(
        DownloadInfo,
        DownloadOptions,
        HistoryEntry,
        QueueStats,
        ServerConfig,
        ServerTestResult,
        ApiError,
        // ... all types
    )),
    tags(
        (name = "downloads", description = "Download queue management"),
        (name = "queue", description = "Queue-wide operations"),
        (name = "history", description = "Download history"),
        (name = "servers", description = "Server management"),
        (name = "config", description = "Configuration"),
        (name = "system", description = "System endpoints"),
    )
)]
pub struct ApiDoc;
```

### API Server Configuration

```rust
pub struct ApiConfig {
    /// Address to bind to (default: 127.0.0.1:6789)
    pub bind_address: SocketAddr,

    /// Optional API key for authentication
    pub api_key: Option<String>,

    /// Enable CORS for browser access (default: true)
    pub cors_enabled: bool,

    /// Allowed CORS origins (default: ["*"])
    pub cors_origins: Vec<String>,

    /// Enable Swagger UI at /swagger-ui (default: true)
    pub swagger_ui: bool,

    /// Rate limiting configuration (disabled by default)
    pub rate_limit: RateLimitConfig,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:6789".parse().unwrap(),  // Localhost only for security
            api_key: None,                  // No auth needed for local access
            cors_enabled: true,             // Easy frontend development
            cors_origins: vec!["*".into()], // Allow all origins (localhost anyway)
            swagger_ui: true,               // Self-documenting API
            rate_limit: RateLimitConfig::default(),  // Disabled
        }
    }
}
```

### Example Client Usage

```bash
# Add NZB from URL
curl -X POST http://localhost:6789/api/v1/downloads/url \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/file.nzb", "options": {"category": "movies"}}'

# Get queue status
curl http://localhost:6789/api/v1/downloads

# Set speed limit to 10 MB/s
curl -X PUT http://localhost:6789/api/v1/config/speed-limit \
  -H "Content-Type: application/json" \
  -d '{"limit_bps": 10485760}'

# Stream events
curl -N http://localhost:6789/api/v1/events
```

---

## Scheduler

### Design: Time-Based Rules

Apply speed limits and pause/resume based on time schedules.

### Configuration

```rust
pub struct ScheduleRule {
    pub id: RuleId,

    /// Human-readable name
    pub name: String,

    /// Days this rule applies (empty = all days)
    pub days: Vec<Weekday>,

    /// Start time (HH:MM, 24-hour format)
    pub start_time: NaiveTime,

    /// End time (HH:MM, 24-hour format)
    pub end_time: NaiveTime,

    /// Action to take during this window
    pub action: ScheduleAction,

    /// Whether rule is active
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub enum ScheduleAction {
    /// Set speed limit (bytes per second)
    SpeedLimit(u64),
    /// Unlimited speed
    Unlimited,
    /// Pause all downloads
    Pause,
}

#[derive(Clone, Copy, Debug)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}
```

### Example Rules

```rust
// Unlimited at night (midnight to 6 AM)
ScheduleRule {
    name: "Night owl".into(),
    days: vec![],  // All days
    start_time: NaiveTime::from_hms(0, 0, 0),
    end_time: NaiveTime::from_hms(6, 0, 0),
    action: ScheduleAction::Unlimited,
    enabled: true,
}

// Limited during work hours
ScheduleRule {
    name: "Work hours".into(),
    days: vec![Monday, Tuesday, Wednesday, Thursday, Friday],
    start_time: NaiveTime::from_hms(9, 0, 0),
    end_time: NaiveTime::from_hms(17, 0, 0),
    action: ScheduleAction::SpeedLimit(1_000_000),  // 1 MB/s
    enabled: true,
}
```

### Rule Evaluation

```rust
impl Scheduler {
    /// Get current effective action (most specific rule wins)
    fn get_current_action(&self, now: DateTime<Local>) -> Option<ScheduleAction> {
        let weekday = now.weekday();
        let time = now.time();

        self.rules
            .iter()
            .filter(|r| r.enabled)
            .filter(|r| r.days.is_empty() || r.days.contains(&weekday))
            .filter(|r| time >= r.start_time && time < r.end_time)
            .map(|r| r.action.clone())
            .next()  // First matching rule wins (sorted by priority)
    }
}
```

---

## RSS Feeds

### Design: Automatic NZB Discovery

Monitor RSS feeds from indexers and auto-queue matching NZBs.

### Configuration

```rust
pub struct RssFeedConfig {
    /// Feed URL
    pub url: String,

    /// How often to check (default: 15 minutes)
    pub check_interval: Duration,

    /// Category to assign to downloads
    pub category: Option<String>,

    /// Only download items matching these filters
    pub filters: Vec<RssFilter>,

    /// Automatically download matches (vs just notify)
    pub auto_download: bool,

    /// Priority for auto-downloaded items
    pub priority: Priority,

    /// Whether feed is active
    pub enabled: bool,
}

pub struct RssFilter {
    /// Filter name (for UI)
    pub name: String,

    /// Patterns to include (regex)
    pub include: Vec<String>,

    /// Patterns to exclude (regex)
    pub exclude: Vec<String>,

    /// Minimum size (bytes)
    pub min_size: Option<u64>,

    /// Maximum size (bytes)
    pub max_size: Option<u64>,

    /// Maximum age (from publish date)
    pub max_age: Option<Duration>,
}
```

### RSS Feed Processing

```rust
impl RssManager {
    async fn check_feed(&self, feed: &RssFeed) -> Result<Vec<RssItem>> {
        let content = self.http_client.get(&feed.url).send().await?.text().await?;
        let channel = content.parse::<rss::Channel>()?;

        let mut new_items = Vec::new();

        for item in channel.items() {
            // Skip if already seen
            let guid = item.guid().map(|g| g.value()).unwrap_or(item.link().unwrap_or(""));
            if self.db.is_rss_item_seen(feed.id, guid).await? {
                continue;
            }

            // Check filters
            if !self.matches_filters(item, &feed.filters) {
                continue;
            }

            // Mark as seen
            self.db.mark_rss_item_seen(feed.id, guid).await?;

            // Auto-download or just collect
            if feed.auto_download {
                if let Some(nzb_url) = extract_nzb_url(item) {
                    self.downloader.add_nzb_url(&nzb_url, DownloadOptions {
                        category: feed.category.clone(),
                        priority: feed.priority,
                        ..Default::default()
                    }).await?;
                }
            }

            new_items.push(item.into());
        }

        Ok(new_items)
    }
}
```

### Database Schema Addition

```sql
-- RSS feeds
CREATE TABLE rss_feeds (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    check_interval_secs INTEGER NOT NULL DEFAULT 900,
    category TEXT,
    auto_download INTEGER NOT NULL DEFAULT 1,
    priority INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_check INTEGER,
    last_error TEXT,
    created_at INTEGER NOT NULL
);

-- RSS filters (per feed)
CREATE TABLE rss_filters (
    id INTEGER PRIMARY KEY,
    feed_id INTEGER NOT NULL REFERENCES rss_feeds(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    include_patterns TEXT,  -- JSON array
    exclude_patterns TEXT,  -- JSON array
    min_size INTEGER,
    max_size INTEGER,
    max_age_secs INTEGER
);

-- Seen RSS items (prevent re-downloading)
CREATE TABLE rss_seen (
    feed_id INTEGER NOT NULL REFERENCES rss_feeds(id) ON DELETE CASCADE,
    guid TEXT NOT NULL,
    seen_at INTEGER NOT NULL,
    PRIMARY KEY (feed_id, guid)
);

CREATE INDEX idx_rss_seen_feed ON rss_seen(feed_id);
```

---

## Duplicate Detection

### Design: Multi-Level Duplicate Checking

Prevent re-downloading content that's already been downloaded.

### Detection Levels

```rust
pub struct DuplicateConfig {
    /// Enable duplicate detection (default: true)
    pub enabled: bool,

    /// What to do when duplicate detected
    pub action: DuplicateAction,

    /// Check methods (in order)
    pub methods: Vec<DuplicateMethod>,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum DuplicateAction {
    /// Block the download entirely
    Block,
    /// Allow but emit warning event
    #[default]
    Warn,
    /// Allow silently
    Allow,
}

#[derive(Clone, Copy, Debug)]
pub enum DuplicateMethod {
    /// NZB content hash (most reliable)
    NzbHash,
    /// NZB filename
    NzbName,
    /// Extracted job name (deobfuscated)
    JobName,
}

impl Default for DuplicateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            action: DuplicateAction::Warn,  // Alert but don't block
            methods: vec![
                DuplicateMethod::NzbHash,   // Most reliable
                DuplicateMethod::JobName,   // Catches renamed NZBs
            ],
        }
    }
}
```

### Implementation

```rust
impl UsenetDownloader {
    async fn check_duplicate(&self, nzb_content: &[u8], name: &str) -> Option<DuplicateInfo> {
        if !self.config.duplicate.enabled {
            return None;
        }

        for method in &self.config.duplicate.methods {
            match method {
                DuplicateMethod::NzbHash => {
                    let hash = sha256(nzb_content);
                    if let Some(existing) = self.db.find_by_nzb_hash(&hash).await.ok().flatten() {
                        return Some(DuplicateInfo {
                            method: *method,
                            existing_id: existing.id,
                            existing_name: existing.name,
                        });
                    }
                }
                DuplicateMethod::NzbName => {
                    if let Some(existing) = self.db.find_by_name(name).await.ok().flatten() {
                        return Some(DuplicateInfo {
                            method: *method,
                            existing_id: existing.id,
                            existing_name: existing.name,
                        });
                    }
                }
                DuplicateMethod::JobName => {
                    let job_name = extract_job_name(name);
                    if let Some(existing) = self.db.find_by_job_name(&job_name).await.ok().flatten() {
                        return Some(DuplicateInfo {
                            method: *method,
                            existing_id: existing.id,
                            existing_name: existing.name,
                        });
                    }
                }
            }
        }

        None
    }
}

#[derive(Debug)]
pub struct DuplicateInfo {
    pub method: DuplicateMethod,
    pub existing_id: DownloadId,
    pub existing_name: String,
}
```

### Database Schema Addition

```sql
-- Add to downloads table
ALTER TABLE downloads ADD COLUMN nzb_hash TEXT;
ALTER TABLE downloads ADD COLUMN job_name TEXT;

CREATE INDEX idx_downloads_nzb_hash ON downloads(nzb_hash);
CREATE INDEX idx_downloads_job_name ON downloads(job_name);
```

---

## Graceful Shutdown

### Design: Clean State Preservation

Handle shutdown signals gracefully to preserve state and avoid corruption.

### Shutdown Sequence

```rust
impl UsenetDownloader {
    pub async fn shutdown(&self) -> Result<()> {
        tracing::info!("initiating graceful shutdown");

        // 1. Stop accepting new downloads
        self.accepting_new.store(false, Ordering::SeqCst);

        // 2. Stop folder watchers
        self.folder_watcher.stop().await;

        // 3. Stop RSS feed checks
        self.rss_manager.stop().await;

        // 4. Pause all active downloads (allow current article to finish)
        for download in self.active_downloads.iter() {
            download.pause_graceful().await;
        }

        // 5. Wait for current articles to complete (with timeout)
        let timeout = Duration::from_secs(30);
        tokio::time::timeout(timeout, self.wait_for_articles()).await.ok();

        // 6. Persist final state
        self.persist_all_state().await?;

        // 7. Close database connections
        self.db.close().await;

        // 8. Emit shutdown event
        self.event_tx.send(Event::Shutdown).ok();

        tracing::info!("shutdown complete");
        Ok(())
    }
}
```

### Signal Handling

```rust
async fn run_with_shutdown(downloader: UsenetDownloader) -> Result<()> {
    let shutdown = async {
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;

        tokio::select! {
            _ = sigterm.recv() => tracing::info!("received SIGTERM"),
            _ = sigint.recv() => tracing::info!("received SIGINT"),
        }

        Ok::<_, std::io::Error>(())
    };

    tokio::select! {
        result = downloader.run() => result,
        _ = shutdown => {
            downloader.shutdown().await?;
            Ok(())
        }
    }
}
```

### State Recovery on Restart

```rust
impl UsenetDownloader {
    async fn recover_state(&self) -> Result<()> {
        // Check for unclean shutdown
        if self.db.was_unclean_shutdown().await? {
            tracing::warn!("recovering from unclean shutdown");

            // Re-verify partially downloaded files
            let interrupted = self.db.get_interrupted_downloads().await?;
            for download in interrupted {
                self.verify_partial_download(download.id).await?;
            }
        }

        // Mark as cleanly started
        self.db.set_clean_start().await?;

        Ok(())
    }
}
```

---

## API Rate Limiting

### Design: Configurable, Not Enforced by Default

Protect API from abuse without impacting normal usage.

### Configuration

```rust
pub struct RateLimitConfig {
    /// Enable rate limiting (default: false)
    pub enabled: bool,

    /// Requests per second per IP (default: 100)
    pub requests_per_second: u32,

    /// Burst size (default: 200)
    pub burst_size: u32,

    /// Endpoints exempt from rate limiting
    pub exempt_paths: Vec<String>,

    /// IPs exempt from rate limiting (e.g., localhost)
    pub exempt_ips: Vec<IpAddr>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,              // Trust local network by default
            requests_per_second: 100,    // Generous limit when enabled
            burst_size: 200,             // Handle UI refresh bursts
            exempt_paths: vec![
                "/api/v1/events".into(),  // SSE is long-lived
                "/api/v1/health".into(),  // Health checks should always work
            ],
            exempt_ips: vec![
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ],
        }
    }
}
```

### Implementation

```rust
use tower::ServiceBuilder;
use tower_governor::{GovernorLayer, GovernorConfigBuilder};

fn create_router(config: &ApiConfig) -> Router {
    let app = Router::new()
        .route("/downloads", get(list_downloads).post(add_download))
        // ... other routes
        ;

    if config.rate_limit.enabled {
        let governor_config = GovernorConfigBuilder::default()
            .per_second(config.rate_limit.requests_per_second as u64)
            .burst_size(config.rate_limit.burst_size)
            .finish()
            .unwrap();

        app.layer(GovernorLayer { config: governor_config })
    } else {
        app
    }
}
```

### Rate Limit Response

```json
// HTTP 429 Too Many Requests
{
  "error": {
    "code": "rate_limited",
    "message": "Too many requests",
    "details": {
      "retry_after_seconds": 1
    }
  }
}
```

---

## Category Scripts

### Design: Per-Category Script Execution

Categories can have their own scripts in addition to global scripts.

### Configuration

```rust
pub struct CategoryConfig {
    pub destination: PathBuf,
    pub post_process: Option<PostProcess>,
    pub watch_folder: Option<WatchFolderConfig>,

    /// Scripts specific to this category
    pub scripts: Vec<ScriptConfig>,
}
```

### Execution Order

When a download completes:

1. **Category scripts** run first (if category has scripts)
2. **Global scripts** run after

```rust
impl UsenetDownloader {
    async fn trigger_completion_scripts(&self, download: &Download) {
        // Category scripts first
        if let Some(category) = &download.category {
            if let Some(cat_config) = self.config.categories.get(category) {
                for script in &cat_config.scripts {
                    if script.events.contains(&ScriptEvent::OnComplete) {
                        self.run_script_async(script, download);
                    }
                }
            }
        }

        // Then global scripts
        for script in &self.config.scripts {
            if script.events.contains(&ScriptEvent::OnComplete) {
                self.run_script_async(script, download);
            }
        }
    }
}
```

### Additional Environment Variables for Category Scripts

| Variable | Description |
|----------|-------------|
| `USENET_DL_CATEGORY` | Category name |
| `USENET_DL_CATEGORY_DESTINATION` | Category destination path |
| `USENET_DL_IS_CATEGORY_SCRIPT` | "true" if this is a category script |

---

## Dependencies

```toml
[dependencies]
nntp-rs = { path = "../nntp-rs" }
tokio = { version = "1", features = ["full", "signal"] }
sqlx = { version = "0.7", features = ["runtime-tokio", "sqlite"] }
unrar = "0.5"
sevenz-rust = "0.5"
zip = "0.6"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
thiserror = "1"
reqwest = { version = "0.11", features = ["json"] }  # For webhooks, URL fetching, RSS
notify = "6"  # For folder watching
chrono = { version = "0.4", features = ["serde"] }
sha2 = "0.10"  # For duplicate detection (NZB hash)
regex = "1"  # For RSS filters

# REST API
axum = { version = "0.7", features = ["multipart"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace"] }
tower-governor = "0.3"  # For rate limiting
utoipa = { version = "4", features = ["axum_extras"] }
utoipa-swagger-ui = { version = "6", features = ["axum"] }
tokio-stream = "0.1"  # For SSE

# RSS
rss = "2"
atom_syndication = "0.4"  # Some feeds use Atom
```

---

## Implementation Order

### Phase 1: Core Library
1. **Core structure** - Config, DownloadId, basic types, defaults
2. **SQLite persistence** - Schema, basic CRUD, article tracking
3. **Event system** - Broadcast channel setup
4. **Download manager** - Wrap nntp-rs, basic queue
5. **Queue with persistence** - Priority ordering, pause/resume
6. **Resume support** - Article-level tracking, restore on startup
7. **Speed limiting** - Token bucket implementation
8. **Retry logic** - Exponential backoff
9. **Graceful shutdown** - Signal handling, state preservation

### Phase 2: Post-Processing
10. **Post-processing skeleton** - Pipeline stages
11. **RAR extraction** - With password handling
12. **7z/ZIP extraction** - With password handling
13. **Nested extraction** - Recursive with depth limit
14. **Deobfuscation** - Filename detection and renaming
15. **File organization** - Move, collision handling
16. **Cleanup** - Remove intermediates

### Phase 3: REST API
17. **API server setup** - Axum, routing, middleware
18. **OpenAPI integration** - utoipa, schema generation
19. **Queue endpoints** - CRUD for downloads
20. **SSE events** - Real-time event streaming
21. **Config endpoints** - Runtime configuration
22. **Swagger UI** - Interactive documentation
23. **API rate limiting** - Optional, disabled by default

### Phase 4: Automation
24. **Folder watching** - NZB auto-import
25. **URL fetching** - NZB from HTTP
26. **RSS feed support** - Feed monitoring, auto-download
27. **Scheduler** - Time-based speed limits
28. **Duplicate detection** - Hash and name based

### Phase 5: Notifications & Polish
29. **External notifications** - Webhooks and scripts
30. **Category scripts** - Per-category script execution
31. **Disk space checking** - Pre-download validation
32. **Server health check** - Connectivity testing
33. **Re-processing API** - Manual post-process trigger
34. **Error handling** - Comprehensive error types
35. **Documentation** - API docs, examples, README
