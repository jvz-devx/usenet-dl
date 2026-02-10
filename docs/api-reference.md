# usenet-dl REST API Documentation

Complete reference for the usenet-dl REST API with curl examples for all endpoints.

## Table of Contents

- [Quick Start](#quick-start)
- [Authentication](#authentication)
- [Error Handling](#error-handling)
- [Endpoints](#endpoints)
  - [System](#system)
  - [Downloads](#downloads)
  - [Queue Management](#queue-management)
  - [History](#history)
  - [Configuration](#configuration)
  - [Categories](#categories)
  - [Server Testing](#server-testing)
  - [RSS Feeds](#rss-feeds)
  - [Scheduler](#scheduler)
  - [Real-time Events](#real-time-events)
- [Common Workflows](#common-workflows)

## Quick Start

### Starting the API Server

```bash
# Using the REST API example
cargo run --example rest_api_server

# Or integrate into your application
use usenet_dl::UsenetDownloader;
use usenet_dl::api::start_api_server;

let downloader = UsenetDownloader::new(config).await?;
let api_config = Arc::new(config.clone());
start_api_server(Arc::new(downloader), api_config).await?;
```

### Base URL

All endpoints use the base URL: `http://localhost:6789/api/v1`

### Health Check

Verify the server is running:

```bash
curl http://localhost:6789/api/v1/health
```

Response:
```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

### Interactive Documentation

Open your browser to: **http://localhost:6789/swagger-ui/**

The Swagger UI provides interactive documentation where you can test all endpoints directly.

## Authentication

If API key authentication is enabled in the configuration:

```bash
# Set your API key
export API_KEY="your-api-key-here"

# Include X-Api-Key header in all requests
curl -H "X-Api-Key: $API_KEY" http://localhost:6789/api/v1/downloads
```

By default, the API server binds to localhost without authentication for easy local development.

## Error Handling

All errors follow a standard JSON format:

```json
{
  "error": {
    "code": "not_found",
    "message": "Download not found",
    "details": {
      "download_id": 123
    }
  }
}
```

### Common Error Codes

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Resource doesn't exist |
| `validation_error` | 400 | Invalid request data |
| `conflict` | 409 | Invalid state transition |
| `unprocessable_entity` | 422 | Request cannot be processed |
| `internal_error` | 500 | Server error |
| `service_unavailable` | 503 | Service shutting down |
| `rate_limited` | 429 | Too many requests |

## Endpoints

### System

#### Health Check

Check if the server is running and get version information.

```bash
curl http://localhost:6789/api/v1/health
```

**Response:**
```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

#### OpenAPI Specification

Get the complete OpenAPI 3.1 specification.

```bash
curl http://localhost:6789/api/v1/openapi.json | jq .
```

---

### Downloads

#### List All Downloads

Get all downloads in the queue with their current status.

```bash
curl http://localhost:6789/api/v1/downloads | jq .
```

**Response:**
```json
[
  {
    "id": 1,
    "name": "Ubuntu.24.04.iso",
    "category": "software",
    "status": "downloading",
    "progress": 45.2,
    "speed_bps": 10485760,
    "size_bytes": 4294967296,
    "downloaded_bytes": 1941962752,
    "eta_seconds": 225,
    "priority": "normal",
    "created_at": "2024-01-23T10:30:00Z",
    "started_at": "2024-01-23T10:31:00Z"
  }
]
```

#### Get Single Download

Get details of a specific download by ID.

```bash
DOWNLOAD_ID=1
curl http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID | jq .
```

**Response:** Same structure as list endpoint, but single object.

**Error (404):**
```json
{
  "error": {
    "code": "not_found",
    "message": "Download not found",
    "details": {
      "download_id": 999
    }
  }
}
```

#### Add Download from File

Upload an NZB file to add to the queue.

```bash
curl -X POST http://localhost:6789/api/v1/downloads \
  -F "file=@/path/to/file.nzb" \
  -F 'options={"priority":"high","category":"movies"}'
```

**Options (all optional):**
- `category` (string): Category name
- `destination` (string): Override destination path
- `post_process` (string): `none`, `verify`, `repair`, `unpack`, `unpack_and_cleanup`
- `priority` (string): `low`, `normal`, `high`, `force`
- `password` (string): Password for extraction

**Response:**
```json
{
  "id": 42
}
```

#### Add Download from URL

Add an NZB file by URL.

```bash
curl -X POST http://localhost:6789/api/v1/downloads/url \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com/file.nzb",
    "options": {
      "category": "movies",
      "priority": "high"
    }
  }' | jq .
```

**Response:**
```json
{
  "id": 43
}
```

#### Pause Download

Pause a specific download.

```bash
DOWNLOAD_ID=1
curl -X POST http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/pause
```

**Response:** 204 No Content

**Error (409):**
```json
{
  "error": {
    "code": "conflict",
    "message": "Download is already paused"
  }
}
```

#### Resume Download

Resume a paused download.

```bash
DOWNLOAD_ID=1
curl -X POST http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/resume
```

**Response:** 204 No Content

#### Delete Download

Cancel and remove a download from the queue.

```bash
DOWNLOAD_ID=1

# Keep downloaded files
curl -X DELETE "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID?delete_files=false"

# Delete downloaded files
curl -X DELETE "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID?delete_files=true"
```

**Query Parameters:**
- `delete_files` (boolean): Whether to delete downloaded files (default: `false`)

**Response:** 204 No Content

#### Set Download Priority

Change the priority of a download in the queue.

```bash
DOWNLOAD_ID=1

# Set to high priority
curl -X PATCH "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/priority" \
  -H "Content-Type: application/json" \
  -d '{"priority":"high"}'
```

**Valid priorities:**
- `low`: Lower priority
- `normal`: Default priority
- `high`: Higher priority
- `force`: Start immediately, ignore concurrent limit

**Response:** 204 No Content

#### Reprocess Download

Re-run the complete post-processing pipeline (verify, repair, extract, move, cleanup).

Useful when:
- Extraction failed due to missing password
- Post-processing settings changed
- Files were manually repaired

```bash
DOWNLOAD_ID=1
curl -X POST "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/reprocess"
```

**Response:** 204 No Content

**Error (404):**
```json
{
  "error": {
    "code": "not_found",
    "message": "Download files not found",
    "details": {
      "download_id": 1
    }
  }
}
```

#### Reextract Download

Re-run extraction only (skip PAR2 verification and repair).

Useful when you want to extract with a different password without re-downloading.

```bash
DOWNLOAD_ID=1
curl -X POST "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/reextract"
```

**Response:** 204 No Content

---

### Queue Management

#### Get Queue Statistics

Get aggregate statistics about the download queue.

```bash
curl http://localhost:6789/api/v1/queue/stats | jq .
```

**Response:**
```json
{
  "total": 15,
  "queued": 8,
  "downloading": 3,
  "paused": 2,
  "processing": 2,
  "complete": 0,
  "failed": 0,
  "total_size_bytes": 10737418240,
  "downloaded_bytes": 3221225472,
  "speed_bps": 31457280
}
```

#### Pause All Downloads

Pause the entire download queue.

```bash
curl -X POST http://localhost:6789/api/v1/queue/pause
```

**Response:** 204 No Content

#### Resume All Downloads

Resume all paused downloads.

```bash
curl -X POST http://localhost:6789/api/v1/queue/resume
```

**Response:** 204 No Content

---

### History

#### Get Download History

Retrieve completed and failed downloads.

```bash
# Get all history
curl http://localhost:6789/api/v1/history | jq .

# Get paginated results
curl "http://localhost:6789/api/v1/history?limit=10&offset=0" | jq .

# Filter by status
curl "http://localhost:6789/api/v1/history?status=complete" | jq .
curl "http://localhost:6789/api/v1/history?status=failed" | jq .
```

**Query Parameters:**
- `limit` (integer): Maximum number of items to return (default: 50)
- `offset` (integer): Number of items to skip (default: 0)
- `status` (string): Filter by status (`complete` or `failed`)

**Response:**
```json
{
  "items": [
    {
      "id": 1,
      "name": "Ubuntu.24.04.iso",
      "category": "software",
      "destination": "/downloads/software/Ubuntu.24.04.iso",
      "status": "complete",
      "size_bytes": 4294967296,
      "download_time_secs": 720,
      "completed_at": "2024-01-23T11:00:00Z"
    }
  ],
  "total": 1
}
```

#### Clear History

Delete history entries with optional filters.

```bash
# Clear all history
curl -X DELETE http://localhost:6789/api/v1/history

# Clear history older than timestamp (Unix epoch)
TIMESTAMP=$(date -d "7 days ago" +%s)
curl -X DELETE "http://localhost:6789/api/v1/history?before=$TIMESTAMP"

# Clear only failed downloads
curl -X DELETE "http://localhost:6789/api/v1/history?status=failed"

# Clear old failed downloads
curl -X DELETE "http://localhost:6789/api/v1/history?before=$TIMESTAMP&status=failed"
```

**Query Parameters:**
- `before` (integer): Unix timestamp - delete entries older than this
- `status` (string): Filter by status (`complete` or `failed`)

**Response:**
```json
{
  "deleted": 42
}
```

---

### Configuration

#### Get Current Configuration

Get the current configuration with sensitive fields redacted.

```bash
curl http://localhost:6789/api/v1/config | jq .
```

**Response:**
```json
{
  "download_dir": "/downloads",
  "temp_dir": "/temp",
  "max_concurrent_downloads": 3,
  "speed_limit_bps": 10485760,
  "servers": [
    {
      "host": "news.example.com",
      "port": 563,
      "tls": true,
      "username": "user123",
      "password": "[REDACTED]",
      "connections": 10,
      "priority": 0
    }
  ]
}
```

**Note:** Passwords, API keys, and other sensitive fields are automatically redacted.

#### Update Configuration

Update runtime-changeable configuration fields.

```bash
curl -X PATCH http://localhost:6789/api/v1/config \
  -H "Content-Type: application/json" \
  -d '{
    "max_concurrent_downloads": 5,
    "speed_limit_bps": 20971520
  }'
```

**Updatable fields:**
- `max_concurrent_downloads` (integer)
- `speed_limit_bps` (integer or null)
- `delete_samples` (boolean)

**Response:** Updated configuration (same format as GET)

#### Get Speed Limit

Get the current speed limit.

```bash
curl http://localhost:6789/api/v1/config/speed-limit | jq .
```

**Response:**
```json
{
  "limit_bps": 10485760
}
```

Or if unlimited:
```json
{
  "limit_bps": null
}
```

#### Set Speed Limit

Set the global speed limit in bytes per second.

```bash
# Set limit to 10 MB/s
curl -X PUT http://localhost:6789/api/v1/config/speed-limit \
  -H "Content-Type: application/json" \
  -d '{"limit_bps": 10485760}'

# Remove limit (unlimited)
curl -X PUT http://localhost:6789/api/v1/config/speed-limit \
  -H "Content-Type: application/json" \
  -d '{"limit_bps": null}'
```

**Response:** 204 No Content

---

### Categories

#### List Categories

Get all configured categories.

```bash
curl http://localhost:6789/api/v1/categories | jq .
```

**Response:**
```json
{
  "movies": {
    "destination": "/downloads/movies",
    "post_process": "unpack_and_cleanup",
    "watch_folder": null,
    "scripts": []
  },
  "tv": {
    "destination": "/downloads/tv",
    "post_process": "unpack_and_cleanup",
    "watch_folder": {
      "path": "/watch/tv",
      "after_import": "move_to_processed",
      "scan_interval_secs": 5
    },
    "scripts": []
  }
}
```

#### Create or Update Category

Create a new category or update an existing one.

```bash
curl -X PUT "http://localhost:6789/api/v1/categories/software" \
  -H "Content-Type: application/json" \
  -d '{
    "destination": "/downloads/software",
    "post_process": "unpack_and_cleanup",
    "watch_folder": null,
    "scripts": []
  }'
```

**Response:** 204 No Content

#### Delete Category

Remove a category configuration.

```bash
curl -X DELETE "http://localhost:6789/api/v1/categories/software"
```

**Response:** 204 No Content

**Note:** Downloads already in the category are not affected.

---

### Server Testing

#### Test Server Configuration

Test connectivity and authentication for a specific NNTP server.

```bash
curl -X POST http://localhost:6789/api/v1/servers/test \
  -H "Content-Type: application/json" \
  -d '{
    "host": "news.example.com",
    "port": 563,
    "tls": true,
    "username": "testuser",
    "password": "testpass",
    "connections": 10,
    "priority": 0
  }' | jq .
```

**Response:**
```json
{
  "success": true,
  "latency": {
    "secs": 0,
    "nanos": 125000000
  },
  "error": null,
  "capabilities": {
    "posting_allowed": false,
    "max_connections": 50,
    "compression": true
  }
}
```

**Error response:**
```json
{
  "success": false,
  "latency": {
    "secs": 2,
    "nanos": 0
  },
  "error": "Connection refused",
  "capabilities": null
}
```

#### Test All Configured Servers

Test all servers in the current configuration.

```bash
curl http://localhost:6789/api/v1/servers/test | jq .
```

**Response:**
```json
[
  {
    "server": "news.example.com",
    "result": {
      "success": true,
      "latency": { "secs": 0, "nanos": 125000000 },
      "error": null,
      "capabilities": {
        "posting_allowed": false,
        "max_connections": 50,
        "compression": true
      }
    }
  }
]
```

---

### RSS Feeds

#### List RSS Feeds

Get all configured RSS feeds.

```bash
curl http://localhost:6789/api/v1/rss | jq .
```

**Response:**
```json
[
  {
    "id": 1,
    "name": "Ubuntu Releases",
    "url": "https://releases.ubuntu.com/rss.xml",
    "check_interval_secs": 900,
    "category": "software",
    "auto_download": true,
    "priority": "normal",
    "enabled": true,
    "last_check": "2024-01-23T10:00:00Z",
    "last_error": null,
    "filters": []
  }
]
```

#### Add RSS Feed

Add a new RSS feed to monitor.

```bash
curl -X POST http://localhost:6789/api/v1/rss \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Ubuntu Releases",
    "url": "https://releases.ubuntu.com/rss.xml",
    "check_interval_secs": 900,
    "category": "software",
    "auto_download": true,
    "priority": "normal",
    "enabled": true,
    "filters": []
  }' | jq .
```

**Response:**
```json
{
  "id": 2
}
```

#### Update RSS Feed

Update an existing RSS feed configuration.

```bash
curl -X PUT http://localhost:6789/api/v1/rss/1 \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Ubuntu Releases (Updated)",
    "url": "https://releases.ubuntu.com/rss.xml",
    "check_interval_secs": 600,
    "category": "software",
    "auto_download": true,
    "priority": "high",
    "enabled": true,
    "filters": []
  }'
```

**Response:** 204 No Content

#### Delete RSS Feed

Remove an RSS feed.

```bash
curl -X DELETE http://localhost:6789/api/v1/rss/1
```

**Response:** 204 No Content

#### Force Check RSS Feed

Manually trigger a feed check (bypass interval).

```bash
curl -X POST http://localhost:6789/api/v1/rss/1/check | jq .
```

**Response:**
```json
{
  "queued": 3
}
```

---

### Scheduler

#### List Schedule Rules

Get all schedule rules.

```bash
curl http://localhost:6789/api/v1/scheduler | jq .
```

**Response:**
```json
[
  {
    "id": 1,
    "name": "Night Owl",
    "days": [],
    "start_time": "00:00",
    "end_time": "06:00",
    "action": { "Unlimited": null },
    "enabled": true
  },
  {
    "id": 2,
    "name": "Work Hours",
    "days": ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"],
    "start_time": "09:00",
    "end_time": "17:00",
    "action": { "SpeedLimit": { "limit_bps": 1048576 } },
    "enabled": true
  }
]
```

#### Add Schedule Rule

Create a new schedule rule.

```bash
curl -X POST http://localhost:6789/api/v1/scheduler \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Weekend Unlimited",
    "days": ["Saturday", "Sunday"],
    "start_time": "00:00",
    "end_time": "23:59",
    "action": { "Unlimited": null },
    "enabled": true
  }' | jq .
```

**Action types:**
- `{"Unlimited": null}`: No speed limit
- `{"SpeedLimit": {"limit_bps": 1048576}}`: Limit to specified bytes/sec
- `{"Pause": null}`: Pause all downloads

**Response:**
```json
{
  "id": 3
}
```

#### Update Schedule Rule

Update an existing schedule rule.

```bash
curl -X PUT http://localhost:6789/api/v1/scheduler/1 \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Night Owl (Updated)",
    "days": [],
    "start_time": "00:00",
    "end_time": "07:00",
    "action": { "Unlimited": null },
    "enabled": true
  }'
```

**Response:** 204 No Content

#### Delete Schedule Rule

Remove a schedule rule.

```bash
curl -X DELETE http://localhost:6789/api/v1/scheduler/1
```

**Response:** 204 No Content

---

### Real-time Events

#### Subscribe to Event Stream

Connect to the Server-Sent Events stream for real-time updates.

```bash
curl -N -H "Accept: text/event-stream" \
  http://localhost:6789/api/v1/events
```

**Event Format:**

```
event: download_progress
data: {"id":1,"percent":45.2,"speed_bps":10485760}

event: download_complete
data: {"id":1,"path":"/downloads/Ubuntu.24.04.iso"}

event: queue_paused
data: {}
```

**Event Types:**

- `queued`: Download added to queue
- `removed`: Download removed from queue
- `download_progress`: Download progress update
- `download_complete`: Download finished successfully
- `download_failed`: Download failed
- `verifying`: PAR2 verification started
- `verify_complete`: PAR2 verification finished
- `repairing`: PAR2 repair started
- `repair_complete`: PAR2 repair finished
- `extracting`: Archive extraction started
- `extract_complete`: Archive extraction finished
- `moving`: Moving files to destination
- `cleaning`: Cleaning up temporary files
- `complete`: Job fully complete
- `failed`: Job failed at some stage
- `speed_limit_changed`: Global speed limit changed
- `queue_paused`: Queue paused
- `queue_resumed`: Queue resumed

---

## Common Workflows

### Complete Download Workflow

```bash
# 1. Add a download from URL
RESPONSE=$(curl -s -X POST http://localhost:6789/api/v1/downloads/url \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com/file.nzb","options":{"category":"movies","priority":"high"}}')

# 2. Extract the download ID
DOWNLOAD_ID=$(echo "$RESPONSE" | jq -r '.id')
echo "Created download: $DOWNLOAD_ID"

# 3. Monitor progress
while true; do
  DATA=$(curl -s "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID")
  STATUS=$(echo "$DATA" | jq -r '.status')
  PROGRESS=$(echo "$DATA" | jq -r '.progress')
  echo "Status: $STATUS, Progress: $PROGRESS%"

  if [ "$STATUS" = "complete" ] || [ "$STATUS" = "failed" ]; then
    break
  fi

  sleep 5
done

# 4. Check final result
curl -s "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID" | jq .
```

### Batch Operations

```bash
# Add multiple downloads
for url in "https://example.com/file1.nzb" "https://example.com/file2.nzb" "https://example.com/file3.nzb"; do
  curl -s -X POST http://localhost:6789/api/v1/downloads/url \
    -H "Content-Type: application/json" \
    -d "{\"url\":\"$url\",\"options\":{\"category\":\"batch\"}}"
done

# Check queue statistics
curl -s http://localhost:6789/api/v1/queue/stats | jq .

# Pause all downloads
curl -X POST http://localhost:6789/api/v1/queue/pause

# Resume all downloads
curl -X POST http://localhost:6789/api/v1/queue/resume
```

### Priority Management

```bash
# Add high-priority download
curl -X POST http://localhost:6789/api/v1/downloads/url \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com/urgent.nzb","options":{"priority":"high"}}'

# Change priority of existing download
DOWNLOAD_ID=5
curl -X PATCH "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/priority" \
  -H "Content-Type: application/json" \
  -d '{"priority":"force"}'
```

### Speed Limit Schedule

```bash
# Set unlimited speed at night (00:00 - 06:00)
curl -X POST http://localhost:6789/api/v1/scheduler \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Night Unlimited",
    "days": [],
    "start_time": "00:00",
    "end_time": "06:00",
    "action": {"Unlimited": null},
    "enabled": true
  }'

# Limit speed during work hours (09:00 - 17:00, weekdays)
curl -X POST http://localhost:6789/api/v1/scheduler \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Work Hours",
    "days": ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"],
    "start_time": "09:00",
    "end_time": "17:00",
    "action": {"SpeedLimit": {"limit_bps": 1048576}},
    "enabled": true
  }'
```

### History Cleanup

```bash
# Get current timestamp
NOW=$(date +%s)

# Delete history older than 30 days
THIRTY_DAYS_AGO=$((NOW - 2592000))
curl -X DELETE "http://localhost:6789/api/v1/history?before=$THIRTY_DAYS_AGO"

# Delete only failed downloads
curl -X DELETE "http://localhost:6789/api/v1/history?status=failed"

# Delete old failed downloads
curl -X DELETE "http://localhost:6789/api/v1/history?before=$THIRTY_DAYS_AGO&status=failed"
```

### Server Health Monitoring

```bash
# Test a new server before adding to config
curl -X POST http://localhost:6789/api/v1/servers/test \
  -H "Content-Type: application/json" \
  -d '{
    "host": "news.example.com",
    "port": 563,
    "tls": true,
    "username": "testuser",
    "password": "testpass",
    "connections": 10,
    "priority": 0
  }' | jq '.success, .latency, .capabilities'

# Test all configured servers
curl http://localhost:6789/api/v1/servers/test | jq '.[] | {server: .server, success: .result.success}'
```

### RSS Feed Automation

```bash
# Add RSS feed with filter
curl -X POST http://localhost:6789/api/v1/rss \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Ubuntu Releases",
    "url": "https://releases.ubuntu.com/rss.xml",
    "check_interval_secs": 900,
    "category": "software",
    "auto_download": true,
    "priority": "normal",
    "enabled": true,
    "filters": []
  }'

# Force immediate check
FEED_ID=1
curl -X POST "http://localhost:6789/api/v1/rss/$FEED_ID/check"

# Check how many items were queued
curl -s "http://localhost:6789/api/v1/rss/$FEED_ID/check" | jq '.queued'
```

---

## Rate Limiting

If rate limiting is enabled, you may receive a 429 response:

```json
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

Wait for the specified time before retrying.

## Additional Resources

- **Swagger UI**: http://localhost:6789/swagger-ui/ - Interactive API documentation
- **OpenAPI Spec**: http://localhost:6789/api/v1/openapi.json - Machine-readable API specification
- **Examples**: See `examples/rest_api_server.rs` for server setup
- **Test Script**: Use `test_api.sh` for automated endpoint testing
- **Manual Testing Guide**: See `tests/manual/api-testing.md` for testing workflows

## Support

For issues, questions, or contributions:
- GitHub Issues: https://github.com/jvz-devx/usenet-dl/issues
- Documentation: See README.md for getting started guide
