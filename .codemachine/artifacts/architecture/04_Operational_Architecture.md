# 04_Operational_Architecture.md

**Project:** Usenet-dl Integration with spotweb-rs
**Architect:** Operational & Documentation Architect
**Date:** 2026-01-23
**Version:** 1.0

---

<!-- anchor: 3-0-proposed-architecture -->
## 3. Proposed Architecture (Operational View)

This document defines the operational aspects of integrating the `usenet-dl` download engine with the `spotweb-rs` backend and Svelte frontend. It covers cross-cutting concerns (security, logging, scalability), deployment strategies, and infrastructure requirements for a self-hosted, medium-scale Usenet download management platform.

---

<!-- anchor: 3-8-cross-cutting-concerns -->
### 3.8. Cross-Cutting Concerns

<!-- anchor: 3-8-1-authentication-authorization -->
#### 3.8.1 Authentication & Authorization

**Current State (Phase 1 - MVP):**

- **No Authentication Required:** The initial implementation deploys without authentication mechanisms
- **Network-Level Security:** Protection via localhost-only binding (`listen_addr: 127.0.0.1`)
- **Deployment Constraint:** The API server MUST NOT be exposed to external networks without implementing authentication first
- **Configuration Enforcement:** The default configuration file MUST set `listen_addr = "127.0.0.1"` with prominent warnings against changing to `0.0.0.0`

**Rationale:** For self-hosted, single-user deployments on trusted hardware (e.g., home NAS, personal VPS with VPN access), localhost binding provides adequate protection while minimizing implementation complexity. This aligns with the Medium-scale classification where rapid delivery is prioritized.

**Future Authentication Strategy (Phase 2 - Deferred):**

When external access is required, the architecture supports these authentication patterns:

1. **Session-Based Authentication:**
   - Axum middleware for session validation
   - HTTP-only secure cookies for token storage
   - Session store backed by SQLite (reusing existing database infrastructure)
   - Login endpoint: `POST /api/auth/login` (username/password → session cookie)
   - Logout endpoint: `POST /api/auth/logout` (invalidate session)

2. **Database Schema Extensions (Future-Ready):**
   ```sql
   -- Not implemented in Phase 1, but design-aware
   CREATE TABLE users (
       id INTEGER PRIMARY KEY AUTOINCREMENT,
       username TEXT UNIQUE NOT NULL,
       password_hash TEXT NOT NULL,  -- bcrypt
       created_at TEXT NOT NULL
   );

   CREATE TABLE sessions (
       id TEXT PRIMARY KEY,  -- UUID v4
       user_id INTEGER NOT NULL,
       expires_at TEXT NOT NULL,
       FOREIGN KEY (user_id) REFERENCES users(id)
   );
   ```

3. **Authorization Model (Future):**
   - Single-user model initially (no multi-tenancy required)
   - Role-Based Access Control (RBAC) for future multi-user scenarios:
     - `admin`: Full access (manage downloads, change configuration)
     - `user`: Read-only access (view downloads, queue spots)
   - Downloads table would gain `user_id` column (nullable for backward compatibility)

4. **API Protection Strategy:**
   - Axum middleware chain: `auth_middleware` → `route_handler`
   - Exempt endpoints: `/api/auth/login`, `/health`, OpenAPI docs (conditional)
   - 401 Unauthorized responses for unauthenticated requests
   - 403 Forbidden for insufficient permissions

**Implementation Readiness:**

- The `AppState` struct is designed to optionally hold an `AuthService` (currently `None`)
- API handlers use `Extension<AppState>` extractors, making auth injection seamless
- Database schema does NOT include auth tables in Phase 1 (avoid migration complexity)

---

<!-- anchor: 3-8-2-logging-monitoring -->
#### 3.8.2 Logging & Monitoring

**Logging Strategy:**

The system uses Rust's `tracing` ecosystem for structured, performance-aware logging across all components.

**Implementation Details:**

1. **Log Levels & Event Classification:**
   - `ERROR`: Critical failures requiring immediate attention (database corruption, NNTP connection pool exhaustion)
   - `WARN`: Recoverable errors and degraded states (article download retry, PAR2 repair failure on first attempt)
   - `INFO`: Significant operational events (download started/completed, NZB queued, API server started)
   - `DEBUG`: Detailed flow tracing (NNTP command/response pairs, queue state changes)
   - `TRACE`: Protocol-level minutiae (article decoding, yEnc parser steps) - disabled in production

2. **Structured Logging Format:**
   ```rust
   // Example log output (JSON format for production)
   {
     "timestamp": "2026-01-23T14:32:10.123Z",
     "level": "INFO",
     "target": "spotweb_rs::services::download",
     "fields": {
       "message": "Download started",
       "download_id": 42,
       "name": "Ubuntu.24.04.ISO",
       "size_bytes": 5368709120,
       "priority": "normal"
     },
     "span": {
       "name": "add_download",
       "request_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479"
     }
   }
   ```

