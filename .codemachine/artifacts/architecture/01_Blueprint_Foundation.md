# 01_Blueprint_Foundation.md

<!-- anchor: 1-0-project-scale-directives -->
### **1.0 Project Scale & Directives for Architects**

*   **Classification:** **Medium**
*   **Rationale:** This project integrates an existing, mature Rust backend library (`usenet-dl`, ~8K+ lines) with an existing Svelte frontend (`spotweb-rs` frontend) to create a unified download management application. The scope involves:
    *   Adding a download service layer to `spotweb-rs` backend (5-8 new modules)
    *   Creating REST API endpoints with SSE for real-time events (~7 endpoints)
    *   Integrating frontend download UI components with backend API
    *   Database integration between two separate systems
    *   Configuration management and feature flagging

    This is a **departmental tool/startup MVP-scale project** with moderate complexity. The codebase size will be tens of KLOC after integration, requiring weeks to months of development. The team size is small (1-3 developers), and the primary goal is to deliver a functional, maintainable download management system without over-engineering.

*   **Core Directive for Architects:** This is a **Medium-scale** project. All architectural designs MUST prioritize:
    *   **Rapid development** using existing patterns from both codebases
    *   **Standard practices** (RESTful APIs, SQLite databases, SSE for real-time updates)
    *   **Moderate scalability** (handle 10-50 concurrent downloads, not thousands)
    *   **Minimal over-engineering** - avoid enterprise-level abstractions
    *   **Strong integration contracts** between `usenet-dl` and `spotweb-rs`
    *   **Feature flag discipline** - downloads MUST be toggleable via configuration

---

<!-- anchor: 2-0-standard-kit -->
### **2.0 The "Standard Kit" (Mandatory Technology Stack)**

*This technology stack is the non-negotiable source of truth. All architects MUST adhere to these choices without deviation.*

*   **Architectural Style:** Layered Monolith with Service-Oriented Components
    *   `spotweb-rs` backend acts as the API gateway and orchestration layer
    *   `usenet-dl` library provides the download engine as an embedded service
    *   Frontend consumes backend APIs via HTTP/SSE

*   **Frontend:** Svelte (existing `spotweb-rs` frontend)
    *   Component-based UI architecture
    *   RESTful API consumption
    *   Server-Sent Events (SSE) for real-time download progress

*   **Backend Language/Framework:** Rust
    *   **Primary Framework:** Axum 0.8 (HTTP server in `spotweb-rs`)
    *   **Download Engine:** `usenet-dl` library (embedded as dependency)
    *   **NNTP Protocol:** `nntp-rs` library (shared dependency)
    *   **Async Runtime:** Tokio 1.x (full features)

*   **Database(s):**
    *   **Primary (spotweb-rs):** SQLite 3.x via sqlx 0.8 (spots, comments, configuration)
    *   **Download State (usenet-dl):** Separate SQLite 3.x via sqlx 0.7 (downloads, articles, history, RSS, schedules)
    *   **Rationale:** Two independent databases to maintain separation of concerns and library independence

*   **Cloud Platform:** None (Self-hosted, on-premise deployment)

*   **Containerization:** Docker (optional for deployment, not mandatory for development)

*   **Messaging/Queues:** In-Memory Event Channels
    *   `tokio::sync::broadcast` for event propagation
    *   No external message broker required at this scale

*   **API Documentation:** OpenAPI 3.0 (utoipa + Swagger UI)

*   **Real-Time Communication:** Server-Sent Events (SSE) via `tokio-stream`

---

<!-- anchor: 3-0-rulebook -->
### **3.0 The "Rulebook" (Cross-Cutting Concerns)**

*This section defines system-wide strategies that apply to all components. These rules ensure consistency across the entire architecture.*

<!-- anchor: 3-1-feature-flag-strategy -->
#### **3.1 Feature Flag Strategy**

*   **Mandatory Approach:** Configuration-based feature toggles via `spotweb-rs` config file
*   **Implementation Rules:**
    *   The download subsystem MUST be controlled by a top-level `download.enabled: bool` flag in the configuration
    *   Default state: `download.enabled = false` (opt-in for initial rollout)
    *   When disabled, all download-related endpoints MUST return HTTP 501 Not Implemented
    *   The `DownloadService` MUST NOT be initialized if the flag is disabled
    *   Frontend MUST gracefully hide/disable download UI elements when the feature is unavailable
    *   No runtime flag changes - requires service restart
