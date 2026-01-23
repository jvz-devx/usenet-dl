//! Route handlers for the REST API

use super::AppState;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, sse::{Event as SseEvent, KeepAlive, Sse}},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use utoipa;

// ============================================================================
// Query/Request Types
// ============================================================================

/// Query parameters for DELETE /downloads/:id
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct DeleteDownloadQuery {
    /// Whether to delete downloaded files (default: false)
    #[serde(default)]
    pub delete_files: bool,
}

/// Query parameters for GET /history
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct HistoryQuery {
    /// Maximum number of items to return (default: 50)
    pub limit: Option<i64>,
    /// Number of items to skip (default: 0)
    pub offset: Option<i64>,
    /// Filter by status: "complete" or "failed"
    pub status: Option<String>,
}

/// Query parameters for DELETE /history
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ClearHistoryQuery {
    /// Clear entries before this timestamp
    pub before: Option<i64>,
    /// Clear only entries with this status: "complete" or "failed"
    pub status: Option<String>,
}

/// Request body for PUT /config/speed-limit
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct SetSpeedLimitRequest {
    /// Speed limit in bytes per second. Use null for unlimited.
    pub limit_bps: Option<u64>,
}

// ============================================================================
// Queue Management - Downloads
// ============================================================================

/// GET /downloads - List all downloads
#[utoipa::path(
    get,
    path = "/api/v1/downloads",
    tag = "downloads",
    responses(
        (status = 200, description = "List of all downloads", body = Vec<crate::types::DownloadInfo>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_downloads(State(state): State<AppState>) -> impl IntoResponse {
    // Query all downloads from database
    match state.downloader.db.list_downloads().await {
        Ok(downloads) => {
            // Convert Download records to DownloadInfo
            let download_infos: Vec<crate::types::DownloadInfo> = downloads
                .into_iter()
                .map(|d| {
                    // Calculate ETA if download is in progress and speed > 0
                    let eta_seconds = if d.speed_bps > 0 && d.status == 1 {
                        // Status 1 = Downloading
                        let remaining = d.size_bytes.saturating_sub(d.downloaded_bytes);
                        if remaining > 0 {
                            Some((remaining as u64) / (d.speed_bps as u64))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    crate::types::DownloadInfo {
                        id: d.id,
                        name: d.name,
                        category: d.category,
                        status: crate::types::Status::from_i32(d.status),
                        progress: d.progress,
                        speed_bps: d.speed_bps as u64,
                        size_bytes: d.size_bytes as u64,
                        downloaded_bytes: d.downloaded_bytes as u64,
                        eta_seconds,
                        priority: crate::types::Priority::from_i32(d.priority),
                        created_at: chrono::DateTime::from_timestamp(d.created_at, 0)
                            .unwrap_or_else(|| chrono::Utc::now()),
                        started_at: d
                            .started_at
                            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0)),
                    }
                })
                .collect();

            (StatusCode::OK, Json(download_infos))
        }
        Err(e) => {
            tracing::error!("Failed to list downloads: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(vec![]), // Return empty array on error
            )
        }
    }
}

/// GET /downloads/:id - Get single download
#[utoipa::path(
    get,
    path = "/api/v1/downloads/{id}",
    tag = "downloads",
    params(
        ("id" = i64, Path, description = "Download ID")
    ),
    responses(
        (status = 200, description = "Download information", body = crate::types::DownloadInfo),
        (status = 404, description = "Download not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    // Query download by ID from database
    match state.downloader.db.get_download(id).await {
        Ok(Some(d)) => {
            // Calculate ETA if download is in progress and speed > 0
            let eta_seconds = if d.speed_bps > 0 && d.status == 1 {
                // Status 1 = Downloading
                let remaining = d.size_bytes.saturating_sub(d.downloaded_bytes);
                if remaining > 0 {
                    Some((remaining as u64) / (d.speed_bps as u64))
                } else {
                    None
                }
            } else {
                None
            };

            let download_info = crate::types::DownloadInfo {
                id: d.id,
                name: d.name,
                category: d.category,
                status: crate::types::Status::from_i32(d.status),
                progress: d.progress,
                speed_bps: d.speed_bps as u64,
                size_bytes: d.size_bytes as u64,
                downloaded_bytes: d.downloaded_bytes as u64,
                eta_seconds,
                priority: crate::types::Priority::from_i32(d.priority),
                created_at: chrono::DateTime::from_timestamp(d.created_at, 0)
                    .unwrap_or_else(|| chrono::Utc::now()),
                started_at: d
                    .started_at
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0)),
            };

            (StatusCode::OK, Json(download_info)).into_response()
        }
        Ok(None) => {
            // Download not found
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "download not found"})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to get download {}: {}", id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal server error"})),
            )
                .into_response()
        }
    }
}