3. **Logging Configuration:**
   ```rust
   // Configured in main.rs
   tracing_subscriber::fmt()
       .with_env_filter(
           EnvFilter::try_from_default_env()
               .unwrap_or_else(|_| EnvFilter::new("info,usenet_dl=debug,nntp_rs=warn"))
       )
       .with_target(true)
       .with_thread_ids(false)
       .with_file(true)
       .with_line_number(true)
       .json()  // Structured JSON for production
       .init();
   ```

4. **Log Targets & Filtering:**
   - `spotweb_rs::api::downloads`: All download API requests (INFO level)
   - `spotweb_rs::services::download`: Download orchestration (DEBUG level)
   - `usenet_dl`: Download engine internals (DEBUG in development, INFO in production)
   - `nntp_rs`: NNTP protocol layer (WARN level to reduce noise)
   - `axum::rejection`: Request validation failures (WARN level)

5. **Critical Events Requiring INFO-Level Logs:**
   - Download lifecycle: `download_queued`, `download_started`, `download_completed`, `download_failed`
   - API server: `server_started`, `server_shutdown`, `graceful_shutdown_timeout`
   - Database: `migration_applied`, `connection_pool_exhausted`
   - Configuration: `config_loaded`, `config_validation_failed`

6. **Log Rotation & Persistence (Production):**
   - Use `tracing-appender` for file-based logging with daily rotation
   - Log path: `{data_dir}/logs/spotweb-rs.{date}.log`
   - Retention: 7 days (configurable via `log.retention_days`)
   - Max size per file: 100 MB (rotation trigger)

**Monitoring Strategy:**

For Medium-scale deployments, monitoring focuses on embedded metrics exposure without requiring external infrastructure.

1. **Metrics Endpoint:**
   - `GET /api/stats` (JSON response with key performance indicators)
   - **No Prometheus/OpenMetrics** in Phase 1 (avoid dependency bloat)
   - Data source: In-memory metrics collected by `DownloadService` and Axum middleware

2. **Key Metrics Exposed:**
   ```json
   {
     "timestamp": "2026-01-23T14:35:00Z",
     "downloads": {
       "total": 156,
       "active": 2,
       "queued": 5,
       "completed": 145,
       "failed": 4
     },
     "performance": {
       "total_bandwidth_bps": 10485760,
       "avg_download_speed_bps": 5242880,
       "nntp_connections_active": 16,
       "nntp_connections_idle": 8
     },
     "system": {
       "uptime_seconds": 86400,
       "database_size_bytes": 52428800,
       "disk_free_bytes": 1073741824000
     },
     "api": {
       "requests_total": 1523,
       "requests_per_minute": 12.5,
       "avg_response_time_ms": 45
     }
   }
   ```

3. **Health Check Endpoint:**
   - `GET /health` (returns 200 OK if all subsystems healthy)
   - Checks:
     - Database connectivity (SQLite query test)
     - NNTP connection pool health (at least 1 idle connection)
     - Disk space availability (>1 GB free in download directory)
   - Response format:
     ```json
     {
       "status": "healthy",
       "checks": {
         "database": "ok",
         "nntp_pool": "ok",
         "disk_space": "ok"
       },
       "timestamp": "2026-01-23T14:40:00Z"
     }
     ```

4. **Frontend Monitoring Integration:**
   - Svelte app polls `/api/stats` every 10 seconds when download page is active
   - Display key metrics in dashboard header (active downloads, total speed)
   - Alert user if `/health` returns degraded status

5. **Future Monitoring Enhancements (Phase 2):**
   - Prometheus metrics exporter (`GET /metrics` in OpenMetrics format)
   - Grafana dashboard templates for visualization
   - Alerting rules (e.g., notify if >10 failed downloads in 1 hour)

**Error Tracking:**

- **Console Output:** All ERROR-level logs print to stderr with full context
- **Database Persistence:** Failed downloads store `error_message` in the `downloads` table for audit trail
- **Event Broadcasting:** Download failure events emitted to SSE stream for real-time frontend notification

---

<!-- anchor: 3-8-3-security-considerations -->
#### 3.8.3 Security Considerations

This section outlines security measures appropriate for a self-hosted, Medium-scale application with no external exposure in Phase 1.

**1. Network Security:**

- **Localhost Binding (Default):**
  - API server binds to `127.0.0.1:8484` by default
  - Prevents accidental exposure to LAN/WAN
  - Configuration validation: Emit WARN log if `listen_addr` is set to `0.0.0.0` or public IP