*   **Rationale:** Simple, file-based approach suitable for medium-scale self-hosted deployments

<!-- anchor: 3-2-observability -->
#### **3.2 Observability (Logging, Metrics, Tracing)**

*   **Logging:**
    *   Structured logging via `tracing` crate (already in use by both codebases)
    *   Log levels: ERROR, WARN, INFO, DEBUG, TRACE
    *   Critical download events (started, completed, failed) MUST log at INFO level
    *   Network errors and retries MUST log at WARN level
    *   Use existing `tracing-subscriber` setup from `spotweb-rs`
*   **Metrics:**
    *   No dedicated metrics endpoint required at this scale
    *   Key statistics exposed via `/api/downloads` list endpoint (active, queued, completed counts)
    *   Speed/bandwidth metrics embedded in download status objects
*   **Tracing:**
    *   Use Tokio's task instrumentation for async operation visibility
    *   No distributed tracing required (single-process architecture)

<!-- anchor: 3-3-security -->
#### **3.3 Security**

*   **Authentication/Authorization:**
    *   Follow `spotweb-rs` existing auth patterns (if any, otherwise unauthenticated localhost-only access)
    *   API binding MUST default to `127.0.0.1` (localhost only) to prevent external exposure
    *   No JWT/session management required at this stage (future enhancement)
*   **Input Validation:**
    *   All API inputs MUST be validated via Axum extractors and serde deserialization
    *   NZB content MUST be validated before passing to `usenet-dl`
    *   File paths MUST be sanitized to prevent directory traversal attacks
*   **Secrets Management:**
    *   NNTP credentials stored in configuration file (existing pattern)
    *   Download database passwords stored in usenet-dl's SQLite database
    *   No external secrets manager required

<!-- anchor: 3-4-error-handling -->
#### **3.4 Error Handling**

*   **API Errors:**
    *   Use standard HTTP status codes (400 Bad Request, 404 Not Found, 500 Internal Server Error, 501 Not Implemented)
    *   Error responses MUST include JSON bodies with `{ "error": "message" }` format
    *   Use Axum's built-in error handling middleware
*   **Service Errors:**
    *   `DownloadService` MUST wrap `usenet-dl` errors and translate to domain-specific error types
    *   Network errors MUST trigger automatic retries (managed by `usenet-dl`)
    *   Critical failures MUST emit ERROR-level logs and broadcast error events
*   **Frontend Error Handling:**
    *   API errors MUST be displayed to users via toast notifications or inline error messages
    *   Network timeouts MUST trigger user-visible retry prompts
    *   SSE connection drops MUST trigger automatic reconnection

<!-- anchor: 3-5-concurrency-model -->
#### **3.5 Concurrency & Resource Management**

*   **Download Concurrency:**
    *   Controlled by `download.max_concurrent_downloads` configuration parameter
    *   Default: 3 concurrent downloads (moderate resource usage)
    *   Enforced via Tokio semaphore in `usenet-dl`
*   **Connection Pooling:**
    *   NNTP connection pool managed by `usenet-dl` (separate from spotweb-rs pools)
    *   Configuration translation from `spotweb-rs` NNTP config to `usenet-dl` server config
*   **Database Access:**
    *   `spotweb-rs` and `usenet-dl` maintain separate SQLite connection pools
    *   No cross-database transactions required
    *   Download metadata synchronization via API calls, not direct database coupling

---

<!-- anchor: 4-0-blueprint -->
### **4.0 The "Blueprint" (Core Components & Boundaries)**

<!-- anchor: 4-1-system-overview -->
#### **4.1 System Overview**

The system architecture integrates two existing Rust applications into a unified Usenet spot browsing and download management platform. The `spotweb-rs` backend serves as the API gateway and orchestration layer, exposing a RESTful API consumed by a Svelte frontend. The `usenet-dl` library is embedded as a dependency, providing a fully-featured download engine with queue management, post-processing (repair, extraction), and persistence. Communication between components follows a strict API contract, with real-time updates delivered via Server-Sent Events (SSE). The architecture prioritizes simplicity and maintainability, avoiding microservice complexity while maintaining clear component boundaries.

