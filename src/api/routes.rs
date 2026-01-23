//! Route handlers for the REST API

use super::AppState;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
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
    State(_state): State<AppState>,
    Path(_id): Path<i64>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
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
    State(_state): State<AppState>,
    Path(_id): Path<i64>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
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
pub async fn pause_queue(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
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
pub async fn resume_queue(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// GET /queue/stats - Get queue statistics
#[utoipa::path(
    get,
    path = "/api/v1/queue/stats",
    tag = "queue",
    responses(
        (status = 200, description = "Queue statistics"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn queue_stats(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
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
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_history(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// DELETE /history - Clear history
#[utoipa::path(
    delete,
    path = "/api/v1/history",
    tag = "history",
    params(
        ("before" = Option<i64>, Query, description = "Clear entries before this timestamp"),
        ("status" = Option<String>, Query, description = "Clear only entries with this status")
    ),
    responses(
        (status = 200, description = "Number of deleted entries"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn clear_history(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
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
pub async fn get_config(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// PATCH /config - Update config
#[utoipa::path(
    patch,
    path = "/api/v1/config",
    tag = "config",
    request_body(content = crate::config::Config, description = "Configuration updates (partial)"),
    responses(
        (status = 200, description = "Configuration updated", body = crate::config::Config),
        (status = 400, description = "Invalid configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_config(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// GET /config/speed-limit - Get speed limit
#[utoipa::path(
    get,
    path = "/api/v1/config/speed-limit",
    tag = "config",
    responses(
        (status = 200, description = "Current speed limit in bytes per second"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_speed_limit(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

/// PUT /config/speed-limit - Set speed limit
#[utoipa::path(
    put,
    path = "/api/v1/config/speed-limit",
    tag = "config",
    request_body(content = u64, description = "Speed limit in bytes per second (null for unlimited)"),
    responses(
        (status = 204, description = "Speed limit updated successfully"),
        (status = 400, description = "Invalid speed limit value"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn set_speed_limit(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
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
pub async fn list_categories(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
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
#[utoipa::path(
    get,
    path = "/api/v1/events",
    tag = "system",
    responses(
        (status = 200, description = "Server-sent events stream (text/event-stream)", content_type = "text/event-stream"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn event_stream(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
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