- **HTTPS/TLS (Future):**
  - Not required for localhost deployment (no network transit)
  - For remote access scenarios (Phase 2):
    - Axum + `rustls` for native TLS support
    - Certificate management via Let's Encrypt (manual setup, no auto-renewal in app)
    - Redirect HTTP → HTTPS via middleware

**2. Input Validation:**

- **API Request Validation:**
  - Serde deserialization provides type-level validation (e.g., `priority` enum prevents invalid values)
  - Path parameters (e.g., `download_id`) validated as `i64` (prevents SQL injection via type safety)
  - Custom validators for file paths:
    ```rust
    fn validate_output_dir(path: &Path) -> Result<(), ValidationError> {
        if path.as_os_str().to_string_lossy().contains("..") {
            return Err(ValidationError::new("directory_traversal_attempt"));
        }
        Ok(())
    }
    ```

- **NZB Content Validation:**
  - XML parsing via `quick-xml` (built-in protection against billion laughs attack)
  - Max NZB size: 10 MB (reject larger files to prevent DoS via memory exhaustion)
  - Reject NZBs with >10,000 file entries (prevents resource exhaustion)

- **SQL Injection Prevention:**
  - All database queries use sqlx's compile-time checked parameterized queries
  - Zero dynamic SQL construction (no string concatenation for queries)

**3. Secrets Management:**

- **NNTP Credentials:**
  - Stored in `config.json` (file system permissions: `chmod 600` recommended)
  - Not logged (tracing filters redact password fields)
  - Future enhancement: Environment variable override (`NNTP_PASSWORD` env var)

- **Database Encryption:**
  - SQLite databases stored unencrypted (acceptable for localhost-only deployment)
  - Future: SQLite encryption extension (`sqlcipher`) for at-rest encryption

- **Archive Passwords:**
  - Stored in `usenet-dl` database (`passwords` table)
  - Plaintext storage (no encryption) - acceptable for self-hosted use case
  - Alternative: bcrypt hashing (not applicable - passwords must be readable for extraction tools)

**4. Dependency Security:**

- **Supply Chain Validation:**
  - Use `cargo-audit` in CI to detect known vulnerabilities in dependencies
  - Pin exact versions in `Cargo.lock` (committed to repository)
  - Automated Dependabot updates for security patches

- **Minimal Dependency Policy:**
  - Avoid unnecessary crates to reduce attack surface
  - Prefer well-maintained crates with security audit history (`axum`, `tokio`, `sqlx`)

**5. File System Security:**

- **Path Traversal Prevention:**
  - All file operations validate paths against configured `download_dir` and `temp_dir`
  - Reject paths containing `..`, absolute paths outside allowed directories
  - Example check:
    ```rust
    fn is_path_safe(base: &Path, requested: &Path) -> bool {
        requested.canonicalize()
            .map(|p| p.starts_with(base.canonicalize().unwrap()))
            .unwrap_or(false)
    }
    ```

- **Archive Extraction Security:**
  - External tools (`unrar`, `7z`) run with restricted permissions
  - Extract to temporary directory first, then validate before moving to final location
  - Reject archives with files exceeding configured size limits (prevent zip bombs)

**6. Denial of Service (DoS) Protection:**

- **Rate Limiting (Future):**
  - Not implemented in Phase 1 (localhost deployment = trusted users)
  - Phase 2: Tower middleware for rate limiting (`tower-governor`)
  - Limits: 100 requests/minute per IP for download queue endpoints

- **Resource Limits:**
  - Max concurrent downloads: Configured via `max_concurrent_downloads` (default: 3)
  - NNTP connection pool: Hard cap at `max_connections * max_concurrent_downloads`
  - Database connection pool: 10 connections max (prevents resource exhaustion)

**7. Security Headers (Future - Phase 2):**