<!-- anchor: 4-2-core-architectural-principle -->
#### **4.2 Core Architectural Principle**

**The architecture MUST enforce strong Separation of Concerns (SoC).** All components listed below must be loosely coupled through explicit interfaces. A change in the `SvelteWebApp` (e.g., UI redesign) MUST NOT require code changes in the `DownloadService`. Similarly, internal changes to the `usenet-dl` library (e.g., performance optimizations) MUST NOT affect the `SpotweBBackend` API contracts. Component communication MUST occur exclusively through:

1.  **HTTP/REST APIs** (synchronous request-response)
2.  **Server-Sent Events** (asynchronous event streaming)
3.  **Configuration objects** (initialization-time dependency injection)

Direct database coupling, shared mutable state, and tight module dependencies are PROHIBITED.

<!-- anchor: 4-3-key-components -->
#### **4.3 Key Components/Services**

<!-- anchor: 4-3-1-svelte-web-app -->
##### **4.3.1 SvelteWebApp**
*   **Responsibility:** Serves the user-facing interface for browsing spots and managing downloads
*   **Technology:** Svelte framework (existing frontend codebase)
*   **Dependencies:** Consumes `SpotweBBackend` REST API and SSE endpoints
*   **Key Features:**
    *   Spot search and browsing interface (existing)
    *   **NEW:** Download queue management page (list, pause, resume, cancel)
    *   **NEW:** Real-time download progress display (via SSE)
    *   **NEW:** "Download Now" button on spot detail pages
*   **Inputs:** User interactions, SSE events from backend
*   **Outputs:** HTTP requests to backend API

<!-- anchor: 4-3-2-spotweb-backend -->
##### **4.3.2 SpotweBBackend**
*   **Responsibility:** Provides the API gateway and orchestration layer for spot browsing and download management
*   **Technology:** Axum 0.8 HTTP server (Rust)
*   **Dependencies:**
    *   `usenet-dl` library (embedded download engine)
    *   `nntp-rs` library (NNTP protocol for spot/NZB fetching)
    *   SQLite database (spots, comments, configuration)
*   **Key Modules:**
    *   **Existing:** `api/routes.rs`, `api/spots.rs`, `api/nzb.rs`, `config.rs`, `db.rs`
    *   **NEW:** `services/download.rs` (DownloadService wrapper)
    *   **NEW:** `api/downloads.rs` (download API handlers)
*   **Inputs:** HTTP requests from `SvelteWebApp`, NZB content from Usenet
*   **Outputs:** HTTP responses, SSE event streams, download orchestration commands to `DownloadEngine`

<!-- anchor: 4-3-3-download-service -->
##### **4.3.3 DownloadService**
*   **Responsibility:** Wraps the `usenet-dl` library, providing a clean service interface for the `SpotweBBackend`
*   **Technology:** Rust module (`src/services/download.rs`)
*   **Dependencies:** `usenet-dl::UsenetDownloader` (embedded library)
*   **Key Methods:**
    *   `new(config: &Config) -> Result<Self>` - Initialize from spotweb config
    *   `subscribe() -> broadcast::Receiver<Event>` - Subscribe to download events
    *   `add_download(nzb_content: &[u8], name: String, options: QueueOptions) -> Result<i64>` - Queue NZB
    *   `list_downloads() -> Result<Vec<DownloadInfo>>` - Get all downloads
    *   `get_download(id: i64) -> Result<Option<DownloadInfo>>` - Get single download
    *   `pause(id: i64) -> Result<()>` - Pause download
    *   `resume(id: i64) -> Result<()>` - Resume download
    *   `cancel(id: i64) -> Result<()>` - Cancel/remove download
    *   `queue_stats() -> Result<QueueStats>` - Get queue statistics
*   **Inputs:** API calls from `SpotweBBackend` download handlers
*   **Outputs:** Download status, events broadcast, orchestration to `DownloadEngine`

<!-- anchor: 4-3-4-download-engine -->
##### **4.3.4 DownloadEngine (usenet-dl)**
*   **Responsibility:** Core download orchestration, queue management, article fetching, post-processing pipeline
*   **Technology:** `usenet-dl` library (embedded as Cargo dependency)
*   **Dependencies:**
    *   `nntp-rs` library (NNTP protocol client)
    *   SQLite database (download state, articles, history)
    *   External tools (`unrar`, `7z` for extraction)