/// POST /downloads - Add NZB from file upload
#[utoipa::path(
    post,
    path = "/api/v1/downloads",
    tag = "downloads",
    request_body(content = Vec<u8>, description = "NZB file upload (multipart/form-data)", content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Download added successfully", body = i64),
        (status = 400, description = "Invalid NZB file"),
        (status = 422, description = "Unprocessable entity"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn add_download(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    // Parse multipart form data
    let mut nzb_content: Option<Vec<u8>> = None;
    let mut nzb_filename: Option<String> = None;
    let mut options_json: Option<String> = None;

    // Extract all multipart fields
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "file" => {
                // Get filename if present
                if let Some(filename) = field.file_name() {
                    nzb_filename = Some(filename.to_string());
                }
                // Read file content
                match field.bytes().await {
                    Ok(bytes) => nzb_content = Some(bytes.to_vec()),
                    Err(e) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "error": {
                                    "code": "invalid_file",
                                    "message": format!("Failed to read file: {}", e)
                                }
                            }))
                        ).into_response();
                    }
                }
            }
            "options" => {
                // Read options JSON
                match field.bytes().await {
                    Ok(bytes) => {
                        if let Ok(s) = String::from_utf8(bytes.to_vec()) {
                            options_json = Some(s);
                        }
                    }
                    Err(_) => {} // Optional field, ignore errors
                }
            }
            _ => {
                // Ignore unknown fields
            }
        }
    }

    // Validate that we have NZB content
    let nzb_bytes = match nzb_content {
        Some(bytes) => bytes,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "code": "missing_file",
                        "message": "No NZB file provided in 'file' field"
                    }
                }))
            ).into_response();
        }
    };

    // Parse options or use defaults
    let options: crate::types::DownloadOptions = match options_json {
        Some(json_str) => {
            match serde_json::from_str(&json_str) {
                Ok(opts) => opts,
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "error": {
                                "code": "invalid_options",
                                "message": format!("Invalid options JSON: {}", e)
                            }
                        }))
                    ).into_response();
                }
            }
        }
        None => crate::types::DownloadOptions::default(),
    };

    // Use filename or generate default
    let name = nzb_filename.unwrap_or_else(|| "upload.nzb".to_string());

    // Add NZB to download queue
    match state.downloader.add_nzb_content(&nzb_bytes, &name, options).await {
        Ok(download_id) => {
            (
                StatusCode::CREATED,
                Json(json!({
                    "id": download_id
                }))
            ).into_response()
        }
        Err(e) => {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": {
                        "code": "nzb_processing_failed",
                        "message": format!("Failed to process NZB: {}", e)
                    }
                }))
            ).into_response()
        }
    }
}

/// POST /downloads/url - Add NZB from URL
#[utoipa::path(
    post,
    path = "/api/v1/downloads/url",
    tag = "downloads",
    request_body(content = String, description = "URL to NZB file"),
    responses(
        (status = 201, description = "Download added successfully", body = i64),
        (status = 400, description = "Invalid URL or NZB content"),
        (status = 422, description = "Unprocessable entity"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn add_download_url(
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    // Extract URL from payload
    let url = match payload.get("url").and_then(|v| v.as_str()) {
        Some(url) => url,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "code": "missing_url",
                        "message": "Missing required field: url"
                    }
                }))
            ).into_response();
        }
    };

    // Extract optional download options
    let options = if let Some(options_value) = payload.get("options") {
        match serde_json::from_value(options_value.clone()) {
            Ok(opts) => opts,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": {
                            "code": "invalid_options",
                            "message": format!("Invalid download options: {}", e)
                        }
                    }))
                ).into_response();
            }
        }
    } else {
        crate::types::DownloadOptions::default()
    };

    // Call add_nzb_url to fetch and add the NZB
    match state.downloader.add_nzb_url(url, options).await {
        Ok(id) => {
            (
                StatusCode::CREATED,
                Json(json!({"id": id}))
            ).into_response()
        }
        Err(e) => {
            // Check error type to determine status code
            let status = match e {
                crate::error::Error::Io(_) => StatusCode::BAD_REQUEST,
                crate::error::Error::Network(_) => StatusCode::BAD_REQUEST,
                crate::error::Error::InvalidNzb(_) => StatusCode::UNPROCESSABLE_ENTITY,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };

            (
                status,
                Json(json!({
                    "error": {
                        "code": "add_failed",
                        "message": format!("Failed to add NZB from URL: {}", e)
                    }
                }))
            ).into_response()
        }
    }
}

