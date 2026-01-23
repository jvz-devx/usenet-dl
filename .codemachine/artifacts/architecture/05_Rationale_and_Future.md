# 05_Rationale_and_Future.md

**Project:** Usenet-dl Integration with spotweb-rs
**Architect:** Operational & Documentation Architect
**Date:** 2026-01-23
**Version:** 1.0

---

<!-- anchor: 4-0-design-rationale-tradeoffs -->
## 4. Design Rationale & Trade-offs

This section documents the key architectural decisions made during the design phase, the alternatives that were considered, and the rationale for rejecting them. It also identifies known risks and proposed mitigation strategies to ensure the architecture remains viable and maintainable.

---

<!-- anchor: 4-1-key-decisions-summary -->
### 4.1 Key Decisions Summary

The following architectural decisions form the foundation of the integrated system. Each decision was made with explicit consideration of the Medium-scale classification, prioritizing rapid development, maintainability, and adequate performance over enterprise-grade abstractions.

<!-- anchor: 4-1-1-monolithic-architecture -->
#### 4.1.1 Monolithic Architecture with Embedded Library

**Decision:** Integrate `usenet-dl` as an embedded library dependency rather than deploying it as a separate microservice.

**Rationale:**
- **Reduced Operational Complexity:** Single binary to deploy, monitor, and update (no inter-service networking, service discovery, or distributed tracing overhead)
- **Shared Resources:** Efficient memory usage by sharing Tokio runtime and connection pools within a single process
- **Development Velocity:** Direct function calls instead of API contracts between services (faster iteration cycles)
- **Deployment Simplicity:** Self-hosted Medium-scale deployments benefit from simpler infrastructure (single systemd service or Docker container)

**Supporting Evidence:**
- The `usenet-dl` library is already designed as a reusable component with clean public APIs
- Existing codebase (~8K lines) demonstrates stability and mature design patterns
- Medium-scale traffic (50-100 API req/sec, 3-10 concurrent downloads) does not justify microservice distribution costs

**Rejected Alternative:** Deploy `usenet-dl` as a standalone microservice communicating via gRPC/REST. This would require:
- Inter-process communication overhead (serialization, network latency)
- Separate deployment artifacts and configuration files
- Distributed tracing and monitoring infrastructure
- Service discovery mechanism (e.g., Consul, etcd)

**Trade-off Accepted:** Tighter coupling between `spotweb-rs` and `usenet-dl` makes independent scaling impossible. This is acceptable because the workload characteristics (browsing spots vs. downloading) do not require separate scaling dimensions at Medium scale.

---

<!-- anchor: 4-1-2-dual-sqlite-databases -->
#### 4.1.2 Dual SQLite Databases (Separate Persistence)

**Decision:** Maintain two independent SQLite databases - one for `spotweb-rs` (spots, comments) and one for `usenet-dl` (downloads, articles, history).

**Rationale:**
- **Library Independence:** `usenet-dl` remains a general-purpose library usable in other projects without spotweb-specific schema coupling
- **Clear Separation of Concerns:** Spot metadata and download state have distinct lifecycles and access patterns
- **Schema Evolution Freedom:** Each database can evolve independently without coordination (no cross-database foreign keys)
- **Backup Granularity:** Users can backup spot data separately from download history (different retention policies)

**Supporting Evidence:**
- SQLite performance is adequate for Medium-scale workloads (tested with 300+ tests in `usenet-dl`)
- Single-writer limitation of SQLite is not a bottleneck (separate databases = separate write locks)
- No transactional requirements across databases (queuing a download does NOT modify spot data)

**Rejected Alternative:** Merge both schemas into a single SQLite database with namespaced tables (e.g., `spotweb_spots`, `usenet_downloads`). This would:
- Improve query efficiency for hypothetical JOIN operations (e.g., "show spots with active downloads")
- Simplify backup procedures (single database file)
- Reduce connection pool overhead

**Why Rejected:**
- Violates library independence principle (usenet-dl would depend on spotweb schema)
- No actual JOIN queries required in the design (API handles associations via application logic)
- Negligible performance benefit at Medium scale (connection pool overhead is minimal)

**Trade-off Accepted:** Slightly higher operational overhead (two databases to backup, monitor, and maintain). Mitigation: Document backup procedures clearly and provide example scripts.

---

<!-- anchor: 4-1-3-sse-realtime-updates -->
#### 4.1.3 Server-Sent Events (SSE) for Real-Time Updates

**Decision:** Use SSE (`text/event-stream`) for real-time download progress updates instead of WebSockets or polling.

**Rationale:**
- **Simplicity:** SSE is unidirectional (server → client), which matches the use case (backend pushes download events, frontend does not send frequent updates)
- **HTTP-Based:** Leverages existing HTTP infrastructure (no WebSocket handshake complexity, compatible with standard reverse proxies)
- **Browser Support:** Native `EventSource` API in all modern browsers (no additional client libraries required)
- **Automatic Reconnection:** Built-in reconnection logic in `EventSource` API (exponential backoff handled by browser)
- **Efficient Resource Usage:** Lower overhead than WebSockets for one-way communication (no full-duplex framing)

**Supporting Evidence:**
- Download events are inherently server-initiated (progress updates, status changes)
- Svelte can consume SSE with ~10 lines of code (minimal boilerplate)
- Medium-scale deployments (5-10 concurrent SSE connections) have negligible memory overhead

