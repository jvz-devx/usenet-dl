# Specification Review & Recommendations: Spotweb-RS Integrated Download Frontend

**Date:** 2026-01-23
**Status:** Awaiting Specification Enhancement

### **1.0 Executive Summary**

This document is an automated analysis of the provided project specifications. It has identified critical decision points that require explicit definition before architectural design can proceed.

**Required Action:** The user is required to review the assertions below and **update the original specification document** to resolve the ambiguities. This updated document will serve as the canonical source for subsequent development phases.

### **2.0 Synthesized Project Vision**

*Based on the provided data, the core project objective is to engineer a system that:*

Integrates the usenet-dl Rust library as a download backend for spotweb-rs, enabling users to queue and manage Usenet downloads directly through a Svelte frontend without manually downloading NZB files, with real-time progress tracking and queue management capabilities.

### **3.0 Critical Assertions & Required Clarifications**

---

#### **Assertion 1: Frontend-Backend Communication Architecture**

*   **Observation:** The specification mandates "connect my svelte frontend to the backend" and "proper download page in the same style" but does not define the frontend's location, deployment strategy, or integration pattern with the Rust backend.
*   **Architectural Impact:** This is a foundational decision affecting development workflow, build pipeline, deployment complexity, and API surface design.
    *   **Path A (Separate SPA):** Svelte frontend as independent application consuming spotweb-rs REST API. Requires CORS configuration, separate build/deploy, enables frontend technology independence.
    *   **Path B (Embedded Static Assets):** Svelte frontend compiled to static assets served by Axum from spotweb-rs. Simplified deployment, single binary distribution, tighter coupling.
    *   **Path C (Hybrid SSR):** Server-side rendering integration using a Rust-Svelte bridge. Maximum performance, significant architectural complexity.
*   **Default Assumption & Required Action:** To minimize initial complexity, the system will be architected assuming **Path B (Embedded Static Assets)** with Axum serving compiled Svelte bundles from a static directory. **The specification must be updated** to explicitly define frontend repository location, build integration strategy, and deployment model.

---

#### **Assertion 2: Download State Synchronization Strategy**

*   **Observation:** The specification mentions "real-time progress via SSE" in the backend plan but does not define how frontend state should be initialized, synchronized on reconnection, or handled during network failures.
*   **Architectural Impact:** This variable dictates client-side state management complexity, backend memory requirements, and user experience during edge cases.
    *   **Tier 1 (SSE-Only):** All state updates pushed via SSE events, frontend polls `/api/downloads` on mount/reconnect. Simple backend, vulnerable to missed events during disconnection.
    *   **Tier 2 (SSE + Snapshot):** SSE stream includes periodic full-state snapshots. Resilient to disconnections, increased bandwidth overhead.
    *   **Tier 3 (Event Sourcing):** Backend maintains replay buffer with sequence IDs, frontend requests missed events by last-seen ID. Maximum reliability, complex implementation.
*   **Default Assumption & Required Action:** The architecture will assume **Tier 1 (SSE-Only)** with frontend reconciliation via GET on reconnect to balance simplicity and reliability. **The specification must be updated** to define acceptable data loss scenarios, reconnection behavior requirements, and offline mode expectations.

---

#### **Assertion 3: NZB Source Integration Point**

*   **Observation:** The specification states "people shouldn't have to download an nzb (unless they want to)" but does not clarify whether users will select spots from an existing spotweb index, upload NZB files directly, or both.
*   **Architectural Impact:** This determines whether spotweb-rs requires a populated spots database, NZB storage infrastructure, and the scope of the "download page" user interface.
    *   **Path A (Spot-to-Download):** Users browse spots in spotweb-rs UI, click download button to queue via existing NZB fetch mechanism. Requires functional spot indexing and NZB API.
    *   **Path B (Manual NZB Upload):** Users can upload NZB files directly to download queue, bypassing spot database. Independent of spotweb functionality.
    *   **Path C (Dual-Mode):** Both spot selection and manual upload supported. Maximum flexibility, increased UI and backend complexity.
*   **Default Assumption & Required Action:** To leverage the existing backend integration plan, the system will assume **Path A (Spot-to-Download)** as primary flow with optional manual upload as future enhancement. **The specification must be updated** to explicitly define whether a populated spotweb spots database is a prerequisite or if manual NZB upload is the primary use case.

---

#### **Assertion 4: Download Queue Persistence and History**