/// POST /downloads/:id/pause - Pause download
#[utoipa::path(
    post,
    path = "/api/v1/downloads/{id}/pause",
    tag = "downloads",
    params(
        ("id" = i64, Path, description = "Download ID")
    ),
    responses(
        (status = 204, description = "Download paused successfully"),
        (status = 404, description = "Download not found"),
        (status = 409, description = "Download already paused"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn pause_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state.downloader.pause(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "error": {
                            "code": "not_found",
                            "message": error_msg
                        }
                    }))
                ).into_response()
            } else if error_msg.contains("Cannot pause") {
                (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "error": {
                            "code": "invalid_state",
                            "message": error_msg
                        }
                    }))
                ).into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": {
                            "code": "internal_error",
                            "message": error_msg
                        }
                    }))
                ).into_response()
            }
        }
    }
}

/// POST /downloads/:id/resume - Resume download
#[utoipa::path(
    post,
    path = "/api/v1/downloads/{id}/resume",
    tag = "downloads",
    params(
        ("id" = i64, Path, description = "Download ID")
    ),
    responses(
        (status = 204, description = "Download resumed successfully"),
        (status = 404, description = "Download not found"),
        (status = 409, description = "Download not paused"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn resume_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state.downloader.resume(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "error": {
                            "code": "not_found",
                            "message": error_msg
                        }
                    }))
                ).into_response()
            } else if error_msg.contains("Cannot resume") {
                (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "error": {
                            "code": "invalid_state",
                            "message": error_msg
                        }
                    }))
                ).into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": {
                            "code": "internal_error",
                            "message": error_msg
                        }
                    }))
                ).into_response()
            }
        }
    }
}

/// DELETE /downloads/:id - Cancel/remove download
#[utoipa::path(
    delete,
    path = "/api/v1/downloads/{id}",
    tag = "downloads",
    params(
        ("id" = i64, Path, description = "Download ID"),
        ("delete_files" = Option<bool>, Query, description = "Whether to delete downloaded files (not yet implemented, always deletes temp files)")
    ),
    responses(
        (status = 204, description = "Download deleted successfully"),
        (status = 404, description = "Download not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(_params): Query<DeleteDownloadQuery>,
) -> impl IntoResponse {
    // TODO: Use delete_files parameter to control whether to delete final destination files
    // Currently always deletes temp files via cancel()
    match state.downloader.cancel(id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            if e.to_string().contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "error": {
                            "code": "not_found",
                            "message": format!("Download {} not found", id)
                        }
                    }))
                ).into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": {
                            "code": "internal_error",
                            "message": e.to_string()
                        }
                    }))
                ).into_response()
            }
        }
    }
}

/// PATCH /downloads/:id/priority - Set priority
#[utoipa::path(
    patch,
    path = "/api/v1/downloads/{id}/priority",
    tag = "downloads",
    params(
        ("id" = i64, Path, description = "Download ID")
    ),
    request_body(content = crate::types::Priority, description = "New priority level"),
    responses(
        (status = 204, description = "Priority updated successfully"),
        (status = 404, description = "Download not found"),
        (status = 400, description = "Invalid priority value"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn set_download_priority(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Extract priority from JSON payload
    // Expected format: {"priority": "low"|"normal"|"high"|"force"}
    let priority = match payload.get("priority") {
        Some(priority_value) => {
            match serde_json::from_value::<crate::types::Priority>(priority_value.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "error": {
                                "code": "invalid_priority",
                                "message": format!("Invalid priority value: {}", e)
                            }
                        }))
                    ).into_response();
                }
            }
        }
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "code": "missing_priority",
                        "message": "Request body must include 'priority' field"
                    }
                }))
            ).into_response();
        }
    };

    // Call UsenetDownloader::set_priority()
    match state.downloader.set_priority(id, priority).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "error": {
                            "code": "not_found",
                            "message": error_msg
                        }
                    }))
                ).into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": {
                            "code": "internal_error",
                            "message": error_msg
                        }
                    }))
                ).into_response()
            }
        }
    }
}