**Rejected Alternative 1: Polling** (frontend sends `GET /api/downloads` every 2-5 seconds)
- **Why Rejected:**
  - Wastes bandwidth (fetching full download list when only one item changed)
  - Introduces latency (up to 5-second delay for status updates)
  - Higher server load (unnecessary database queries)
  - Poor user experience (choppy progress bars)

**Rejected Alternative 2: WebSockets** (bidirectional TCP connection)
- **Why Rejected:**
  - Overkill for unidirectional use case (frontend rarely sends commands during active download)
  - Requires additional Axum dependency (`axum-ws`)
  - More complex reconnection logic (need custom ping/pong frames)
  - No significant benefits over SSE for this specific use case

**Trade-off Accepted:** SSE connections remain open for extended periods, consuming one Tokio task per connection. Mitigation: Limit concurrent SSE connections to 10 (close oldest on overflow) and implement automatic reconnection on backend restart.

---

<!-- anchor: 4-1-4-feature-flag-config -->
#### 4.1.4 Configuration-Based Feature Flags (No Runtime Toggle)

**Decision:** Control download subsystem via `download.enabled` boolean in configuration file, requiring service restart to change state.

**Rationale:**
- **Simplicity:** No runtime feature flag infrastructure required (no database table, no admin UI for toggling features)
- **Predictable State:** Service state is deterministic based on configuration (no hidden runtime overrides)
- **Resource Safety:** Disabled features do NOT initialize resources (no NNTP connection pool, no database connections for usenet-dl)
- **Gradual Rollout:** Users can enable downloads in their own deployment schedule (no forced feature activation)

**Supporting Evidence:**
- Medium-scale self-hosted deployments have infrequent configuration changes (manual edits are acceptable)
- Service restarts are fast (<5 seconds) and infrequent (weekly/monthly updates)
- Feature flag state does not need to change dynamically based on runtime conditions

**Rejected Alternative:** Runtime feature flags via admin API (e.g., `POST /api/admin/features` to toggle downloads on/off without restart).
- **Why Rejected:**
  - Introduces state management complexity (need to persist flag state to database)
  - Requires graceful resource cleanup (draining active downloads, closing NNTP connections)
  - Adds security risk (admin API needs authentication/authorization)
  - No compelling use case at Medium scale (configuration file is sufficient)

**Trade-off Accepted:** Changing feature state requires service restart (brief downtime). This is acceptable because:
- Restarts are infrequent (configuration is stable after initial setup)
- Downtime is minimal (<10 seconds with graceful shutdown)
- Medium-scale availability target (99% uptime) accommodates planned maintenance

---

<!-- anchor: 4-1-5-localhost-only-default -->
#### 4.1.5 Localhost-Only Binding by Default (Security First)

**Decision:** Default `listen_addr = "127.0.0.1:8484"` with explicit warnings against external exposure without authentication.

**Rationale:**
- **Security by Default:** Prevents accidental exposure to LAN/internet (common misconfiguration risk)
- **Progressive Disclosure:** Users who need remote access must explicitly change configuration (forces conscious decision)
- **Minimal Attack Surface:** Phase 1 has no authentication - localhost binding is the primary security control
- **Fail-Safe Design:** If user forgets to configure authentication, default binding prevents unauthorized access

**Supporting Evidence:**
- Many self-hosted tools (Plex, Sonarr, Radarr) default to localhost or require explicit binding configuration
- Common attack vector: Web services exposed to internet with default credentials or no authentication
- Medium-scale deployment assumption: Primary use case is local access or VPN-based remote access

**Rejected Alternative:** Default to `0.0.0.0:8484` (bind to all interfaces) with authentication required.
- **Why Rejected:**
  - Requires implementing authentication in Phase 1 (delays MVP delivery)
  - Forces all users to configure authentication even if not needed (increased complexity)
  - Higher risk of misconfiguration (users might disable auth for "testing" and forget to re-enable)

**Trade-off Accepted:** Users wanting remote access must either:
1. Change `listen_addr` to `0.0.0.0` and use VPN/SSH tunnel (recommended)
2. Change `listen_addr` and implement reverse proxy with authentication (advanced users)

This creates a minor usability hurdle for remote access scenarios but significantly improves default security posture.

---

<!-- anchor: 4-1-6-no-frontend-auth-phase-1 -->
#### 4.1.6 Deferred Authentication (Phase 2 Enhancement)

**Decision:** Ship Phase 1 without any authentication/authorization system in the frontend or API.

**Rationale:**
- **Rapid MVP Delivery:** Authentication adds 20-30% development time (user management, password hashing, session storage, UI login forms)
- **Self-Hosted Context:** Primary deployment scenario (home NAS, personal VPS) has single trusted user
- **Network-Level Protection:** Localhost binding + VPN/SSH tunnel provides adequate access control for initial release
- **Design Flexibility:** Deferring auth allows user feedback to inform the best approach (session-based vs. token-based vs. OAuth)

**Supporting Evidence:**
- Many successful self-hosted tools launch without auth (add it based on user demand)
- Foundation document classifies this as Medium-scale (1-3 developers) - minimize scope to deliver faster
- Database schema is designed to be auth-ready (can add `user_id` columns in future migration)