When external access is enabled, implement HTTP security headers via Axum middleware:
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`
- `X-XSS-Protection: 1; mode=block`
- `Content-Security-Policy: default-src 'self'`
- `Strict-Transport-Security: max-age=31536000` (HTTPS only)

**8. Audit Trail:**

- **Download History:** All download attempts logged to `history` table in usenet-dl database
- **API Access Logs:** Axum middleware logs all requests (method, path, status code, duration)
- **Configuration Changes:** Manual tracking (no automated audit log in Phase 1)

---

<!-- anchor: 3-8-4-scalability-performance -->
#### 3.8.4 Scalability & Performance

**Performance Targets (Medium-Scale):**

- **Concurrent Downloads:** 3-10 simultaneous downloads (configurable)
- **API Throughput:** 50-100 requests/second (adequate for single-user or small team)
- **SSE Connections:** Support 5-10 concurrent frontend clients (same user, multiple tabs/devices)
- **Download Speed:** Saturate available bandwidth (tested up to 211 MB/s in benchmarks)
- **Database Performance:** <50ms query latency for list operations (SQLite with indexes)

**Scalability Strategies:**

1. **Horizontal Scalability (Not Applicable):**
   - This is a **single-instance, monolithic architecture**
   - No load balancing or multi-instance deployment required at Medium scale
   - SQLite database is single-writer (precludes multi-instance deployment without major refactoring)

2. **Vertical Scalability (Primary Approach):**
   - **CPU:** Multi-core utilization via Tokio async runtime
     - Download workers run in parallel (limited by semaphore, not CPU cores)
     - yEnc decoding is CPU-bound (currently single-threaded per download, acceptable for Medium scale)
   - **Memory:** Efficient memory usage through streaming downloads
     - Article chunks buffered in memory (configurable chunk size: 256 KB default)
     - NZB parsing uses streaming XML parser (no full DOM in memory)
   - **Disk I/O:** Sequential write patterns for downloaded data
     - Temporary file writes use buffered I/O (`BufWriter`)
     - Post-processing reads/writes use OS page cache effectively

3. **Concurrency Optimizations:**
   - **NNTP Connection Pooling:**
     - Reusable connections per server (reduce SSL/TLS handshake overhead)
     - Connection pool size: `nntp_connections_per_download * max_concurrent_downloads`
     - Idle connection timeout: 60 seconds (balance between keep-alive overhead and reconnection cost)
   - **Async I/O:**
     - All network operations use Tokio's async I/O (non-blocking)
     - Database queries use sqlx async API (connection pooling with max 10 connections)
     - File writes remain synchronous (blocking I/O acceptable for local disk, minimal contention)

4. **Database Performance:**
   - **Indexing Strategy:**
     ```sql
     -- Critical indexes for usenet-dl database
     CREATE INDEX idx_downloads_status ON downloads(status);
     CREATE INDEX idx_downloads_priority_status ON downloads(priority, status);
     CREATE INDEX idx_download_articles_download_id ON download_articles(download_id);
     ```
   - **Query Optimization:**
     - List downloads: Single query with WHERE clause filter (no joins required)
     - Download details: Two queries (download row + article count) - acceptable latency
     - Queue statistics: Aggregate query with GROUP BY status (indexed, <10ms)
   - **Write Patterns:**
     - Batch inserts for articles (insert 100 articles per transaction during NZB queue)
     - Progress updates: Max 1 write per second per download (rate-limited to avoid contention)

5. **Bandwidth Management:**
   - **Token Bucket Rate Limiter:**
     - Implemented in `usenet-dl` (`speed_limiter.rs`)
     - Configurable global speed limit (`speed_limit_bps` in config)
     - Dynamically adjustable (future: API endpoint to change limit at runtime)
   - **Fair Bandwidth Allocation:**
     - Equal share per active download (no priority-based bandwidth allocation in Phase 1)
     - Future: Priority-weighted bandwidth distribution (high-priority downloads get 2x tokens)

6. **Caching (Minimal):**
   - **No HTTP response caching** (download data is dynamic, changes frequently)
   - **NNTP article caching:** Not implemented (articles downloaded once and discarded after assembly)
   - **Frontend caching:** Static assets (Svelte build output) served with ETags and cache headers

**Performance Bottlenecks & Mitigations:**

| Bottleneck | Impact | Mitigation |
|------------|--------|------------|
| SQLite write lock contention | Progress updates block each other | Rate-limit progress writes to 1 Hz per download |
| yEnc decoding CPU usage | High CPU on multi-GB downloads | Acceptable at Medium scale (future: SIMD optimizations) |
| NNTP server rate limits | Download stalls if server throttles | Exponential backoff retry (already implemented in usenet-dl) |
| Disk I/O on slow drives | Post-processing extraction slow | Recommend SSD for download directories (document requirement) |
| SSE connection overhead | Memory usage grows with clients | Limit to 10 concurrent SSE connections (close oldest on overflow) |

**Load Testing Recommendations:**

- **Simulated Workload:**
  - 5 concurrent downloads of 1 GB files
  - Frontend client polling `/api/downloads` every 5 seconds
  - SSE connection active for real-time updates
- **Metrics to Track:**
  - P50/P95/P99 API response times
  - Database query latencies (via tracing logs)
  - Memory usage growth over 24-hour period
  - Download completion time vs. theoretical network speed

---

<!-- anchor: 3-8-5-reliability-availability -->
#### 3.8.5 Reliability & Availability

**Availability Targets:**

For a self-hosted, Medium-scale application, the availability target is **99% uptime** (acceptable downtime: ~7 hours/month). This is appropriate for non-critical personal infrastructure where scheduled maintenance windows are feasible.

**Fault Tolerance Strategies:**

1. **Download Resumption:**
   - **Automatic Checkpoint Recovery:**
     - `usenet-dl` persists article download progress to SQLite (`download_articles` table)
     - Each article has a `downloaded: bool` flag
     - On restart, incomplete downloads resume from last checkpoint (only download missing articles)
   - **Crash Recovery:**
     - If process crashes during download, next startup scans `downloads` table for `status = 'downloading'`
     - Downloads automatically transition to `queued` state and resume
   - **Network Failure Handling:**
     - NNTP connection failures trigger exponential backoff retry (3 attempts with jitter)
     - Article fetch failures marked as failed, retry in next download cycle
     - If all NNTP servers unreachable, download transitions to `paused` (user notified via SSE)

2. **Database Durability:**
   - **SQLite WAL Mode:**
     - Both databases use Write-Ahead Logging (`PRAGMA journal_mode=WAL`)
     - Improves write concurrency and crash recovery
     - Checkpoint on graceful shutdown (flush WAL to main database file)
   - **Backup Strategy (User Responsibility):**
     - Document recommendation: Daily backups of SQLite files via cron/systemd timer
     - Example backup script:
       ```bash
       #!/bin/bash
       sqlite3 /path/to/spotweb.db ".backup /backups/spotweb-$(date +%Y%m%d).db"
       sqlite3 /path/to/usenet-dl.db ".backup /backups/usenet-dl-$(date +%Y%m%d).db"
       ```
   - **No Automatic Replication:** Single-instance architecture = no database replication

3. **Graceful Shutdown:**
   - **Signal Handling:**
     - Application listens for SIGTERM/SIGINT signals
     - On shutdown signal:
       1. Stop accepting new API requests (return 503 Service Unavailable)
       2. Wait for in-flight downloads to reach next checkpoint (max 30 seconds)
       3. Broadcast shutdown event to SSE clients
       4. Close NNTP connections gracefully
       5. Close database connections (triggers WAL checkpoint)
       6. Exit process
   - **Timeout Enforcement:**
     - If graceful shutdown exceeds 60 seconds, force quit (log ERROR with state snapshot)

4. **Health Monitoring:**
   - **Self-Healing (Automatic):**
     - If NNTP connection pool exhausted, log ERROR and attempt pool reset
     - If database connection pool exhausted, log ERROR and wait for connections to free (no automatic kill)
   - **Manual Intervention (User-Triggered):**
     - API endpoint: `POST /api/admin/reset-nntp-pool` (future enhancement)
     - Restart recommendation if degraded state persists >5 minutes

5. **Error Recovery Paths:**
   - **Download Failures:**
     - If download fails 3 times consecutively, transition to `failed` status
     - User can manually retry via `POST /api/downloads/{id}/resume` (resets failure counter)
   - **Post-Processing Failures:**
     - PAR2 repair failure: Mark download as `failed`, preserve partial data for manual intervention
     - Extraction failure: Mark as `failed`, log detailed error (password incorrect, corrupt archive, etc.)
   - **Disk Full:**
     - If write fails due to ENOSPC (no space), pause all downloads
     - Broadcast error event to SSE clients
     - Resume queue automatically once disk space freed (polling check every 60 seconds)

**High Availability (Not Applicable):**

- **No HA in Medium Scale:** Single-instance deployment is acceptable
- **Downtime Expectations:**
  - Planned maintenance: Schedule during low-usage hours (announce via logs)
  - Unplanned outages: User responsible for process monitoring (systemd restart on failure)
- **Future HA Considerations (Enterprise Scale):**
  - Migrate to PostgreSQL (multi-master replication)
  - Deploy multiple backend instances behind load balancer
  - Distribute download workers across instances (requires shared queue mechanism)

**Data Integrity:**

- **Article Validation:**
  - yEnc CRC32 checksum verification (per-article)
  - PAR2 verification (post-download, optional)
- **File Integrity:**
  - Hash verification if NZB includes `<file>` hash attributes (future enhancement)
  - No end-to-end integrity checks in Phase 1 (rely on NNTP protocol reliability)

---

<!-- anchor: 3-9-deployment-view -->
### 3.9. Deployment View

<!-- anchor: 3-9-1-target-environment -->
#### 3.9.1 Target Environment

**Cloud Platform:** None (Self-Hosted / On-Premise)

**Deployment Contexts:**

1. **Primary Target:** Personal NAS/Home Server
   - Hardware: x86_64 Linux (Ubuntu 22.04+, Debian 12+, Arch, etc.)
   - Resources: 2+ CPU cores, 4+ GB RAM, 100+ GB available disk space
   - Network: Gigabit LAN connection to internet router

2. **Alternative Targets:**
   - **VPS (Virtual Private Server):** DigitalOcean, Linode, Hetzner, etc.
     - Use case: Remote access via VPN (WireGuard, Tailscale)
     - Resources: 2 vCPU, 4 GB RAM, 100 GB SSD minimum
   - **Local Development Machine:** macOS, Linux, Windows (WSL2)
     - Use case: Development and testing only

3. **Unsupported Environments:**
   - Shared hosting (requires dedicated server or VPS)
   - Windows native (Rust toolchain requires WSL2 for usenet-dl dependencies)
   - Raspberry Pi (insufficient CPU for high-speed yEnc decoding, though functional at lower speeds)

---

<!-- anchor: 3-9-2-deployment-strategy -->
#### 3.9.2 Deployment Strategy

**Phase 1: Manual Deployment (Minimal Docker)**

The initial deployment approach prioritizes simplicity and avoids containerization complexity for experienced self-hosters. Docker is OPTIONAL, not mandatory.

**Option A: Native Binary Deployment (Recommended for NixOS/Arch Users):**

1. **Prerequisites:**
   - Rust toolchain 1.75+ (`rustup` installation)
   - System dependencies: `openssl`, `sqlite3`, `unrar`, `p7zip`
   - NNTP account credentials

2. **Build Process:**
   ```bash
   # Clone repositories (spotweb-rs and usenet-dl sibling directories)
   git clone https://github.com/user/spotweb-rs.git
   git clone https://github.com/user/usenet-dl.git

   # Build spotweb-rs (usenet-dl built as dependency)
   cd spotweb-rs
   cargo build --release

   # Binary output: target/release/spotweb-rs
   ```

3. **Installation:**
   ```bash
   # Copy binary to system path
   sudo cp target/release/spotweb-rs /usr/local/bin/

   # Create service user (optional but recommended)
   sudo useradd -r -s /bin/false spotweb

   # Create data directories
   sudo mkdir -p /var/lib/spotweb/{config,data,downloads,temp,logs}
   sudo chown -R spotweb:spotweb /var/lib/spotweb
   ```

4. **Configuration:**
   ```bash
   # Generate default config
   spotweb-rs --generate-config > /var/lib/spotweb/config/config.json

   # Edit config (set NNTP credentials, enable downloads)
   vi /var/lib/spotweb/config/config.json
   ```

5. **Systemd Service:**
   ```ini
   # /etc/systemd/system/spotweb-rs.service
   [Unit]
   Description=Spotweb-rs Usenet Download Manager
   After=network-online.target
   Wants=network-online.target

   [Service]
   Type=exec
   User=spotweb
   Group=spotweb
   WorkingDirectory=/var/lib/spotweb
   ExecStart=/usr/local/bin/spotweb-rs --config /var/lib/spotweb/config/config.json
   Restart=on-failure
   RestartSec=10s

   # Security hardening
   NoNewPrivileges=true
   PrivateTmp=true
   ProtectSystem=strict
   ProtectHome=true
   ReadWritePaths=/var/lib/spotweb

   [Install]
   WantedBy=multi-user.target
   ```

6. **Start Service:**
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl enable spotweb-rs
   sudo systemctl start spotweb-rs

   # Check status
   sudo systemctl status spotweb-rs
   sudo journalctl -u spotweb-rs -f
   ```