/// POST /downloads/:id/reprocess - Re-run post-processing
#[utoipa::path(
    post,
    path = "/api/v1/downloads/{id}/reprocess",
    tag = "downloads",
    params(
        ("id" = i64, Path, description = "Download ID")
    ),
    responses(
        (status = 204, description = "Reprocessing started successfully"),
        (status = 404, description = "Download not found"),
        (status = 400, description = "Download files not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn reprocess_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Call reprocess on the downloader
    match state.downloader.reprocess(id).await {
        Ok(()) => {
            // Success - reprocessing started
            StatusCode::NO_CONTENT.into_response()
        }
        Err(crate::Error::NotFound(msg)) => {
            // Download or files not found
            let error_code = if msg.contains("Download files not found") {
                "files_not_found"
            } else {
                "not_found"
            };

            (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": {
                        "code": error_code,
                        "message": msg
                    }
                }))
            ).into_response()
        }
        Err(e) => {
            // Other errors
            tracing::error!(download_id = id, error = %e, "Failed to reprocess download");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "internal_error",
                        "message": format!("Failed to reprocess download: {}", e)
                    }
                }))
            ).into_response()
        }
    }
}

/// POST /downloads/:id/reextract - Re-run extraction only
#[utoipa::path(
    post,
    path = "/api/v1/downloads/{id}/reextract",
    tag = "downloads",
    params(
        ("id" = i64, Path, description = "Download ID")
    ),
    responses(
        (status = 204, description = "Re-extraction started successfully"),
        (status = 404, description = "Download not found"),
        (status = 400, description = "Download files not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn reextract_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Call reextract on the downloader
    match state.downloader.reextract(id).await {
        Ok(()) => {
            // Success - re-extraction started
            StatusCode::NO_CONTENT.into_response()
        }
        Err(crate::Error::NotFound(msg)) => {
            // Download or files not found
            let error_code = if msg.contains("Download files not found") {
                "files_not_found"
            } else {
                "not_found"
            };

            (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": {
                        "code": error_code,
                        "message": msg
                    }
                }))
            ).into_response()
        }
        Err(e) => {
            // Other errors
            tracing::error!(download_id = id, error = %e, "Failed to reextract download");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "internal_error",
                        "message": format!("Failed to reextract download: {}", e)
                    }
                }))
            ).into_response()
        }
    }
}

// ============================================================================
// Queue-Wide Operations
// ============================================================================

/// POST /queue/pause - Pause all downloads
#[utoipa::path(
    post,
    path = "/api/v1/queue/pause",
    tag = "queue",
    responses(
        (status = 204, description = "Queue paused successfully"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn pause_queue(State(state): State<AppState>) -> impl IntoResponse {
    match state.downloader.pause_all().await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to pause queue");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "pause_failed",
                        "message": format!("Failed to pause queue: {}", e)
                    }
                }))
            ).into_response()
        }
    }
}

/// POST /queue/resume - Resume all downloads
#[utoipa::path(
    post,
    path = "/api/v1/queue/resume",
    tag = "queue",
    responses(
        (status = 204, description = "Queue resumed successfully"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn resume_queue(State(state): State<AppState>) -> impl IntoResponse {
    match state.downloader.resume_all().await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to resume queue");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "resume_failed",
                        "message": format!("Failed to resume queue: {}", e)
                    }
                }))
            ).into_response()
        }
    }
}