*   **Key Capabilities:**
    *   Priority-based download queue (BinaryHeap)
    *   Multi-connection NNTP article fetching
    *   yEnc decoding and article assembly
    *   Par2 verification and repair
    *   Archive extraction (RAR/7z/ZIP)
    *   Filename deobfuscation
    *   Event broadcasting (download lifecycle events)
*   **Inputs:** NZB files, configuration from `DownloadService`
*   **Outputs:** Downloaded files, status updates, events

<!-- anchor: 4-3-5-nzb-service -->
##### **4.3.5 NzbService**
*   **Responsibility:** Fetches NZB content from Usenet newsgroups based on spot metadata
*   **Technology:** Existing `spotweb-rs` module
*   **Dependencies:** `nntp-rs` library, NNTP connection pool
*   **Integration Point:** Called by `SpotweBBackend` download handler to retrieve NZB before passing to `DownloadService`
*   **Inputs:** Spot message ID, newsgroup name
*   **Outputs:** NZB file content (XML bytes)

<!-- anchor: 4-3-6-primary-database -->
##### **4.3.6 PrimaryDatabase (spotweb-rs SQLite)**
*   **Responsibility:** Stores spot metadata, comments, reports, and spotweb-rs configuration
*   **Technology:** SQLite 3.x via sqlx 0.8
*   **Tables:** `spots`, `comments`, `reports`, `categories`, `config` (existing schema)
*   **Access Pattern:** Managed by `spotweb-rs` backend, not directly accessed by `usenet-dl`
*   **Inputs:** Spot sync operations, API queries
*   **Outputs:** Spot data for frontend display

<!-- anchor: 4-3-7-download-database -->
##### **4.3.7 DownloadDatabase (usenet-dl SQLite)**
*   **Responsibility:** Stores download state, article tracking, history, RSS feeds, scheduler rules
*   **Technology:** SQLite 3.x via sqlx 0.7
*   **Tables:** `downloads`, `download_articles`, `passwords`, `processed_nzbs`, `history`, `rss_feeds`, `rss_filters`, `rss_seen`, `schedule_rules` (usenet-dl schema)
*   **Access Pattern:** Managed exclusively by `usenet-dl` library, opaque to `spotweb-rs`
*   **Inputs:** Download operations from `DownloadEngine`
*   **Outputs:** Persistent download state

---

<!-- anchor: 5-0-contract -->
### **5.0 The "Contract" (API & Data Definitions)**

*This section defines the explicit rules of engagement between components. **These contracts are the single source of truth.** Parallel agents will build against these contracts, not their own assumptions, to ensure integration succeeds.*

<!-- anchor: 5-1-primary-api-style -->
#### **5.1 Primary API Style**

*   **Style:** RESTful HTTP/JSON (OpenAPI 3.0 documented)
*   **Base Path:** `/api/downloads` (new download management endpoints)
*   **Authentication:** None (localhost-only deployment, future enhancement)
*   **Content-Type:** `application/json` for requests/responses
*   **SSE Content-Type:** `text/event-stream` for event endpoints

<!-- anchor: 5-2-api-endpoints -->
#### **5.2 API Endpoints (Contract)**

| Endpoint | Method | Request Body | Response | Description |
|----------|--------|--------------|----------|-------------|
| `/api/spots/{messageid}/download` | POST | `QueueDownloadRequest` | `QueueDownloadResponse` | Queue spot for download |
| `/api/downloads` | GET | - | `DownloadListResponse` | List all downloads with stats |
| `/api/downloads/{id}` | GET | - | `DownloadDetailResponse` | Get single download details |
| `/api/downloads/{id}/pause` | POST | - | `204 No Content` | Pause download |
| `/api/downloads/{id}/resume` | POST | - | `204 No Content` | Resume download |
| `/api/downloads/{id}` | DELETE | - | `204 No Content` | Cancel/remove download |
| `/api/downloads/events` | GET | - | SSE Stream (`text/event-stream`) | Real-time download events |

<!-- anchor: 5-3-request-types -->
#### **5.3 Request Data Types**