**Option B: Docker Deployment (Simplified for Non-Rust Users):**

1. **Dockerfile:**
   ```dockerfile
   # Multi-stage build for minimal image size
   FROM rust:1.75-slim AS builder

   # Install build dependencies
   RUN apt-get update && apt-get install -y \
       pkg-config libssl-dev sqlite3 \
       && rm -rf /var/lib/apt/lists/*

   # Copy source
   WORKDIR /build
   COPY spotweb-rs ./spotweb-rs
   COPY usenet-dl ./usenet-dl

   # Build release binary
   WORKDIR /build/spotweb-rs
   RUN cargo build --release

   # Runtime image
   FROM debian:bookworm-slim

   # Install runtime dependencies
   RUN apt-get update && apt-get install -y \
       ca-certificates unrar p7zip-full sqlite3 \
       && rm -rf /var/lib/apt/lists/*

   # Create app user
   RUN useradd -r -s /bin/false spotweb

   # Copy binary from builder
   COPY --from=builder /build/spotweb-rs/target/release/spotweb-rs /usr/local/bin/

   # Create data directories
   RUN mkdir -p /data/config /data/db /data/downloads /data/temp /data/logs && \
       chown -R spotweb:spotweb /data

   USER spotweb
   WORKDIR /data

   EXPOSE 8484
   VOLUME ["/data/config", "/data/db", "/data/downloads"]

   ENTRYPOINT ["/usr/local/bin/spotweb-rs"]
   CMD ["--config", "/data/config/config.json"]
   ```