/// GET /queue/stats - Get queue statistics
#[utoipa::path(
    get,
    path = "/api/v1/queue/stats",
    tag = "queue",
    responses(
        (status = 200, description = "Queue statistics", body = crate::types::QueueStats),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn queue_stats(State(state): State<AppState>) -> Response {
    // Query all downloads from database
    match state.downloader.db.get_all_downloads().await {
        Ok(downloads) => {
            // Initialize counters
            let mut queued = 0;
            let mut downloading = 0;
            let mut paused = 0;
            let mut processing = 0;
            let mut total_speed_bps = 0u64;
            let mut total_size_bytes = 0u64;
            let mut downloaded_bytes = 0u64;

            // Calculate statistics
            for download in &downloads {
                let status = crate::types::Status::from_i32(download.status);

                // Count by status
                match status {
                    crate::types::Status::Queued => queued += 1,
                    crate::types::Status::Downloading => downloading += 1,
                    crate::types::Status::Paused => paused += 1,
                    crate::types::Status::Processing => processing += 1,
                    _ => {} // Complete and Failed are not in "active" queue
                }

                // Sum up speeds (only for downloading items)
                if status == crate::types::Status::Downloading {
                    total_speed_bps += download.speed_bps as u64;
                }

                // Sum up sizes
                total_size_bytes += download.size_bytes as u64;
                downloaded_bytes += download.downloaded_bytes as u64;
            }

            let total = downloads.len();

            // Calculate overall progress
            let overall_progress = if total_size_bytes > 0 {
                (downloaded_bytes as f32 / total_size_bytes as f32) * 100.0
            } else {
                0.0
            };

            // Get speed limit from speed limiter (this reflects runtime changes via set_speed_limit)
            let speed_limit_bps = state.downloader.speed_limiter.get_limit();

            // Get accepting_new flag
            let accepting_new = state.downloader.accepting_new.load(std::sync::atomic::Ordering::SeqCst);

            let stats = crate::types::QueueStats {
                total,
                queued,
                downloading,
                paused,
                processing,
                total_speed_bps,
                total_size_bytes,
                downloaded_bytes,
                overall_progress,
                speed_limit_bps,
                accepting_new,
            };

            (StatusCode::OK, Json(stats)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to get queue statistics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "stats_failed",
                        "message": format!("Failed to get queue statistics: {}", e)
                    }
                }))
            ).into_response()
        }
    }
}

// ============================================================================
// History
// ============================================================================

/// GET /history - Get download history (with pagination)
#[utoipa::path(
    get,
    path = "/api/v1/history",
    tag = "history",
    params(
        ("limit" = Option<i64>, Query, description = "Maximum number of items to return"),
        ("offset" = Option<i64>, Query, description = "Number of items to skip"),
        ("status" = Option<String>, Query, description = "Filter by status (complete/failed)")
    ),
    responses(
        (status = 200, description = "Download history", body = Vec<crate::types::HistoryEntry>),
        (status = 400, description = "Invalid query parameters"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    // Set defaults for pagination
    let limit = query.limit.unwrap_or(50).max(1).min(1000) as usize;
    let offset = query.offset.unwrap_or(0).max(0) as usize;

    // Parse status filter if provided
    let status_filter = if let Some(status_str) = query.status {
        match status_str.to_lowercase().as_str() {
            "complete" => Some(4), // Status::Complete = 4
            "failed" => Some(5),   // Status::Failed = 5
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": {
                            "code": "invalid_status",
                            "message": "Invalid status filter. Must be 'complete' or 'failed'"
                        }
                    })),
                )
                    .into_response();
            }
        }
    } else {
        None
    };

    // Query history from database
    match state.downloader.db.query_history(status_filter, limit, offset).await {
        Ok(entries) => {
            // Get total count for pagination metadata
            match state.downloader.db.count_history(status_filter).await {
                Ok(total) => {
                    let response = json!({
                        "items": entries,
                        "total": total,
                        "limit": limit,
                        "offset": offset
                    });
                    (StatusCode::OK, Json(response)).into_response()
                }
                Err(e) => {
                    tracing::error!("Failed to count history: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "error": {
                                "code": "database_error",
                                "message": "Failed to count history entries"
                            }
                        })),
                    )
                        .into_response()
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to query history: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "database_error",
                        "message": "Failed to retrieve history"
                    }
                })),
            )
                .into_response()
        }
    }
}

/// DELETE /history - Clear history
#[utoipa::path(
    delete,
    path = "/api/v1/history",
    tag = "history",
    params(
        ("before" = Option<i64>, Query, description = "Clear entries before this timestamp"),
        ("status" = Option<String>, Query, description = "Clear only entries with this status (complete/failed)")
    ),
    responses(
        (status = 200, description = "Number of deleted entries"),
        (status = 400, description = "Invalid status filter"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn clear_history(
    State(state): State<AppState>,
    Query(query): Query<ClearHistoryQuery>,
) -> impl IntoResponse {
    // Parse status filter if provided
    let status_filter = if let Some(status_str) = query.status {
        match status_str.to_lowercase().as_str() {
            "complete" => Some(4), // Status::Complete = 4
            "failed" => Some(5),   // Status::Failed = 5
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": {
                            "code": "invalid_status",
                            "message": "Invalid status filter. Must be 'complete' or 'failed'"
                        }
                    })),
                )
                    .into_response();
            }
        }
    } else {
        None
    };

    // Delete history entries with filters
    match state
        .downloader
        .db
        .delete_history_filtered(query.before, status_filter)
        .await
    {
        Ok(deleted_count) => (
            StatusCode::OK,
            Json(json!({
                "deleted": deleted_count
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to clear history");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "clear_failed",
                        "message": format!("Failed to clear history: {}", e)
                    }
                })),
            )
                .into_response()
        }
    }
}