```rust
/// Queue download request (POST /api/spots/{messageid}/download)
#[derive(Deserialize, ToSchema)]
pub struct QueueDownloadRequest {
    /// Priority level for queue positioning
    /// Valid values: "low", "normal", "high", "force"
    /// Default: "normal"
    #[serde(default)]
    pub priority: Priority,

    /// Category for organization and post-processing rules
    /// Optional, null means no category
    pub category: Option<String>,

    /// Custom output directory (overrides default)
    /// Optional, null uses configured download_dir
    pub output_dir: Option<PathBuf>,
}

#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Normal,
    High,
    Force,  // Jumps to front of queue
}
```

<!-- anchor: 5-4-response-types -->
#### **5.4 Response Data Types**

```rust
/// Queue download response
#[derive(Serialize, ToSchema)]
pub struct QueueDownloadResponse {
    /// Unique download ID (usenet-dl database primary key)
    pub download_id: i64,

    /// Download name (from NZB metadata)
    pub name: String,

    /// Initial status ("queued")
    pub status: DownloadStatus,
}

/// Download list response (GET /api/downloads)
#[derive(Serialize, ToSchema)]
pub struct DownloadListResponse {
    /// All downloads (active, queued, completed, failed)
    pub downloads: Vec<DownloadItem>,

    /// Queue statistics
    pub stats: QueueStats,
}

/// Individual download item
#[derive(Serialize, ToSchema)]
pub struct DownloadItem {
    pub id: i64,
    pub name: String,
    pub status: DownloadStatus,

    /// Progress percentage (0.0 to 100.0)
    pub progress: f32,

    /// Current download speed in bytes per second
    pub speed_bps: u64,

    /// Total size in bytes
    pub size_bytes: u64,

    /// Downloaded bytes so far
    pub downloaded_bytes: u64,

    /// Estimated time remaining in seconds (null if unknown)
    pub eta_seconds: Option<u64>,

    /// Timestamp when download was added
    pub created_at: String,  // ISO 8601 format

    /// Timestamp of last status update
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Queued,       // In queue, not yet started
    Downloading,  // Actively fetching articles
    Paused,       // User paused
    Processing,   // Post-processing (verify/repair/extract)
    Completed,    // Finished successfully
    Failed,       // Encountered unrecoverable error
}

/// Queue statistics
#[derive(Serialize, ToSchema)]
pub struct QueueStats {
    pub total: usize,
    pub downloading: usize,
    pub queued: usize,
    pub paused: usize,
    pub processing: usize,
    pub completed: usize,
    pub failed: usize,
}
```

<!-- anchor: 5-5-sse-event-format -->
#### **5.5 SSE Event Format**

```
event: download_started
data: {"download_id": 123, "name": "Example.Download"}

event: download_progress
data: {"download_id": 123, "progress": 45.2, "speed_bps": 5242880, "eta_seconds": 120}

event: download_status_changed
data: {"download_id": 123, "status": "processing", "stage": "extracting"}

event: download_completed
data: {"download_id": 123, "name": "Example.Download", "path": "/downloads/Example.Download"}

event: download_failed
data: {"download_id": 123, "error": "Connection timeout after 3 retries"}
```

<!-- anchor: 5-6-data-model-core-entities -->
#### **5.6 Data Model - Core Entities**

<!-- anchor: 5-6-1-download-entity -->
##### **5.6.1 Download Entity (usenet-dl database)**

```sql
CREATE TABLE downloads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    nzb_content BLOB NOT NULL,  -- Original NZB XML
    status TEXT NOT NULL,        -- queued, downloading, paused, processing, completed, failed
    priority INTEGER NOT NULL,   -- -1=low, 0=normal, 1=high, 2=force
    category TEXT,
    size_bytes INTEGER,
    downloaded_bytes INTEGER DEFAULT 0,
    progress REAL DEFAULT 0.0,
    speed_bps INTEGER DEFAULT 0,
    eta_seconds INTEGER,
    output_dir TEXT,
    error_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

<!-- anchor: 5-6-2-spot-entity -->
##### **5.6.2 Spot Entity (spotweb-rs database)**

*Existing entity, no schema changes required.*

```sql
CREATE TABLE spots (
    messageid TEXT PRIMARY KEY,
    poster TEXT NOT NULL,
    title TEXT NOT NULL,
    tag TEXT,
    category INTEGER NOT NULL,
    subcata TEXT,
    subcatb TEXT,
    subcatc TEXT,
    subcatd TEXT,
    subcatz TEXT,
    filesize INTEGER,
    spotdate INTEGER NOT NULL,
    -- ... additional fields
);
```

<!-- anchor: 5-6-3-nzb-metadata-entity -->
##### **5.6.3 NzbMetadata Entity (in-memory, not persisted in spotweb-rs)**

*This is the intermediate data structure passed from `NzbService` to `DownloadService`.*

```rust
pub struct NzbMetadata {
    /// Raw NZB XML content
    pub content: Vec<u8>,