2. **Docker Compose:**
   ```yaml
   version: '3.8'

   services:
     spotweb-rs:
       build:
         context: .
         dockerfile: Dockerfile
       container_name: spotweb-rs
       restart: unless-stopped
       ports:
         - "127.0.0.1:8484:8484"  # Localhost-only binding
       volumes:
         - ./config:/data/config:ro
         - ./data:/data/db
         - /path/to/downloads:/data/downloads
         - /tmp/spotweb-temp:/data/temp
       environment:
         - RUST_LOG=info,usenet_dl=debug
       healthcheck:
         test: ["CMD", "curl", "-f", "http://localhost:8484/health"]
         interval: 30s
         timeout: 10s
         retries: 3
         start_period: 40s
   ```

3. **Deployment Commands:**
   ```bash
   # Build image
   docker compose build

   # Start service
   docker compose up -d

   # View logs
   docker compose logs -f spotweb-rs

   # Stop service
   docker compose down
   ```

**Configuration Management:**

- **Config File Location:**
  - Native: `/var/lib/spotweb/config/config.json`
  - Docker: `./config/config.json` (bind-mounted to `/data/config/`)

- **Version Control:**
  - Store config template in repository (with placeholders for secrets)
  - Actual config with credentials: NOT committed to git (add to `.gitignore`)

