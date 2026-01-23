# Project Specifications

- Describe goals, constraints, and context.
- Link any relevant docs or tickets.
- This file is created by workspace bootstrap and can be safely edited.


I want to make a proper frontend for the downloader backend. people shouldnt have to download an nzb (unless they want to) it should be all in the client.

So connect my svelte frontend to the backend and give me a proper download page in the same style.

First execute this plan to create the backend.

# Plan: Integrate usenet-dl as Download Backend for spotweb-rs
## Summary
Integrate your `usenet-dl` library (`/home/jens/Documents/source/usenet-dl/`) as the download backend for `spotweb-rs`. This will enable users to queue spots for download directly, with full queue management (pause/resume/cancel) and real-time progress via SSE.
## Files to Modify/Create
| File | Action | Description |
|------|--------|-------------|
| `spotweb-rs/Cargo.toml` | Modify | Add usenet-dl dependency |
| `spotweb-rs/src/config.rs` | Modify | Add `DownloadConfig` struct and conversion to usenet-dl config |
| `spotweb-rs/src/services/mod.rs` | Modify | Export new download module |
| `spotweb-rs/src/services/download.rs` | **Create** | New `DownloadService` wrapping usenet-dl |
| `spotweb-rs/src/api/mod.rs` | Modify | Export new downloads module |
| `spotweb-rs/src/api/downloads.rs` | **Create** | New download API handlers |
| `spotweb-rs/src/api/routes.rs` | Modify | Add to AppState, register routes, update OpenAPI |
| `spotweb-rs/src/main.rs` | Modify | Initialize DownloadService at startup |
## Implementation Steps
### 1. Add Dependency (`Cargo.toml`)
```toml
# Add after line 19
usenet-dl = { path = "../../usenet-dl" }
tokio-stream = { version = "0.1", features = ["sync"] }
```
### 2. Extend Configuration (`src/config.rs`)
Add `DownloadConfig` struct:
- `enabled: bool` - Feature toggle (default: false)
- `download_dir: PathBuf` - Output directory
- `temp_dir: PathBuf` - Temp directory
- `max_concurrent_downloads: usize` - Parallel downloads
- `speed_limit_bps: Option<u64>` - Bandwidth limit
- `database_path: PathBuf` - usenet-dl's SQLite database
Add `From<&NntpServerConfig> for usenet_dl::ServerConfig` conversion.
### 3. Create Download Service (`src/services/download.rs`)
Wrapper around `UsenetDownloader`:
```rust
pub struct DownloadService {
    downloader: Arc<UsenetDownloader>,
    event_tx: broadcast::Sender<Event>,
}
```
Methods:
- `new(config: &Config)` - Initialize from spotweb config
- `subscribe()` - Get event broadcast receiver
- `add_download(nzb_content, name, options)` - Queue NZB
- `list_downloads()` - Get all downloads
- `get_download(id)` - Get single download
- `pause(id)` / `resume(id)` / `cancel(id)` - Queue control
- `queue_stats()` - Get queue statistics
### 4. Create Download API Handlers (`src/api/downloads.rs`)
| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/spots/{messageid}/download` | POST | Queue spot for download |
| `/api/downloads` | GET | List all downloads with stats |
| `/api/downloads/{id}` | GET | Get download details |
| `/api/downloads/{id}/pause` | POST | Pause download |
| `/api/downloads/{id}/resume` | POST | Resume download |
| `/api/downloads/{id}` | DELETE | Cancel/remove download |
| `/api/downloads/events` | GET | SSE stream for real-time events |
The `queue_spot_download` handler:
1. Fetches NZB content via existing `NzbService`
2. Passes to `DownloadService.add_download()`
3. Returns download ID
### 5. Update AppState (`src/api/routes.rs`)
Add field to `AppState` (line 153):
```rust
pub download_service: Option<Arc<DownloadService>>,
```
Update `create_router()` signature (line 843) to accept `download_service` parameter.
Register new routes in the OpenApiRouter chain.
### 6. Initialize in main.rs
After NZB service initialization (~line 190):
```rust
let download_service = if config.download.enabled {
    Some(Arc::new(DownloadService::new(&config).await?))
} else {
    None
};
```
Pass to `create_router()`.
## Key Design Decisions
1. **Separate Databases**: usenet-dl maintains its own SQLite for download state
2. **Config Translation**: Convert spotweb-rs `NntpServerConfig` → usenet-dl `ServerConfig`
3. **Event Forwarding**: usenet-dl events → broadcast channel → SSE endpoint
4. **Feature Toggle**: `download.enabled` in config (default: false)
## API Response Types
```rust
// Queue request
struct QueueDownloadRequest {
    priority: Option<String>,  // "low" | "normal" | "high" | "force"
    category: Option<String>,
}
// Download item
struct DownloadItem {
    id: i64,
    name: String,
    status: String,  // "queued" | "downloading" | "paused" | "processing" | "completed" | "failed"
    progress: f32,
    speed_bps: u64,
    size_bytes: u64,
    downloaded_bytes: u64,
    eta_seconds: Option<u64>,
}
// Queue stats
struct QueueStatsResponse {
    total: usize,
    downloading: usize,
    queued: usize,
    paused: usize,
}
```
## Verification
1. **Build**: `cd spotweb-rs && cargo build`
2. **Tests**: `cargo test`
3. **Manual test**:
   - Enable downloads in config: `"download": { "enabled": true, "download_dir": "/tmp/downloads" }`
   - Start server: `cargo run`
   - Check OpenAPI: `http://127.0.0.1:8484/swagger-ui/`
   - Queue a download: `POST /api/spots/{messageid}/download`
   - List downloads: `GET /api/downloads`
   - Connect to SSE: `GET /api/downloads/events`