// ============================================================================
// Server Management
// ============================================================================

/// POST /servers/test - Test server connection
#[utoipa::path(
    post,
    path = "/api/v1/servers/test",
    tag = "servers",
    request_body(content = crate::config::ServerConfig, description = "Server configuration to test"),
    responses(
        (status = 200, description = "Server test result"),
        (status = 400, description = "Invalid server configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn test_server(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// GET /servers/test - Test all configured servers
#[utoipa::path(
    get,
    path = "/api/v1/servers/test",
    tag = "servers",
    responses(
        (status = 200, description = "Test results for all servers"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn test_all_servers(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

// ============================================================================
// Configuration
// ============================================================================

/// GET /config - Get current config (sensitive fields redacted)
#[utoipa::path(
    get,
    path = "/api/v1/config",
    tag = "config",
    responses(
        (status = 200, description = "Current configuration", body = crate::config::Config),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_config(State(state): State<AppState>) -> impl IntoResponse {
    // Get the config from the downloader
    let config = state.downloader.get_config();

    // Clone the config and redact sensitive fields
    let mut redacted_config = (*config).clone();

    // Redact server passwords
    for server in &mut redacted_config.servers {
        if server.password.is_some() {
            server.password = Some("***REDACTED***".to_string());
        }
    }

    // Redact API key
    if redacted_config.api.api_key.is_some() {
        redacted_config.api.api_key = Some("***REDACTED***".to_string());
    }

    (StatusCode::OK, Json(redacted_config))
}

/// PATCH /config - Update config
#[utoipa::path(
    patch,
    path = "/api/v1/config",
    tag = "config",
    request_body(content = crate::config::ConfigUpdate, description = "Configuration updates (runtime-changeable fields only)"),
    responses(
        (status = 200, description = "Configuration updated", body = crate::config::Config),
        (status = 400, description = "Invalid configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_config(
    State(state): State<AppState>,
    Json(updates): Json<crate::config::ConfigUpdate>,
) -> impl IntoResponse {
    // Apply the configuration updates
    state.downloader.update_config(updates).await;

    // Return the updated config (with redaction)
    get_config(State(state)).await
}

/// GET /config/speed-limit - Get speed limit
#[utoipa::path(
    get,
    path = "/api/v1/config/speed-limit",
    tag = "config",
    responses(
        (status = 200, description = "Current speed limit in bytes per second", body = inline(Object)),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_speed_limit(State(state): State<AppState>) -> impl IntoResponse {
    // Get the current speed limit from the downloader
    let limit_bps = state.downloader.get_speed_limit();

    // Return JSON response with limit_bps field
    // null indicates unlimited speed
    Json(json!({
        "limit_bps": limit_bps
    }))
}

/// PUT /config/speed-limit - Set speed limit
#[utoipa::path(
    put,
    path = "/api/v1/config/speed-limit",
    tag = "config",
    request_body = SetSpeedLimitRequest,
    responses(
        (status = 204, description = "Speed limit updated successfully"),
        (status = 400, description = "Invalid speed limit value"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn set_speed_limit(
    State(state): State<AppState>,
    Json(request): Json<SetSpeedLimitRequest>,
) -> impl IntoResponse {
    // Update the speed limit in the downloader
    state.downloader.set_speed_limit(request.limit_bps).await;

    // Return 204 No Content on success
    StatusCode::NO_CONTENT
}

// ============================================================================
// Categories
// ============================================================================

/// GET /categories - List categories
#[utoipa::path(
    get,
    path = "/api/v1/categories",
    tag = "categories",
    responses(
        (status = 200, description = "Map of category names to configurations"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_categories(State(state): State<AppState>) -> impl IntoResponse {
    // Get the config from the downloader
    let config = state.downloader.get_config();

    // Clone and return the categories HashMap
    (StatusCode::OK, Json(config.categories.clone()))
}

/// PUT /categories/:name - Create/update category
#[utoipa::path(
    put,
    path = "/api/v1/categories/{name}",
    tag = "categories",
    params(
        ("name" = String, Path, description = "Category name")
    ),
    request_body(content = crate::config::CategoryConfig, description = "Category configuration"),
    responses(
        (status = 204, description = "Category created/updated successfully"),
        (status = 400, description = "Invalid category configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn create_or_update_category(
    State(_state): State<AppState>,
    Path(_name): Path<String>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// DELETE /categories/:name - Delete category
#[utoipa::path(
    delete,
    path = "/api/v1/categories/{name}",
    tag = "categories",
    params(
        ("name" = String, Path, description = "Category name")
    ),
    responses(
        (status = 204, description = "Category deleted successfully"),
        (status = 404, description = "Category not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_category(
    State(_state): State<AppState>,
    Path(_name): Path<String>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

// ============================================================================
// System
// ============================================================================

/// GET /health - Health check
#[utoipa::path(
    get,
    path = "/api/v1/health",
    tag = "system",
    responses(
        (status = 200, description = "Service is healthy")
    )
)]
pub async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// GET /openapi.json - OpenAPI specification
#[utoipa::path(
    get,
    path = "/api/v1/openapi.json",
    tag = "system",
    responses(
        (status = 200, description = "OpenAPI 3.1 specification in JSON format")
    )
)]
pub async fn openapi_spec() -> impl IntoResponse {
    use crate::api::openapi::ApiDoc;
    use utoipa::OpenApi;

    Json(ApiDoc::openapi())
}

/// GET /events - Server-sent events stream
///
/// Subscribe to real-time events from the download manager.
/// Returns a Server-Sent Events (SSE) stream in text/event-stream format.
///
/// # Event Format
///
/// Each event is sent as:
/// ```text
/// event: <event_type>
/// data: <json_payload>
/// ```
///
/// # Example Usage
///
/// Using curl:
/// ```bash
/// curl -N http://localhost:6789/api/v1/events
/// ```
///
/// Using JavaScript EventSource:
/// ```javascript
/// const events = new EventSource('http://localhost:6789/api/v1/events');
/// events.onmessage = (e) => {
///     const event = JSON.parse(e.data);
///     console.log(event);
/// };
/// ```
#[utoipa::path(
    get,
    path = "/api/v1/events",
    tag = "system",
    responses(
        (status = 200, description = "Server-sent events stream (text/event-stream)", content_type = "text/event-stream"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn event_stream(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    // Subscribe to the downloader's event broadcast channel
    let receiver = state.downloader.subscribe();

    // Convert the broadcast receiver to a stream
    let stream = BroadcastStream::new(receiver);

    // Map events to SSE format
    let sse_stream = stream.filter_map(|result| {
        match result {
            Ok(event) => {
                // Serialize the event to JSON
                match serde_json::to_string(&event) {
                    Ok(json_data) => {
                        // Determine event type from the event variant
                        let event_type = match &event {
                            crate::types::Event::Queued { .. } => "queued",
                            crate::types::Event::Removed { .. } => "removed",
                            crate::types::Event::Downloading { .. } => "downloading",
                            crate::types::Event::DownloadComplete { .. } => "download_complete",
                            crate::types::Event::DownloadFailed { .. } => "download_failed",
                            crate::types::Event::Verifying { .. } => "verifying",
                            crate::types::Event::VerifyComplete { .. } => "verify_complete",
                            crate::types::Event::Repairing { .. } => "repairing",
                            crate::types::Event::RepairComplete { .. } => "repair_complete",
                            crate::types::Event::Extracting { .. } => "extracting",
                            crate::types::Event::ExtractComplete { .. } => "extract_complete",
                            crate::types::Event::Moving { .. } => "moving",
                            crate::types::Event::Cleaning { .. } => "cleaning",
                            crate::types::Event::Complete { .. } => "complete",
                            crate::types::Event::Failed { .. } => "failed",
                            crate::types::Event::SpeedLimitChanged { .. } => "speed_limit_changed",
                            crate::types::Event::QueuePaused => "queue_paused",
                            crate::types::Event::QueueResumed => "queue_resumed",
                            crate::types::Event::WebhookFailed { .. } => "webhook_failed",
                            crate::types::Event::ScriptFailed { .. } => "script_failed",
                            crate::types::Event::Shutdown => "shutdown",
                        };

                        // Create SSE event with type and data
                        Some(Ok(SseEvent::default()
                            .event(event_type)
                            .data(json_data)))
                    }
                    Err(e) => {
                        tracing::warn!("Failed to serialize event to JSON: {}", e);
                        None
                    }
                }
            }
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(skipped)) => {
                // Client is too slow and missed some events
                tracing::warn!("SSE client lagged, skipped {} events", skipped);
                Some(Ok(SseEvent::default()
                    .event("error")
                    .data(format!(r#"{{"error":"lagged","skipped":{}}}"#, skipped))))
            }
        }
    });

    // Return SSE response with keep-alive
    Sse::new(sse_stream).keep_alive(KeepAlive::default())
}

/// POST /shutdown - Graceful shutdown
#[utoipa::path(
    post,
    path = "/api/v1/shutdown",
    tag = "system",
    responses(
        (status = 202, description = "Shutdown initiated"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn shutdown(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

// ============================================================================
// RSS Feeds
// ============================================================================

/// GET /rss - List RSS feeds
#[utoipa::path(
    get,
    path = "/api/v1/rss",
    tag = "rss",
    responses(
        (status = 200, description = "List of RSS feeds"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_rss_feeds(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// POST /rss - Add RSS feed
#[utoipa::path(
    post,
    path = "/api/v1/rss",
    tag = "rss",
    responses(
        (status = 201, description = "RSS feed added successfully", body = i64),
        (status = 400, description = "Invalid RSS feed configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn add_rss_feed(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// PUT /rss/:id - Update RSS feed
#[utoipa::path(
    put,
    path = "/api/v1/rss/{id}",
    tag = "rss",
    params(
        ("id" = i64, Path, description = "RSS feed ID")
    ),
    responses(
        (status = 204, description = "RSS feed updated successfully"),
        (status = 404, description = "RSS feed not found"),
        (status = 400, description = "Invalid RSS feed configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_rss_feed(
    State(_state): State<AppState>,
    Path(_id): Path<i64>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// DELETE /rss/:id - Delete RSS feed
#[utoipa::path(
    delete,
    path = "/api/v1/rss/{id}",
    tag = "rss",
    params(
        ("id" = i64, Path, description = "RSS feed ID")
    ),
    responses(
        (status = 204, description = "RSS feed deleted successfully"),
        (status = 404, description = "RSS feed not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_rss_feed(
    State(_state): State<AppState>,
    Path(_id): Path<i64>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// POST /rss/:id/check - Force check feed now
#[utoipa::path(
    post,
    path = "/api/v1/rss/{id}/check",
    tag = "rss",
    params(
        ("id" = i64, Path, description = "RSS feed ID")
    ),
    responses(
        (status = 200, description = "Number of new items queued"),
        (status = 404, description = "RSS feed not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn check_rss_feed(
    State(_state): State<AppState>,
    Path(_id): Path<i64>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

// ============================================================================
// Scheduler
// ============================================================================

/// GET /scheduler - Get schedule rules
#[utoipa::path(
    get,
    path = "/api/v1/scheduler",
    tag = "scheduler",
    responses(
        (status = 200, description = "List of schedule rules", body = Vec<crate::config::ScheduleRule>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_schedule_rules(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// POST /scheduler - Add schedule rule
#[utoipa::path(
    post,
    path = "/api/v1/scheduler",
    tag = "scheduler",
    request_body(content = crate::config::ScheduleRule, description = "Schedule rule configuration"),
    responses(
        (status = 201, description = "Schedule rule added successfully", body = i64),
        (status = 400, description = "Invalid schedule rule configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn add_schedule_rule(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// PUT /scheduler/:id - Update schedule rule
#[utoipa::path(
    put,
    path = "/api/v1/scheduler/{id}",
    tag = "scheduler",
    params(
        ("id" = i64, Path, description = "Schedule rule ID")
    ),
    request_body(content = crate::config::ScheduleRule, description = "Updated schedule rule configuration"),
    responses(
        (status = 204, description = "Schedule rule updated successfully"),
        (status = 404, description = "Schedule rule not found"),
        (status = 400, description = "Invalid schedule rule configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_schedule_rule(
    State(_state): State<AppState>,
    Path(_id): Path<i64>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// DELETE /scheduler/:id - Delete schedule rule
#[utoipa::path(
    delete,
    path = "/api/v1/scheduler/{id}",
    tag = "scheduler",
    params(
        ("id" = i64, Path, description = "Schedule rule ID")
    ),
    responses(
        (status = 204, description = "Schedule rule deleted successfully"),
        (status = 404, description = "Schedule rule not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_schedule_rule(
    State(_state): State<AppState>,
    Path(_id): Path<i64>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}