    /// Display name extracted from NZB or spot title
    pub name: String,

    /// Total size in bytes (sum of all file segments)
    pub size_bytes: u64,

    /// Source spot message ID (for audit trail)
    pub source_spot_id: String,
}
```

<!-- anchor: 5-7-configuration-contract -->
#### **5.7 Configuration Contract**

<!-- anchor: 5-7-1-spotweb-config-download-section -->
##### **5.7.1 spotweb-rs Config - Download Section**

```json
{
  "download": {
    "enabled": false,
    "download_dir": "/path/to/downloads",
    "temp_dir": "/path/to/temp",
    "max_concurrent_downloads": 3,
    "speed_limit_bps": null,
    "database_path": "/path/to/usenet-dl.db",
    "nntp_connections_per_download": 8,
    "enable_par2_repair": true,
    "enable_extraction": true,
    "enable_deobfuscation": true,
    "delete_failed_downloads": false
  }
}
```

<!-- anchor: 5-7-2-config-translation-rules -->
##### **5.7.2 Config Translation Rules (spotweb-rs → usenet-dl)**

*   **NNTP Server Config:** Translate `spotweb-rs::NntpServerConfig.binary_server` to `usenet_dl::ServerConfig`
    *   Map `host`, `port`, `username`, `password` directly
    *   Map `ssl: bool` to `usenet_dl::ServerConfig::use_tls`
*   **Download Paths:** Use `download.download_dir` and `download.temp_dir` directly
*   **Concurrency:** Use `download.max_concurrent_downloads` for `usenet_dl::Config::concurrent_limit`
*   **NNTP Connections:** Use `download.nntp_connections_per_download` for `usenet_dl::ServerConfig::max_connections`
*   **Feature Flags:** Map `enable_par2_repair`, `enable_extraction`, `enable_deobfuscation` to usenet-dl configuration

---

<!-- anchor: 6-0-safety-net -->
### **6.0 The "Safety Net" (Ambiguities & Assumptions)**

*This section clarifies ambiguities from the user specifications to prevent incorrect work by the architects.*

<!-- anchor: 6-1-identified-ambiguities -->
#### **6.1 Identified Ambiguities**

1.  **Frontend Framework Version:** The specification mentions "Svelte frontend" but does not specify the version (Svelte 3, 4, or 5) or build tooling (Vite, SvelteKit).
2.  **Authentication Requirements:** The plan does not specify whether the download API should have authentication/authorization beyond localhost-only binding.
3.  **SSE Reconnection Strategy:** The specification does not define how the frontend should handle SSE connection drops or backend restarts.
4.  **Download Cleanup Policy:** No specification for handling completed downloads (auto-delete from queue, retention period, manual cleanup only).
5.  **Error Notification Mechanism:** The plan does not specify how frontend should display download errors (toast, banner, inline in list).
6.  **Multi-NZB Downloads:** The specification does not clarify whether users can queue multiple spots simultaneously (batch download).
7.  **Existing Download UI:** The plan assumes no existing download management UI exists in the Svelte frontend.
8.  **Category Management:** No specification for how categories should be managed (predefined list, user-defined, free text).
9.  **Bandwidth Limiting UI:** The plan does not specify whether speed limits should be adjustable via UI or config-only.
10. **Download History Visibility:** No specification for whether completed/failed downloads should be visible in the UI indefinitely or have a retention policy.

<!-- anchor: 6-2-governing-assumptions -->
#### **6.2 Governing Assumptions**

*   **Assumption 1 (Frontend):** The Svelte frontend uses a modern Vite-based build system with standard component architecture. The `Behavior_Architect` should design download UI components following existing patterns in the `spotweb-rs` frontend codebase (if available, otherwise standard Svelte conventions).

*   **Assumption 2 (Authentication):** The initial implementation will have NO authentication. The `listen_addr` configuration MUST default to `127.0.0.1` (localhost only) to prevent external access. The `Structural_Data_Architect` should design the database schema and API to be authentication-ready (e.g., include user_id columns with NULL default) for future enhancement.

*   **Assumption 3 (SSE Reconnection):** The frontend MUST implement automatic SSE reconnection with exponential backoff (1s, 2s, 4s, max 30s). The `Behavior_Architect` should model this reconnection logic in the frontend behavior specification.

*   **Assumption 4 (Download Cleanup):** Completed and failed downloads remain in the queue indefinitely until manually removed via DELETE endpoint. The `Structural_Data_Architect` should design the database to support future auto-cleanup rules (e.g., timestamp-based retention).

*   **Assumption 5 (Error Notification):** Download errors will be displayed as toast notifications (temporary, auto-dismissing) for real-time events, AND as inline error messages in the download list item. The `Behavior_Architect` should design both notification mechanisms.

*   **Assumption 6 (Batch Downloads):** Users CAN queue multiple spots by clicking download buttons sequentially. The initial implementation will NOT have a "select multiple and download" UI feature (future enhancement). The `Behavior_Architect` should design the API to support this pattern (multiple POST requests).

*   **Assumption 7 (Existing UI):** The Svelte frontend has NO existing download management UI. The `Behavior_Architect` MUST design a new `/downloads` page from scratch, following the existing design language of the spot browsing interface.

*   **Assumption 8 (Category Management):** Categories are free-text fields in the initial implementation. No predefined category list or validation. The `Structural_Data_Architect` should store categories as TEXT columns without foreign key constraints. Future enhancement may add a categories table.

*   **Assumption 9 (Bandwidth Limiting):** Speed limits are CONFIGURATION-ONLY in the initial implementation. No UI controls for adjusting bandwidth on-the-fly. The `Behavior_Architect` should NOT design speed limit controls in the frontend.

*   **Assumption 10 (Download History):** All downloads (queued, active, completed, failed) are visible in a single unified list view. The frontend should provide client-side filtering/sorting by status. The `Behavior_Architect` should design filter controls (e.g., tabs or dropdown for "All", "Active", "Completed", "Failed").

*   **Assumption 11 (Post-Processing Visibility):** The download status "processing" will display a generic "Post-processing..." message without detailed stage information (e.g., "Verifying", "Repairing", "Extracting"). The `usenet-dl` library emits stage events, but the `Behavior_Architect` should simplify the UI to avoid overwhelming users. Detailed stage info can be a future enhancement.

*   **Assumption 12 (NZB Fetching Integration):** The `SpotweBBackend` MUST fetch NZB content via the existing `NzbService` before queuing to `DownloadService`. The `Behavior_Architect` should model the handler flow as: `POST /api/spots/{messageid}/download` → `NzbService.fetch_nzb(messageid)` → `DownloadService.add_download(nzb_content)` → return download_id.

*   **Assumption 13 (Database Migration Strategy):** The `usenet-dl` library manages its own database migrations automatically on startup. The `Structural_Data_Architect` does NOT need to design migration tooling for the download database. However, the `spotweb-rs` database MAY need a migration to add a `downloaded: bool` flag to the `spots` table (future enhancement for tracking which spots have been queued).

*   **Assumption 14 (Error Recovery):** If the backend crashes or restarts, in-progress downloads will resume from the last checkpoint (managed by `usenet-dl`). The frontend should refresh the download list on reconnection to SSE. The `Behavior_Architect` should model this as a "refetch on SSE reconnect" behavior.

*   **Assumption 15 (Download Prioritization UI):** Users can set priority ("low", "normal", "high", "force") when queueing a download via an optional dropdown in the download dialog. The default priority is "normal". The `Behavior_Architect` should design a simple priority selector that is OPTIONAL (collapsed by default, expandable for advanced users).

---

**END OF FOUNDATION DOCUMENT**

*This document was generated by the Foundation Architect protocol (version 1.0). All specialized architects (`Structural_Data_Architect`, `Behavior_Architect`, `Ops_Docs_Architect`) MUST adhere to the constraints, contracts, and assumptions defined herein. Deviations from this foundation require explicit user approval and an updated version of this document.*