**Rejected Alternative:** Implement basic HTTP authentication (username/password) in Phase 1.
- **Why Rejected:**
  - HTTP Basic Auth over HTTP is insecure (requires HTTPS)
  - Implementing HTTPS requires certificate management (Let's Encrypt, self-signed certs)
  - Adds infrastructure complexity (TLS termination, certificate renewal)
  - Many self-hosters prefer VPN-based access control over built-in auth

**Trade-off Accepted:** Users MUST understand the security implications of external exposure. Mitigation:
- Prominent warnings in documentation
- Log WARNING message if `listen_addr != 127.0.0.1` on startup
- Include "Security Considerations" section in README

**Future Path (Phase 2):**
When authentication is implemented, the design supports:
- Session-based auth with HTTP-only cookies (recommended for web apps)
- SQLite-backed user/session tables (reuse existing database infrastructure)
- Axum middleware for authentication enforcement (minimal code changes to existing handlers)

---

<!-- anchor: 4-1-7-priority-queue-ordering -->
#### 4.1.7 Priority-Based Queue with Force Option

**Decision:** Implement priority-based download queue with four levels: `low`, `normal`, `high`, `force`.

**Rationale:**
- **User Control:** Allows prioritizing important downloads (e.g., time-sensitive content, incomplete series)
- **Minimal Complexity:** Enum-based priority system (4 levels is simple to understand and implement)
- **Force Override:** `force` priority jumps to front of queue (emergency downloads bypass normal ordering)
- **Default Sensible:** `normal` priority for most downloads (users only set priority when needed)

**Supporting Evidence:**
- SABnzbd and NZBGet (industry-standard Usenet downloaders) use similar priority systems
- `usenet-dl` already implements priority queue via `BinaryHeap` (design reuses existing capability)
- User research: Priority is frequently requested feature in download managers

**Rejected Alternative 1:** FIFO queue only (no priority support).
- **Why Rejected:**
  - Poor user experience (cannot expedite urgent downloads)
  - Common scenario: User queues 20 large downloads, then wants one small file immediately
  - Competitive disadvantage (users expect priority control in modern download managers)

**Rejected Alternative 2:** Numeric priority (1-10 scale).
- **Why Rejected:**
  - Decision paralysis (users unsure whether to use 7 or 8 for "important")
  - Harder to communicate in UI (dropdown with 10 options vs. 4 semantic labels)
  - No clear behavioral difference between adjacent levels (what's the difference between 5 and 6?)

**Trade-off Accepted:** Four priority levels may not cover all edge cases (e.g., user wants 7 distinct priority levels). This is acceptable because:
- Medium-scale use case assumes small queue sizes (10-50 downloads, not thousands)
- Users can manually reorder queue in future UI enhancement (drag-and-drop reordering)
- `force` priority provides escape hatch for truly urgent cases

---

<!-- anchor: 4-2-alternatives-considered -->
### 4.2 Alternatives Considered

This section documents significant architectural alternatives that were evaluated but ultimately rejected. Understanding these alternatives provides insight into the constraints and priorities that shaped the final design.

<!-- anchor: 4-2-1-microservices-architecture -->
#### 4.2.1 Microservices Architecture (Rejected)

**Alternative Design:**
- Deploy `spotweb-rs` and `usenet-dl` as separate services communicating via gRPC
- Use PostgreSQL for shared database (instead of separate SQLite files)
- Deploy services in Kubernetes with service mesh (Istio/Linkerd)
- Implement distributed tracing (Jaeger) and centralized logging (Loki)

**Why Considered:**
- Industry trend toward microservices for "modern" architectures
- Independent scalability (scale download workers separately from API frontend)
- Technology heterogeneity (could rewrite download service in Go/Rust while keeping API in Rust)
- Fault isolation (download service crash doesn't affect spot browsing)

**Why Rejected:**

1. **Over-Engineering for Medium Scale:**
   - Microservices add complexity (service discovery, API versioning, distributed transactions)
   - Medium-scale workload (3-10 concurrent downloads) does NOT require independent scaling
   - Single-instance monolith handles target load with headroom (tested up to 211 MB/s throughput)

2. **Operational Overhead:**
   - Kubernetes cluster requires dedicated infrastructure (3+ nodes for HA)
   - Service mesh adds latency (sidecar proxies, network hops)
   - Distributed tracing requires additional services (Jaeger, Tempo)
   - Increases MTTR (Mean Time To Recovery) for failures (more components to debug)

3. **Development Velocity:**
   - Inter-service API contracts slow iteration (need to coordinate schema changes)
   - Integration testing requires spinning up multiple services
   - Debugging distributed systems is harder (need correlated trace IDs)

4. **Self-Hosted Deployment Context:**
   - Target users run on single NAS/VPS (no Kubernetes cluster available)
   - Users want "install and run" experience (single binary or Docker container)
   - Microservices require docker-compose with multiple containers (confusing for non-experts)

**Lessons Applied to Final Design:**
- Keep architecture simple (monolith) but maintain modularity (clean service boundaries)
- Design API contracts even within monolith (prepare for future extraction if needed)
- Use feature flags to disable subsystems (achieve some fault isolation benefits)

---

<!-- anchor: 4-2-2-postgresql-database -->
#### 4.2.2 PostgreSQL Database (Rejected)

**Alternative Design:**
- Use PostgreSQL 15+ instead of SQLite for both spotweb and usenet-dl databases
- Unified database with shared schema (single source of truth)
- Connection pooling via PgBouncer
- Streaming replication for high availability

**Why Considered:**
- Better concurrency (multi-writer support, no write-lock contention)
- Advanced features (full-text search, JSON operators, window functions)
- Horizontal scalability (read replicas, sharding)
- Industry standard for "serious" applications

**Why Rejected:**

1. **Deployment Complexity:**
   - PostgreSQL requires separate service (docker-compose with postgres container OR system postgres installation)
   - Connection string configuration (host, port, username, password, database)
   - Backup complexity (pg_dump, WAL archiving)
   - Version compatibility issues (client library vs. server version)

2. **Operational Overhead:**
   - Requires database administration (vacuum, analyze, index maintenance)
   - Monitoring (connection pool exhaustion, slow queries)
   - Security (database user permissions, network access control)

3. **Resource Usage:**
   - PostgreSQL consumes more memory (shared_buffers, work_mem per connection)
   - Higher disk usage (MVCC overhead, index size)
   - Medium-scale workload does NOT benefit from postgres's concurrency features (low write volume)

4. **Alignment with usenet-dl Design:**
   - `usenet-dl` library is designed for SQLite (migration system uses sqlx with SQLite-specific pragmas)
   - Changing to PostgreSQL would require forking the library (violates reusability goal)
   - No actual concurrency issues with SQLite at Medium scale (separate databases = separate locks)

**Lessons Applied to Final Design:**
- Document SQLite performance characteristics (when it's adequate, when to migrate)
- Use WAL mode for better concurrency (already standard in modern SQLite)
- Design schema to be migration-ready (if future scale requires PostgreSQL)

---

<!-- anchor: 4-2-3-graphql-api -->
#### 4.2.3 GraphQL API (Rejected)

**Alternative Design:**
- Replace REST endpoints with GraphQL API (single `/graphql` endpoint)
- Use `async-graphql` crate for Rust GraphQL server
- Frontend uses Apollo Client for data fetching and caching

**Why Considered:**
- Flexible queries (frontend requests exactly the fields it needs)
- Reduced over-fetching (no need to fetch entire download list to check status)
- Real-time subscriptions (GraphQL subscriptions for download events, instead of SSE)
- Typed schema (auto-generated TypeScript types for frontend)

**Why Rejected:**

1. **Complexity Without Benefit:**
   - API surface is small (~7 endpoints) - REST is perfectly adequate
   - No complex relationship traversal (no deeply nested data structures)
   - Frontend queries are simple (list all downloads, get single download by ID)

2. **Over-Fetching Not a Problem:**
   - Download list payload is small (100 downloads × 500 bytes = 50 KB, easily fits in single request)
   - No pagination required at Medium scale (users rarely have >100 downloads)
   - Network bandwidth is not a bottleneck (localhost or LAN deployment)

3. **Real-Time Subscriptions Complexity:**
   - GraphQL subscriptions require WebSocket setup (same complexity as SSE, but with GraphQL overhead)
   - SSE is simpler for unidirectional push (no need for GraphQL subscription resolvers)

4. **Development Overhead:**
   - Learning curve for GraphQL schema design
   - Frontend requires Apollo Client setup (additional bundle size)
   - Harder to test (need GraphQL query IDE like GraphiQL)

**Lessons Applied to Final Design:**
- Use standard REST conventions (predictable, well-documented)
- Leverage OpenAPI for typed contracts (similar benefits to GraphQL schema without complexity)
- Keep API surface small and focused (7 endpoints is manageable with REST)

---

<!-- anchor: 4-3-known-risks-mitigation -->
### 4.3 Known Risks & Mitigation

This section identifies potential risks to the architecture's success and proposes mitigation strategies. Risks are categorized by likelihood and impact.

<!-- anchor: 4-3-1-risk-sqlite-scaling-limits -->
#### 4.3.1 Risk: SQLite Scaling Limits

**Likelihood:** Medium
**Impact:** High
**Severity:** **Medium-High**

**Description:**
SQLite's single-writer limitation may become a bottleneck if:
- Concurrent download count exceeds 20 (high write contention on downloads table)
- Frontend polling frequency increases (frequent read queries block writes)
- Download history grows unbounded (query performance degrades over time)

**Symptoms:**
- API response latency exceeds 500ms for `/api/downloads` endpoint
- Database locked errors in logs (`SQLITE_BUSY` errors)
- Slow download progress updates (writes blocked by long-running reads)

**Mitigation Strategies:**

1. **Immediate (Phase 1):**
   - Enable WAL mode (Write-Ahead Logging) for better read/write concurrency
   - Add database indexes on frequently queried columns (`status`, `priority`, `created_at`)
   - Implement read query optimization (limit list downloads to 100 most recent)
   - Rate-limit progress updates (max 1 write per second per download)

2. **Short-Term (Phase 2):**
   - Implement download history pruning (auto-delete completed downloads older than 90 days)
   - Add pagination to `/api/downloads` endpoint (reduce query result set size)
   - Use database connection pooling with max 10 connections (prevent connection exhaustion)

3. **Long-Term (If Scale Exceeds Medium):**
   - Migrate to PostgreSQL for downloads database (keep spotweb SQLite for read-heavy spot data)
   - Implement read replicas (PostgreSQL streaming replication)
   - Consider splitting downloads table (active downloads vs. historical downloads)

**Early Warning Indicators:**
- Monitor database query latency (alert if P95 > 100ms)
- Track `SQLITE_BUSY` error rate (alert if >10 errors per hour)
- Database file size growth (alert if >1 GB, indicates need for pruning)

---

<!-- anchor: 4-3-2-risk-sse-connection-overhead -->
#### 4.3.2 Risk: SSE Connection Resource Exhaustion

**Likelihood:** Low
**Impact:** Medium
**Severity:** **Low-Medium**

**Description:**
Each SSE connection consumes:
- One Tokio task (small memory overhead, ~2-4 KB per task)
- One broadcast channel subscription (negligible memory)
- HTTP connection keep-alive (TCP socket held open)

If many clients connect simultaneously (e.g., user opens 20 browser tabs), resource usage could grow unexpectedly.

**Symptoms:**
- Memory usage grows linearly with SSE connections
- Tokio runtime reports high task count (thousands of active tasks)
- Connection limit errors in logs

**Mitigation Strategies:**

1. **Immediate (Phase 1):**
   - Limit concurrent SSE connections to 10 per instance
   - Implement connection eviction (close oldest connection when limit reached)
   - Log WARNING when connection limit approached

2. **Short-Term (Phase 2):**
   - Add connection health checks (close idle connections after 5 minutes)
   - Implement connection authentication (prevent unauthorized SSE subscriptions)
   - Monitor connection count (expose in `/api/stats` endpoint)

3. **Long-Term:**
   - Consider WebSocket upgrade with message compression (reduce bandwidth for high-frequency updates)
   - Implement connection rate limiting (max 1 new SSE connection per 10 seconds per IP)

**Early Warning Indicators:**
- Monitor active SSE connection count (alert if >10)
- Track memory usage per connection (baseline: 4 KB, alert if >20 KB)

---

<!-- anchor: 4-3-3-risk-nntp-provider-throttling -->
#### 4.3.3 Risk: NNTP Provider Rate Limiting/Throttling

**Likelihood:** Medium
**Impact:** Medium
**Severity:** **Medium**

**Description:**
Usenet providers may implement:
- Connection count limits (e.g., max 30 concurrent connections)
- Bandwidth throttling (speed caps after quota exceeded)
- Anti-abuse detection (temporary blocks for "suspicious" activity)

This could cause downloads to stall or fail unexpectedly.

**Symptoms:**
- Downloads pause with "connection refused" errors
- Slow download speeds despite available bandwidth
- Frequent NNTP disconnections

**Mitigation Strategies:**

1. **Immediate (Phase 1):**
   - Document recommended NNTP configuration (connection limits per provider)
   - Implement exponential backoff retry (already in `usenet-dl`)
   - Log WARN on repeated connection failures (alert user to check provider status)

2. **Short-Term (Phase 2):**
   - Add multiple NNTP provider support (failover to backup provider)
   - Implement adaptive connection scaling (reduce connections if errors detected)
   - Provider-specific configuration profiles (presets for Newshosting, Eweka, etc.)

3. **Long-Term:**
   - Intelligent provider selection (route downloads to least-loaded provider)
   - Provider health monitoring (track error rates per provider)

**Early Warning Indicators:**
- Monitor NNTP error rate (alert if >5% of article fetches fail)
- Track average download speed (alert if drops below 50% of expected speed)

---

<!-- anchor: 4-3-4-risk-archive-extraction-vulnerabilities -->
#### 4.3.4 Risk: Archive Extraction Security Vulnerabilities

**Likelihood:** Low
**Impact:** High
**Severity:** **Medium**

**Description:**
External extraction tools (`unrar`, `7z`) may have security vulnerabilities:
- Zip bombs (compressed file expands to terabytes, fills disk)
- Path traversal (malicious archive extracts to `/etc/passwd`)
- Buffer overflows in extraction tools (arbitrary code execution)

**Symptoms:**
- Disk space exhaustion after extraction
- Files extracted outside download directory
- Extraction process crashes or hangs

**Mitigation Strategies:**

1. **Immediate (Phase 1):**
   - Validate extraction paths (reject paths with `..`, absolute paths)
   - Set disk space limits (check available space before extraction)
   - Run extraction with restricted user permissions (systemd `ProtectSystem=strict`)

2. **Short-Term (Phase 2):**
   - Implement extraction size limits (reject if uncompressed size >100 GB)
   - Sandbox extraction process (use `bubblewrap` or similar containerization)
   - Scan archive contents before extraction (list files, validate paths)

3. **Long-Term:**
   - Use memory-safe extraction libraries (Rust-based archive crates instead of external tools)
   - Implement virus scanning integration (ClamAV scan after extraction)

**Early Warning Indicators:**
- Monitor disk usage during extraction (alert if usage spikes >10 GB/minute)
- Track extraction failures (alert if >10% of extractions fail)

---

<!-- anchor: 4-3-5-risk-dependency-vulnerabilities -->
#### 4.3.5 Risk: Dependency Security Vulnerabilities

**Likelihood:** Medium
**Impact:** Medium
**Severity:** **Medium**

**Description:**
Rust crate dependencies may have known security vulnerabilities:
- SQLx, Axum, Tokio, etc. - all have potential CVEs
- Transitive dependencies (dependencies of dependencies) may be unmaintained

**Symptoms:**
- `cargo-audit` reports known vulnerabilities
- Security advisories published for dependencies
- Unexpected crashes or behavior changes

**Mitigation Strategies:**

1. **Immediate (Phase 1):**
   - Run `cargo-audit` before every release
   - Pin exact dependency versions in `Cargo.lock` (committed to repository)
   - Review dependency update changelogs before upgrading

2. **Short-Term (Phase 2):**
   - Automate `cargo-audit` in CI pipeline (fail build if HIGH/CRITICAL vulnerabilities)
   - Set up Dependabot alerts (GitHub automated security updates)
   - Establish update policy (security patches within 7 days, minor updates monthly)

3. **Long-Term:**
   - Minimize dependency count (prefer std library when possible)
   - Fork critical dependencies if unmaintained (take ownership of security patches)

**Early Warning Indicators:**
- Monitor RustSec advisory database (subscribe to security mailing lists)
- Track dependency update frequency (alert if dependency >1 year without updates)

---

<!-- anchor: 5-0-future-considerations -->
## 5. Future Considerations

This section outlines potential evolution paths for the architecture, areas requiring deeper design work, and features deferred beyond the initial MVP release.

---

<!-- anchor: 5-1-potential-evolution -->
### 5.1 Potential Evolution

<!-- anchor: 5-1-1-authentication-authorization-phase-2 -->
#### 5.1.1 Authentication & Authorization (Phase 2)

**Trigger Condition:** User requests remote access without VPN/SSH tunnel.

**Implementation Plan:**

1. **Database Schema Extensions:**
   ```sql
   CREATE TABLE users (
       id INTEGER PRIMARY KEY AUTOINCREMENT,
       username TEXT UNIQUE NOT NULL,
       password_hash TEXT NOT NULL,  -- argon2id
       role TEXT NOT NULL,  -- 'admin' | 'user'
       created_at TEXT NOT NULL
   );

   CREATE TABLE sessions (
       id TEXT PRIMARY KEY,  -- UUID v4
       user_id INTEGER NOT NULL,
       expires_at TEXT NOT NULL,
       created_at TEXT NOT NULL,
       FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
   );

   -- Add user ownership to downloads
   ALTER TABLE downloads ADD COLUMN user_id INTEGER REFERENCES users(id);
   ```

2. **Backend Changes:**
   - Add `AuthService` to handle login/logout/session validation
   - Implement Axum middleware for session validation
   - Add endpoints: `POST /api/auth/login`, `POST /api/auth/logout`, `GET /api/auth/me`
   - Update download handlers to filter by `user_id` (multi-tenancy support)

3. **Frontend Changes:**
   - Add login page (username/password form)
   - Store session cookie (HTTP-only, secure flag if HTTPS)
   - Handle 401 responses (redirect to login page)
   - Add user profile dropdown (logout button)

4. **Security Considerations:**
   - Require HTTPS for external deployment (Let's Encrypt integration docs)
   - Implement CSRF protection (double-submit cookie pattern)
   - Rate-limit login attempts (max 5 attempts per 15 minutes per IP)

**Estimated Effort:** 40-60 hours (2-3 weeks for single developer)

---

<!-- anchor: 5-1-2-multi-server-support -->
#### 5.1.2 Multiple NNTP Server Support (Phase 2)

**Trigger Condition:** User has multiple Usenet provider accounts (primary + backup, or primary + block account).

**Implementation Plan:**

1. **Configuration Schema:**
   ```json
   {
     "nntp_servers": [
       {
         "name": "primary",
         "host": "news.provider1.com",
         "port": 563,
         "username": "user1",
         "password": "pass1",
         "ssl": true,
         "max_connections": 30,
         "priority": 1  // Higher number = preferred
       },
       {
         "name": "backup",
         "host": "news.provider2.com",
         "port": 563,
         "username": "user2",
         "password": "pass2",
         "ssl": true,
         "max_connections": 10,
         "priority": 2  // Fallback server
       }
     ]
   }
   ```

2. **Backend Changes (usenet-dl Library):**
   - Modify `UsenetDownloader` to accept multiple server configurations
   - Implement round-robin or priority-based server selection
   - Automatic failover (if article fetch fails on primary, retry on backup)
   - Track per-server statistics (success rate, average speed)

3. **Frontend Changes:**
   - Display active NNTP server in download details
   - Server health indicator (connection status, error rate)

**Estimated Effort:** 20-30 hours (1-2 weeks)

---

<!-- anchor: 5-1-3-advanced-search-filtering -->
#### 5.1.3 Advanced Download Search & Filtering (Phase 3)

**Trigger Condition:** User accumulates >100 downloads in history.

**Implementation Plan:**

1. **Backend Changes:**
   - Add `/api/downloads/search` endpoint with query parameters:
     - `?query=ubuntu` (full-text search on download name)
     - `?status=completed` (filter by status)
     - `?category=tv` (filter by category)
     - `?from=2026-01-01&to=2026-01-31` (date range)
   - Implement SQLite FTS5 (full-text search) for download names:
     ```sql
     CREATE VIRTUAL TABLE downloads_fts USING fts5(
         id UNINDEXED,
         name,
         category
     );
     ```

2. **Frontend Changes:**
   - Add search bar to download page (autocomplete suggestions)
   - Filter chips (status, category, date range)
   - Sort controls (name, date, size, speed)

**Estimated Effort:** 15-25 hours (1 week)

---

<!-- anchor: 5-1-4-rss-automation -->
#### 5.1.4 RSS Feed Automation (Phase 3)

**Trigger Condition:** User wants automatic downloads based on RSS feed patterns (TV show automation, similar to Sonarr).

**Implementation Plan:**

`usenet-dl` already includes RSS infrastructure (`rss_manager.rs`, `rss_scheduler.rs`). Integration steps:

1. **Backend Changes:**
   - Add API endpoints:
     - `GET /api/rss/feeds` (list configured feeds)
     - `POST /api/rss/feeds` (add new feed)
     - `POST /api/rss/feeds/{id}/filters` (add download filters)
   - Enable RSS scheduler in `usenet-dl` configuration
   - Implement NZB auto-fetch (RSS item → fetch NZB → queue download)

2. **Frontend Changes:**
   - RSS management page (add feeds, configure filters)
   - Show RSS-triggered downloads with source feed label

**Estimated Effort:** 30-40 hours (2 weeks)

---

<!-- anchor: 5-1-5-notification-system -->
#### 5.1.5 Notification System (Phase 3)

**Trigger Condition:** User wants alerts for download completion/failure.

**Implementation Plan:**

1. **Backend Changes:**
   - Add notification service supporting multiple channels:
     - Email (SMTP integration)
     - Webhook (POST to custom URL)
     - Desktop notifications (via browser Notifications API)
   - Configuration:
     ```json
     {
       "notifications": {
         "email": {
           "enabled": true,
           "smtp_host": "smtp.gmail.com",
           "smtp_port": 587,
           "from": "spotweb@example.com",
           "to": "user@example.com"
         },
         "webhook": {
           "enabled": false,
           "url": "https://example.com/webhook"
         }
       }
     }
     ```

2. **Frontend Changes:**
   - Browser notification permission prompt
   - Notification settings page (configure channels)

**Estimated Effort:** 20-30 hours (1-2 weeks)

---

<!-- anchor: 5-2-areas-for-deeper-dive -->
### 5.2 Areas for Deeper Dive

The following areas require additional research and detailed design before implementation.

<!-- anchor: 5-2-1-cicd-pipeline-design -->
#### 5.2.1 CI/CD Pipeline Design

**Current State:** Manual builds and testing.

**Future Requirements:**
- Automated builds on commit (GitHub Actions or GitLab CI)
- Automated testing (cargo test + clippy + fmt)
- Docker image builds and publishing (GitHub Container Registry)
- Release automation (git tag → build artifacts → GitHub Releases)
- Security scanning (cargo-audit, dependency vulnerability checks)

**Research Needed:**
- Multi-architecture builds (x86_64, ARM64 for Raspberry Pi)
- Cargo caching strategies (speed up CI builds)
- Integration test execution (requires NNTP credentials in CI secrets)

---

<!-- anchor: 5-2-2-performance-benchmarking -->
#### 5.2.2 Performance Benchmarking & Optimization

**Current State:** Informal testing (e2e tests with real NZBs, speedtest example shows 211 MB/s).

**Future Requirements:**
- Establish performance baselines (download speed, API latency, memory usage)
- Automated performance regression testing (detect slowdowns in CI)
- Profiling infrastructure (flamegraphs for CPU profiling, memory profiling)
- Load testing (simulate 10 concurrent downloads + API traffic)

**Research Needed:**
- Benchmark framework (`criterion` for micro-benchmarks)
- Load testing tools (wrk for API, custom script for download simulation)
- Profiling integration (perf, valgrind, heaptrack)

---

<!-- anchor: 5-2-3-observability-platform -->
#### 5.2.3 Observability Platform (Prometheus + Grafana)

**Current State:** Structured logging only, basic `/api/stats` endpoint.

**Future Requirements:**
- Prometheus metrics exporter (`/metrics` endpoint in OpenMetrics format)
- Grafana dashboard templates (pre-built dashboards for spotweb-rs)
- Alerting rules (download failure rate, disk space low, etc.)

**Metrics to Export:**
- Download metrics: active count, completed count, failure rate, average speed
- API metrics: request rate, error rate, P50/P95/P99 latency
- System metrics: CPU usage, memory usage, disk I/O, network throughput
- NNTP metrics: connection pool size, article fetch success rate

**Research Needed:**
- Prometheus client library for Rust (prometheus crate)
- Grafana provisioning (automated dashboard import)
- Alertmanager integration (email, Slack, webhook alerts)

---

<!-- anchor: 5-2-4-database-migration-strategy -->
#### 5.2.4 Database Migration Strategy (SQLite → PostgreSQL)

**Current State:** SQLite with manual schema evolution.

**Trigger for Migration:**
- Download count exceeds 10,000 (query performance degradation)
- Concurrent user count exceeds 10 (write lock contention)
- Need for advanced features (full-text search, JSON queries)

**Migration Plan (Outline):**
1. **Schema Translation:** Convert SQLite schema to PostgreSQL-compatible DDL
2. **Data Export:** Use `sqlite3 .dump` to export data
3. **Data Import:** Transform dump to PostgreSQL format (data type differences)
4. **Code Changes:** Update sqlx queries (PostgreSQL-specific syntax)
5. **Testing:** Validate data integrity, performance benchmarks
6. **Rollback Plan:** Keep SQLite as fallback for 30 days post-migration

**Research Needed:**
- SQLite → PostgreSQL migration tools (pgloader, custom script)
- Data validation strategies (checksum verification)
- Performance comparison (before/after benchmarks)

---

<!-- anchor: 5-2-5-frontend-progressive-web-app -->
#### 5.2.5 Frontend Progressive Web App (PWA) Capabilities

**Current State:** Standard Svelte SPA (Single Page Application).

**Future Capabilities:**
- Offline support (service worker caching for static assets)
- Install as desktop app (PWA manifest for "Add to Home Screen")
- Background sync (queue download requests while offline, sync when online)
- Push notifications (via service worker, requires backend integration)

**Research Needed:**
- Svelte PWA libraries (vite-plugin-pwa)
- Service worker strategies (cache-first vs. network-first)
- Push notification infrastructure (requires HTTPS + Push API server)

---

<!-- anchor: 6-0-glossary -->
## 6. Glossary

**API (Application Programming Interface):** A set of defined rules and protocols that enable different software components to communicate. In this project, the REST API allows the Svelte frontend to interact with the Rust backend.

**Axum:** A Rust web framework for building HTTP servers, used in `spotweb-rs` for handling API requests and serving the frontend.

**C4 Model:** A lightweight approach to software architecture diagramming (Context, Container, Component, Code). Used in this project for visualizing system structure.

**CRC32:** A checksum algorithm used in yEnc encoding to verify article integrity. Each article includes a CRC32 hash for validation.

**Docker:** A containerization platform that packages applications and dependencies into portable containers. Optional deployment method for spotweb-rs.

**ERD (Entity-Relationship Diagram):** A visual representation of database schema, showing tables, columns, and relationships. Used in the structural architecture document.

**gRPC:** A high-performance RPC (Remote Procedure Call) framework. Considered but rejected for inter-service communication in favor of monolithic architecture.

**HTTP:** Hypertext Transfer Protocol, the foundation of web communication. Used for all API requests between frontend and backend.

**JSON (JavaScript Object Notation):** A lightweight data interchange format. Used for API requests/responses and configuration files.

**Kubernetes (K8s):** A container orchestration platform for managing clustered deployments. Considered but rejected (overkill for Medium-scale self-hosted deployment).

**Medium-Scale:** Project classification indicating moderate complexity (tens of KLOC, 1-3 developers, weeks-to-months timeline). Dictates architecture simplicity over enterprise patterns.

**Microservices:** An architectural pattern where applications are composed of small, independent services communicating over networks. Rejected in favor of monolithic design for this project.

**Monolith:** An architectural pattern where all components run in a single process. Chosen for this project due to simplicity and adequate performance at Medium scale.

**MVP (Minimum Viable Product):** The initial release with core features only. Phase 1 focuses on essential download functionality, deferring authentication and advanced features.

**NZB:** An XML file format that describes Usenet binary posts. Contains article metadata (message IDs, segment numbers, file names).

**NNTP (Network News Transfer Protocol):** The protocol used to communicate with Usenet servers. Typically uses port 119 (plaintext) or 563 (TLS).

**OpenAPI:** A specification for describing REST APIs. Used with Swagger UI to provide interactive API documentation for spotweb-rs.

**PAR2:** Parity Archive Volume Set, a file verification and repair format. Used by usenet-dl to detect and fix corrupted downloads.

**PostgreSQL:** An open-source relational database. Considered but rejected in favor of SQLite for Phase 1 (deferred to future scaling needs).

**RBAC (Role-Based Access Control):** An authorization model where permissions are assigned to roles (e.g., admin, user). Planned for Phase 2 authentication.

**REST (Representational State Transfer):** An architectural style for web APIs using HTTP methods (GET, POST, DELETE) and resource-based URLs.

**Rust:** A systems programming language focused on safety, concurrency, and performance. Used for both spotweb-rs and usenet-dl.

**SQLite:** A lightweight, serverless SQL database engine. Used for both spotweb and usenet-dl data persistence in Phase 1.

**SSE (Server-Sent Events):** A standard for servers to push real-time updates to browsers over HTTP. Used for download progress events.

**Svelte:** A modern JavaScript framework for building reactive user interfaces. Used for the spotweb-rs frontend.

**systemd:** A Linux system and service manager. Used to run spotweb-rs as a background service on native deployments.

**Tokio:** An asynchronous runtime for Rust, providing async I/O, task scheduling, and concurrency primitives. Foundation of usenet-dl and spotweb-rs.

**TOML:** A configuration file format (Tom's Obvious Minimal Language). Alternative to JSON, not used in this project.

**Usenet:** A worldwide distributed discussion system consisting of newsgroups. Used for binary file sharing via encoded posts.

**VPN (Virtual Private Network):** An encrypted network tunnel. Recommended for remote access to spotweb-rs instead of exposing it to the internet.

**WAL (Write-Ahead Logging):** A SQLite journaling mode that improves concurrency by allowing simultaneous reads and writes. Enabled by default in this project.

**WebSocket:** A full-duplex communication protocol over TCP. Considered but rejected in favor of SSE for real-time updates.

**yEnc:** A binary-to-text encoding format optimized for Usenet. Reduces file size overhead compared to older encodings (uuencode, Base64).

---

**END OF RATIONALE & FUTURE CONSIDERATIONS DOCUMENT**

*This document captures the strategic thinking behind the architecture, alternative paths explored, and the roadmap for future enhancements. It serves as a historical record of design decisions and a guide for architects working on future phases of the project.*
