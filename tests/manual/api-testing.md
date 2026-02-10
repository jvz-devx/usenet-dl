# API Testing Guide

This guide provides comprehensive examples for testing all usenet-dl API endpoints using curl and Postman.

## Table of Contents

- [Quick Start](#quick-start)
- [Using the Test Script](#using-the-test-script)
- [Using Postman](#using-postman)
- [Manual Testing with curl](#manual-testing-with-curl)
- [Endpoint Reference](#endpoint-reference)

## Quick Start

### Prerequisites

1. **Start the API server** (requires a running usenet-dl instance)
2. **Default URL**: `http://localhost:6789/api/v1`
3. **Authentication**: Optional (set `API_KEY` environment variable if enabled)

### Health Check

```bash
curl http://localhost:6789/api/v1/health
```

Expected response:
```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

## Using the Test Script

The automated test script (`test_api.sh`) tests all implemented endpoints:

```bash
# Run with default URL
./test_api.sh

# Run with custom URL
./test_api.sh http://192.168.1.100:6789/api/v1

# Run with API key authentication
API_KEY="your-api-key-here" ./test_api.sh
```

The script will:
- ✅ Check server health
- ✅ Test all download endpoints
- ✅ Test queue management endpoints
- ✅ Test history endpoints
- ✅ Verify OpenAPI spec and Swagger UI
- ✅ Provide manual test examples for interactive operations

## Using Postman

### Import Collection

1. Open Postman
2. Click **Import** → **File** → Select `postman_collection.json`
3. The collection includes all endpoints organized by category

### Configure Variables

1. Click on the collection → **Variables** tab
2. Set the following:
   - `baseUrl`: `http://localhost:6789/api/v1` (or your server URL)
   - `apiKey`: Your API key (if authentication is enabled)
   - `downloadId`: ID of an existing download for testing individual operations

### Run Collection

1. Right-click the collection → **Run collection**
2. Select endpoints to test
3. View results in the runner

## Manual Testing with curl

### System Endpoints

#### Health Check

```bash
curl -X GET http://localhost:6789/api/v1/health
```

#### Get OpenAPI Specification

```bash
curl -X GET http://localhost:6789/api/v1/openapi.json | jq .
```

### Download Management

#### List All Downloads

```bash
curl -X GET http://localhost:6789/api/v1/downloads | jq .
```

#### Get Specific Download

```bash
DOWNLOAD_ID=1
curl -X GET "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID" | jq .
```

#### Add Download from File

```bash
curl -X POST http://localhost:6789/api/v1/downloads \
  -F "file=@/path/to/file.nzb" \
  -F 'options={"priority":"high","category":"movies"}'
```

#### Add Download from URL

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

#### Pause Download

```bash
DOWNLOAD_ID=1
curl -X POST "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/pause"
```

#### Resume Download

```bash
DOWNLOAD_ID=1
curl -X POST "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/resume"
```

#### Delete Download

```bash
DOWNLOAD_ID=1
curl -X DELETE "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID?delete_files=false"
```

With file deletion:
```bash
curl -X DELETE "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID?delete_files=true"
```

#### Set Download Priority

```bash
DOWNLOAD_ID=1
curl -X PATCH "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/priority" \
  -H "Content-Type: application/json" \
  -d '{"priority":"high"}'
```

Priority options: `low`, `normal`, `high`, `force`

#### Reprocess Download

Re-run full post-processing (verify, repair, extract):

```bash
DOWNLOAD_ID=1
curl -X POST "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/reprocess"
```

#### Reextract Download

Re-run extraction only (skip verify/repair):

```bash
DOWNLOAD_ID=1
curl -X POST "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/reextract"
```

### Queue Management

#### Get Queue Statistics

```bash
curl -X GET http://localhost:6789/api/v1/queue/stats | jq .
```

Expected response:
```json
{
  "total": 10,
  "queued": 5,
  "downloading": 2,
  "paused": 3,
  "processing": 0,
  "complete": 0,
  "failed": 0,
  "total_size_bytes": 5368709120,
  "downloaded_bytes": 1073741824,
  "speed_bps": 10485760
}
```

#### Pause All Downloads

```bash
curl -X POST http://localhost:6789/api/v1/queue/pause
```

#### Resume All Downloads

```bash
curl -X POST http://localhost:6789/api/v1/queue/resume
```

### History

#### Get All History

```bash
curl -X GET http://localhost:6789/api/v1/history | jq .
```

#### Get Paginated History

```bash
curl -X GET "http://localhost:6789/api/v1/history?limit=10&offset=0" | jq .
```

#### Get History Filtered by Status

```bash
# Get completed downloads only
curl -X GET "http://localhost:6789/api/v1/history?status=complete" | jq .

# Get failed downloads only
curl -X GET "http://localhost:6789/api/v1/history?status=failed" | jq .
```

#### Clear All History

```bash
curl -X DELETE http://localhost:6789/api/v1/history
```

#### Clear History Before Timestamp

```bash
# Delete history older than Jan 1, 2024
TIMESTAMP=1704067200
curl -X DELETE "http://localhost:6789/api/v1/history?before=$TIMESTAMP"
```

#### Clear History by Status

```bash
# Delete only failed downloads
curl -X DELETE "http://localhost:6789/api/v1/history?status=failed"

# Delete only completed downloads
curl -X DELETE "http://localhost:6789/api/v1/history?status=complete"
```

#### Clear History with Combined Filters

```bash
# Delete failed downloads older than a specific date
TIMESTAMP=1704067200
curl -X DELETE "http://localhost:6789/api/v1/history?before=$TIMESTAMP&status=failed"
```

### Authentication (If Enabled)

When API key authentication is enabled, include the `X-Api-Key` header:

```bash
API_KEY="your-api-key-here"

curl -X GET http://localhost:6789/api/v1/downloads \
  -H "X-Api-Key: $API_KEY"
```

### Server-Sent Events (Not Yet Implemented)

When implemented (Phase 3, Task 20), subscribe to real-time events:

```bash
curl -N -H "Accept: text/event-stream" \
  http://localhost:6789/api/v1/events
```

## Endpoint Reference

### Implemented Endpoints (Phase 3, Tasks 19.1-19.15)

| Method | Endpoint | Description | Status |
|--------|----------|-------------|--------|
| GET | `/health` | Health check | ✅ Implemented |
| GET | `/openapi.json` | OpenAPI spec | ✅ Implemented |
| GET | `/downloads` | List downloads | ✅ Implemented |
| GET | `/downloads/:id` | Get download | ✅ Implemented |
| POST | `/downloads` | Add from file | ✅ Implemented |
| POST | `/downloads/url` | Add from URL | ✅ Implemented |
| POST | `/downloads/:id/pause` | Pause download | ✅ Implemented |
| POST | `/downloads/:id/resume` | Resume download | ✅ Implemented |
| DELETE | `/downloads/:id` | Delete download | ✅ Implemented |
| PATCH | `/downloads/:id/priority` | Set priority | ✅ Implemented |
| POST | `/downloads/:id/reprocess` | Reprocess | ✅ Implemented |
| POST | `/downloads/:id/reextract` | Reextract | ✅ Implemented |
| POST | `/queue/pause` | Pause queue | ✅ Implemented |
| POST | `/queue/resume` | Resume queue | ✅ Implemented |
| GET | `/queue/stats` | Queue stats | ✅ Implemented |
| GET | `/history` | Get history | ✅ Implemented |
| DELETE | `/history` | Clear history | ✅ Implemented |

### Not Yet Implemented

| Method | Endpoint | Description | Phase |
|--------|----------|-------------|-------|
| GET | `/events` | SSE stream | Phase 3, Task 20 |
| GET | `/config` | Get config | Phase 3, Task 21 |
| PATCH | `/config` | Update config | Phase 3, Task 21 |
| GET | `/config/speed-limit` | Get speed limit | Phase 3, Task 21 |
| PUT | `/config/speed-limit` | Set speed limit | Phase 3, Task 21 |
| GET | `/categories` | List categories | Phase 3, Task 21 |
| PUT | `/categories/:name` | Create/update category | Phase 3, Task 21 |
| DELETE | `/categories/:name` | Delete category | Phase 3, Task 21 |

## Testing Workflow

### 1. Start the Server

```bash
# In development mode with test database
cd /path/to/usenet-dl
cargo run --example api_server
```

### 2. Run Automated Tests

```bash
./test_api.sh
```

### 3. Interactive Testing with Swagger UI

Open in browser: http://localhost:6789/swagger-ui/

The Swagger UI provides:
- 📖 Complete API documentation
- 🧪 Interactive "Try it out" testing
- 📝 Request/response schemas
- ✅ Real-time validation

### 4. Manual Workflow Testing

Example workflow: Add → Pause → Resume → Delete

```bash
# 1. Add a download from URL
RESPONSE=$(curl -s -X POST http://localhost:6789/api/v1/downloads/url \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com/test.nzb"}')

# 2. Extract download ID
DOWNLOAD_ID=$(echo "$RESPONSE" | jq -r '.id')
echo "Created download: $DOWNLOAD_ID"

# 3. Pause the download
curl -X POST "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/pause"

# 4. Check status
curl -s "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID" | jq '.status'

# 5. Resume the download
curl -X POST "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/resume"

# 6. Delete the download
curl -X DELETE "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID?delete_files=true"
```

## Error Handling

All errors follow a consistent format:

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

Common error codes:
- `not_found` (404): Resource doesn't exist
- `validation_error` (400): Invalid request data
- `internal_error` (500): Server error
- `rate_limited` (429): Too many requests (if rate limiting enabled)

## Advanced Testing

### Test with Multiple Downloads

```bash
# Add multiple downloads
for i in {1..5}; do
  curl -s -X POST http://localhost:6789/api/v1/downloads/url \
    -H "Content-Type: application/json" \
    -d "{\"url\":\"https://example.com/test$i.nzb\"}"
done

# Check queue stats
curl -s http://localhost:6789/api/v1/queue/stats | jq .

# Pause all
curl -X POST http://localhost:6789/api/v1/queue/pause

# Resume all
curl -X POST http://localhost:6789/api/v1/queue/resume
```

### Test Priority Changes

```bash
DOWNLOAD_ID=1

# Set to low priority
curl -X PATCH "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/priority" \
  -H "Content-Type: application/json" \
  -d '{"priority":"low"}'

# Set to high priority
curl -X PATCH "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/priority" \
  -H "Content-Type: application/json" \
  -d '{"priority":"high"}'

# Force immediate start
curl -X PATCH "http://localhost:6789/api/v1/downloads/$DOWNLOAD_ID/priority" \
  -H "Content-Type: application/json" \
  -d '{"priority":"force"}'
```

### Test History Management

```bash
# Get current timestamp
NOW=$(date +%s)

# Add test downloads (will complete quickly with empty NZBs)
# ... (add and complete some downloads)

# Get all history
curl -s http://localhost:6789/api/v1/history | jq '.items | length'

# Delete old entries (older than 1 day ago)
ONE_DAY_AGO=$((NOW - 86400))
curl -X DELETE "http://localhost:6789/api/v1/history?before=$ONE_DAY_AGO"

# Delete only failed downloads
curl -X DELETE "http://localhost:6789/api/v1/history?status=failed"

# Verify deletion
curl -s http://localhost:6789/api/v1/history | jq '.items | length'
```

## Troubleshooting

### Server Not Responding

```bash
# Check if server is running
curl http://localhost:6789/api/v1/health

# Check if port is in use
netstat -tuln | grep 6789

# Check server logs
tail -f /tmp/usenet-dl.log
```

### Authentication Errors

```bash
# Verify API key is set
echo $API_KEY

# Test with explicit header
curl -H "X-Api-Key: your-key-here" http://localhost:6789/api/v1/downloads
```

### JSON Parsing Errors

```bash
# Use jq to validate JSON
echo '{"priority":"high"}' | jq .

# Pretty-print responses
curl -s http://localhost:6789/api/v1/downloads | jq .
```

## Next Steps

After completing manual testing:

1. ✅ Verify all endpoints respond correctly
2. ✅ Test error cases (invalid IDs, malformed JSON, etc.)
3. ✅ Check Swagger UI completeness
4. 🔄 Proceed to Phase 3, Task 20 (SSE Events)
5. 🔄 Proceed to Phase 3, Task 21 (Config Endpoints)

## Resources

- **Swagger UI**: http://localhost:6789/swagger-ui/
- **OpenAPI Spec**: http://localhost:6789/api/v1/openapi.json
- **Test Script**: `./test_api.sh`
- **Postman Collection**: `postman_collection.json`