*   **Observation:** The backend plan references usenet-dl's SQLite database for download state, but the specification does not address whether download history should be queryable through the spotweb-rs UI or remain isolated in usenet-dl's domain.
*   **Architectural Impact:** This decision affects database schema design, API surface, and whether download history integrates with spotweb's existing "processed_nzbs" and "history" tables.
    *   **Path A (Isolated Persistence):** usenet-dl manages its own SQLite with no cross-database queries. Simple separation of concerns, potential data duplication for completed downloads.
    *   **Path B (Unified History):** spotweb-rs mirrors completed downloads into its own history tables for unified querying. Single source of truth, requires database synchronization logic.
    *   **Path C (Foreign Data Wrapper):** spotweb-rs queries usenet-dl database directly for history views. Minimal duplication, tight coupling between database schemas.
*   **Default Assumption & Required Action:** The architecture will assume **Path A (Isolated Persistence)** with usenet-dl as authoritative source for download state, exposing history via API only. **The specification must be updated** to define whether download history must integrate with spotweb's existing blacklist/whitelist features or remain a standalone view.

---

#### **Assertion 5: NNTP Credential Management**

*   **Observation:** The backend plan shows conversion from spotweb-rs `NntpServerConfig` to usenet-dl `ServerConfig`, but the specification does not clarify whether NNTP credentials should be configured once globally or support multiple server profiles.
*   **Architectural Impact:** This determines configuration schema complexity, credential storage security requirements, and whether users can balance downloads across multiple providers.
    *   **Tier 1 (Single Global):** One NNTP server configured in spotweb-rs config, shared by all downloads. Simplest implementation, no load distribution.
    *   **Tier 2 (Multi-Server Pool):** Multiple NNTP servers with automatic failover/load balancing. Resilient, requires server selection logic.
    *   **Tier 3 (Per-Download Assignment):** Users can assign specific servers to individual downloads. Maximum control, complex credential and UI management.
*   **Default Assumption & Required Action:** The architecture will assume **Tier 1 (Single Global)** NNTP configuration to match typical SABnzbd deployment patterns. **The specification must be updated** to define whether multi-server support is required for the initial release or can be deferred as an enhancement.

---

#### **Assertion 6: Frontend Styling and Component Library**

*   **Observation:** The specification requires the download page to be "in the same style" but does not reference an existing spotweb-rs UI implementation or design system.
*   **Architectural Impact:** This variable affects whether a UI component library must be selected, custom CSS framework integration is required, or the project assumes an existing frontend codebase.
    *   **Path A (New Greenfield):** No existing spotweb-rs frontend exists, full Svelte application must be built from scratch. Requires UI/UX design decisions, component library selection.
    *   **Path B (Existing Codebase):** spotweb-rs already has a Svelte frontend with established styling that download page must match. Requires codebase inspection to identify component patterns.
    *   **Path C (Minimal Functional):** Unstyled functional implementation acceptable, styling is secondary to backend integration. Fastest path to working prototype.
*   **Default Assumption & Required Action:** Without evidence of existing frontend code in the specifications, the system will assume **Path A (New Greenfield)** with a minimal Svelte application using modern component library like Skeleton UI or Flowbite-Svelte for rapid development. **The specification must be updated** to provide path to existing frontend codebase if one exists, or confirm greenfield development is acceptable.

---

#### **Assertion 7: Error Handling and User Notification Strategy**

*   **Observation:** The backend plan includes download states like "failed" but does not define how errors should be surfaced to users or what retry/recovery mechanisms the UI should expose.
*   **Architectural Impact:** This determines whether the frontend requires a toast notification system, error modal design, and what granularity of error information the backend API must expose.
    *   **Path A (Status-Only):** Download status shows "failed" with generic error message. Simple implementation, limited user actionability.
    *   **Path B (Detailed Error Context):** API exposes specific error types with suggested actions. Better UX, requires structured error modeling.
    *   **Path C (Automated Retry):** usenet-dl handles all retries transparently, frontend only shows final success/failure. Best UX, may mask underlying issues.
*   **Default Assumption & Required Action:** The architecture will assume **Path B (Detailed Error Context)** where the SSE stream and download API expose structured error information enabling frontend to present actionable user guidance. **The specification must be updated** to define acceptable error recovery workflows and whether manual retry controls are required in the UI.

---

### **4.0 Next Steps**

Upon the user's update of the original specification document, the development process will be unblocked and can proceed to the architectural design phase.