- **Environment Variable Overrides (Future):**
  - Critical secrets overrideable via env vars: `NNTP_USERNAME`, `NNTP_PASSWORD`
  - Config priority: Environment variables > config file > defaults

---

<!-- anchor: 3-9-3-deployment-diagram -->
#### 3.9.3 Deployment Diagram (PlantUML)

```plantuml
@startuml
!include https://raw.githubusercontent.com/plantuml-stdlib/C4-PlantUML/master/C4_Deployment.puml

LAYOUT_WITH_LEGEND()

title Deployment Diagram - Spotweb-rs with usenet-dl Integration

Deployment_Node(homeserver, "Home Server / NAS", "Ubuntu 22.04 LTS", "Physical or VM"){
    Deployment_Node(docker, "Docker Engine", "Docker 24.x", "Optional - Containerization"){
        Container(spotweb_container, "spotweb-rs", "Rust Binary", "Axum HTTP server + usenet-dl library")
    }

    Deployment_Node(systemd, "systemd", "System Service Manager", "Alternative to Docker"){
        Container(spotweb_binary, "spotweb-rs", "Native Binary", "Axum HTTP server + usenet-dl library")
    }

    Deployment_Node(filesystem, "File System", "ext4 / btrfs / ZFS"){
        ContainerDb(spotweb_db, "spotweb.db", "SQLite 3.x", "Spots, comments, config")
        ContainerDb(downloads_db, "usenet-dl.db", "SQLite 3.x", "Downloads, articles, history")
        Container(downloads_storage, "Download Files", "File Storage", "Completed downloads")
        Container(temp_storage, "Temp Files", "File Storage", "In-progress downloads")
    }
}

Deployment_Node(client_device, "User Device", "Laptop / Desktop / Mobile"){
    Container(browser, "Web Browser", "Firefox / Chrome", "Svelte Frontend Application")
}

Deployment_Node(usenet_provider, "Usenet Provider", "External Service", "Newshosting, Eweka, etc."){
    ContainerDb(nntp_servers, "NNTP Servers", "NNTP Protocol", "Article storage")
}

Rel(browser, spotweb_container, "HTTP/SSE", "REST API + Real-time events")
Rel(browser, spotweb_binary, "HTTP/SSE", "REST API + Real-time events")

Rel(spotweb_container, spotweb_db, "Reads/Writes", "SQLite queries")
Rel(spotweb_container, downloads_db, "Reads/Writes", "Download state")
Rel(spotweb_container, downloads_storage, "Writes", "Completed files")
Rel(spotweb_container, temp_storage, "Reads/Writes", "Temp article data")

Rel(spotweb_binary, spotweb_db, "Reads/Writes", "SQLite queries")
Rel(spotweb_binary, downloads_db, "Reads/Writes", "Download state")
Rel(spotweb_binary, downloads_storage, "Writes", "Completed files")
Rel(spotweb_binary, temp_storage, "Reads/Writes", "Temp article data")

Rel(spotweb_container, nntp_servers, "NNTP/TLS", "Article fetching")
Rel(spotweb_binary, nntp_servers, "NNTP/TLS", "Article fetching")

SHOW_LEGEND()
@enduml
```

**Diagram Notes:**

- **Dual Deployment Options:** The diagram shows both Docker (containerized) and systemd (native binary) deployment paths - user chooses one
- **Localhost-Only Access:** The browser connects to `127.0.0.1:8484` (no external network ingress)
- **Database Separation:** Two independent SQLite databases on the same file system
- **Storage Hierarchy:** Temp files cleaned up post-download, completed files retained indefinitely (or until user deletes)
- **NNTP Connectivity:** Encrypted TLS connections to external Usenet provider (typically port 563)

---

<!-- anchor: 3-9-4-infrastructure-requirements -->
#### 3.9.4 Infrastructure Requirements

**Hardware Specifications:**

| Component | Minimum | Recommended | Notes |
|-----------|---------|-------------|-------|
| **CPU** | 2 cores @ 2.0 GHz | 4 cores @ 3.0 GHz | yEnc decoding is CPU-intensive |
| **RAM** | 4 GB | 8 GB | 2 GB for OS, 2-4 GB for app + buffers |
| **Storage** | 100 GB SSD/HDD | 1 TB+ SSD | Fast disk for post-processing (PAR2/extraction) |
| **Network** | 100 Mbps | 1 Gbps | Saturate available bandwidth |

**Software Dependencies:**

- **Operating System:** Linux kernel 5.10+ (systemd for service management)
- **Rust Toolchain:** 1.75+ (for building from source)
- **System Libraries:** `openssl` (1.1.1+), `sqlite3` (3.35+)
- **Archive Tools:** `unrar` (any version), `p7zip-full` (16.02+)
- **Optional:** Docker 24.x + Docker Compose 2.x (for containerized deployment)

**Network Requirements:**

- **Outbound Access:**
  - NNTP servers: Port 563 (NNTP over TLS) or 119 (plaintext, not recommended)
  - NTP servers: Port 123 (time synchronization for certificate validation)
- **Inbound Access:**
  - API server: Port 8484 (configurable, localhost-only by default)
  - No external ingress required (user accesses via SSH tunnel or VPN if remote)

**Disk I/O Patterns:**

- **Reads:** Low (only during post-processing verification/extraction)
- **Writes:** High sustained throughput during active downloads (up to 211 MB/s measured)
- **IOPS:** Moderate (SQLite updates, article assembly)
- **Recommendation:** SSD for download/temp directories, HDD acceptable for final storage

**Backup Requirements:**

- **Critical Data (Must Backup):**
  - `/var/lib/spotweb/config/config.json` (contains NNTP credentials)
  - `/var/lib/spotweb/data/spotweb.db` (spot metadata)
  - `/var/lib/spotweb/data/usenet-dl.db` (download history, queue state)
- **Non-Critical (Optional Backup):**
  - Downloaded files (re-downloadable from Usenet if NZB retained)
- **Backup Frequency:** Daily incremental (config/databases change frequently)

---

<!-- anchor: 3-9-5-update-rollback-strategy -->
#### 3.9.5 Update & Rollback Strategy

**Update Process (Native Binary):**

1. **Pre-Update Checklist:**
   - Backup databases: `sqlite3 spotweb.db ".backup spotweb.db.bak"`
   - Note current version: `spotweb-rs --version`
   - Check release notes for breaking changes

2. **Update Steps:**
   ```bash
   # Stop service
   sudo systemctl stop spotweb-rs

   # Pull latest code
   cd /path/to/spotweb-rs
   git pull origin main
   cd ../usenet-dl
   git pull origin main

   # Rebuild
   cd ../spotweb-rs
   cargo build --release

   # Replace binary
   sudo cp target/release/spotweb-rs /usr/local/bin/

   # Start service
   sudo systemctl start spotweb-rs

   # Verify
   sudo systemctl status spotweb-rs
   curl http://127.0.0.1:8484/health
   ```

3. **Rollback (If Update Fails):**
   ```bash
   # Stop broken service
   sudo systemctl stop spotweb-rs

   # Restore old binary (keep previous version as .old)
   sudo cp /usr/local/bin/spotweb-rs.old /usr/local/bin/spotweb-rs

   # Restore database backups (if schema migration failed)
   cp /var/lib/spotweb/data/spotweb.db.bak /var/lib/spotweb/data/spotweb.db

   # Start service
   sudo systemctl start spotweb-rs
   ```

**Update Process (Docker):**

1. **Update Steps:**
   ```bash
   # Pull latest code
   git pull origin main

   # Rebuild image
   docker compose build

   # Stop old container
   docker compose down

   # Start new container
   docker compose up -d

   # Verify
   docker compose logs -f spotweb-rs
   curl http://127.0.0.1:8484/health
   ```

2. **Rollback:**
   ```bash
   # Revert git changes
   git checkout HEAD~1

   # Rebuild previous version
   docker compose build

   # Restart
   docker compose down
   docker compose up -d
   ```

**Database Migrations:**

- **usenet-dl Migrations:** Automatic on startup (managed by sqlx migrations in library)
- **spotweb-rs Migrations:** Manual execution required (if schema changes)
  ```bash
  # Run migrations
  spotweb-rs migrate --database /var/lib/spotweb/data/spotweb.db
  ```

**Zero-Downtime Updates (Future Enhancement):**

Not applicable for single-instance deployment. Downtime during updates is expected and acceptable for Medium-scale self-hosted use case.

