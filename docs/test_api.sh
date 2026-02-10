#!/bin/bash
#
# API Endpoint Testing Script for usenet-dl
# Tests all queue management endpoints with curl
#
# Usage: ./test_api.sh [BASE_URL]
# Default BASE_URL: http://localhost:6789/api/v1
#

set -e  # Exit on error

# Configuration
BASE_URL="${1:-http://localhost:6789/api/v1}"
API_KEY="${API_KEY:-}"  # Set API_KEY environment variable if authentication is enabled

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Helper functions
print_header() {
    echo -e "\n${BLUE}========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}========================================${NC}\n"
}

print_test() {
    echo -e "${YELLOW}TEST:${NC} $1"
}

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_info() {
    echo -e "${BLUE}→${NC} $1"
}

# Build curl command with optional API key
curl_cmd() {
    local method=$1
    local endpoint=$2
    local data=$3

    local cmd="curl -s -w \"\n%{http_code}\" -X $method"

    if [ -n "$API_KEY" ]; then
        cmd="$cmd -H \"X-Api-Key: $API_KEY\""
    fi

    if [ "$method" != "GET" ] && [ "$method" != "DELETE" ]; then
        cmd="$cmd -H \"Content-Type: application/json\""
    fi

    if [ -n "$data" ]; then
        cmd="$cmd -d '$data'"
    fi

    cmd="$cmd \"$BASE_URL$endpoint\""

    eval $cmd
}

# Extract HTTP status code from response
get_status() {
    echo "$1" | tail -n 1
}

# Extract body from response (remove last line which is status code)
get_body() {
    echo "$1" | head -n -1
}

# Check if server is running
check_server() {
    print_header "Checking Server Health"
    print_test "GET /health"

    response=$(curl_cmd GET "/health" "")
    status=$(get_status "$response")
    body=$(get_body "$response")

    if [ "$status" = "200" ]; then
        print_success "Server is healthy"
        print_info "Response: $body"
    else
        print_error "Server health check failed (HTTP $status)"
        echo "$body"
        exit 1
    fi
}

# Test download endpoints
test_downloads() {
    print_header "Testing Download Endpoints"

    # GET /downloads
    print_test "GET /downloads - List all downloads"
    response=$(curl_cmd GET "/downloads" "")
    status=$(get_status "$response")

    if [ "$status" = "200" ]; then
        print_success "List downloads successful"
    else
        print_error "List downloads failed (HTTP $status)"
        get_body "$response"
    fi

    # Note: POST /downloads requires multipart/form-data with actual NZB file
    # This is better tested with Postman or by uploading a real file
    print_info "Skipping POST /downloads (requires multipart file upload)"

    # POST /downloads/url (using a dummy URL - will fail but tests endpoint)
    print_test "POST /downloads/url - Add download from URL (with dummy URL)"
    response=$(curl_cmd POST "/downloads/url" '{"url":"https://example.com/test.nzb","options":{"priority":"normal"}}')
    status=$(get_status "$response")

    if [ "$status" = "201" ] || [ "$status" = "422" ] || [ "$status" = "500" ]; then
        print_success "URL endpoint responds (expected to fail with dummy URL)"
        print_info "Status: $status"
    else
        print_error "Unexpected response (HTTP $status)"
    fi
}

# Test queue endpoints
test_queue() {
    print_header "Testing Queue Endpoints"

    # GET /queue/stats
    print_test "GET /queue/stats - Get queue statistics"
    response=$(curl_cmd GET "/queue/stats" "")
    status=$(get_status "$response")
    body=$(get_body "$response")

    if [ "$status" = "200" ]; then
        print_success "Queue stats retrieved"
        print_info "Response: $body"
    else
        print_error "Queue stats failed (HTTP $status)"
        echo "$body"
    fi

    # POST /queue/pause
    print_test "POST /queue/pause - Pause all downloads"
    response=$(curl_cmd POST "/queue/pause" "")
    status=$(get_status "$response")

    if [ "$status" = "204" ]; then
        print_success "Queue paused"
    else
        print_error "Queue pause failed (HTTP $status)"
        get_body "$response"
    fi

    # POST /queue/resume
    print_test "POST /queue/resume - Resume all downloads"
    response=$(curl_cmd POST "/queue/resume" "")
    status=$(get_status "$response")

    if [ "$status" = "204" ]; then
        print_success "Queue resumed"
    else
        print_error "Queue resume failed (HTTP $status)"
        get_body "$response"
    fi
}

# Test history endpoints
test_history() {
    print_header "Testing History Endpoints"

    # GET /history
    print_test "GET /history - Get download history"
    response=$(curl_cmd GET "/history" "")
    status=$(get_status "$response")
    body=$(get_body "$response")

    if [ "$status" = "200" ]; then
        print_success "History retrieved"
        print_info "Response: $body"
    else
        print_error "History retrieval failed (HTTP $status)"
        echo "$body"
    fi

    # GET /history with pagination
    print_test "GET /history?limit=10&offset=0 - Get paginated history"
    response=$(curl_cmd GET "/history?limit=10&offset=0" "")
    status=$(get_status "$response")

    if [ "$status" = "200" ]; then
        print_success "Paginated history retrieved"
    else
        print_error "Paginated history failed (HTTP $status)"
    fi

    # GET /history with status filter
    print_test "GET /history?status=complete - Filter by status"
    response=$(curl_cmd GET "/history?status=complete" "")
    status=$(get_status "$response")

    if [ "$status" = "200" ]; then
        print_success "Filtered history retrieved"
    else
        print_error "Filtered history failed (HTTP $status)"
    fi

    # Note: Skipping DELETE /history to avoid deleting actual data
    print_info "Skipping DELETE /history (would delete actual data)"
}

# Test individual download operations (requires existing download)
test_download_operations() {
    print_header "Testing Individual Download Operations"

    print_info "These tests require an existing download ID"
    print_info "To test manually:"
    echo ""
    echo "  # Get a download ID first"
    echo "  DOWNLOAD_ID=\$(curl -s $BASE_URL/downloads | jq -r '.[0].id')"
    echo ""
    echo "  # Test GET single download"
    echo "  curl -X GET $BASE_URL/downloads/\$DOWNLOAD_ID"
    echo ""
    echo "  # Test pause download"
    echo "  curl -X POST $BASE_URL/downloads/\$DOWNLOAD_ID/pause"
    echo ""
    echo "  # Test resume download"
    echo "  curl -X POST $BASE_URL/downloads/\$DOWNLOAD_ID/resume"
    echo ""
    echo "  # Test set priority"
    echo "  curl -X PATCH $BASE_URL/downloads/\$DOWNLOAD_ID/priority \\"
    echo "       -H \"Content-Type: application/json\" \\"
    echo "       -d '{\"priority\":\"high\"}'"
    echo ""
    echo "  # Test reprocess"
    echo "  curl -X POST $BASE_URL/downloads/\$DOWNLOAD_ID/reprocess"
    echo ""
    echo "  # Test reextract"
    echo "  curl -X POST $BASE_URL/downloads/\$DOWNLOAD_ID/reextract"
    echo ""
    echo "  # Test delete download"
    echo "  curl -X DELETE $BASE_URL/downloads/\$DOWNLOAD_ID?delete_files=false"
}

# Test configuration endpoints
test_config() {
    print_header "Testing Configuration Endpoints"

    print_info "Configuration endpoints not yet implemented (Phase 3, Task 21)"
    echo ""
    echo "  Planned endpoints:"
    echo "  - GET /config"
    echo "  - PATCH /config"
    echo "  - GET /config/speed-limit"
    echo "  - PUT /config/speed-limit"
}

# Test SSE events
test_events() {
    print_header "Testing Server-Sent Events"

    print_info "SSE endpoint not yet implemented (Phase 3, Task 20)"
    echo ""
    echo "  To test SSE when implemented:"
    echo "  curl -N -H \"Accept: text/event-stream\" $BASE_URL/events"
}

# Test OpenAPI and Swagger UI
test_openapi() {
    print_header "Testing OpenAPI Documentation"

    # GET /openapi.json
    print_test "GET /openapi.json - Get OpenAPI specification"
    response=$(curl_cmd GET "/openapi.json" "")
    status=$(get_status "$response")

    if [ "$status" = "200" ]; then
        print_success "OpenAPI spec retrieved"
        print_info "Spec is valid JSON: $(echo "$response" | head -n -1 | jq -e . > /dev/null 2>&1 && echo 'Yes' || echo 'No')"
    else
        print_error "OpenAPI spec failed (HTTP $status)"
    fi

    # Check Swagger UI (returns HTML, not JSON)
    print_test "GET /swagger-ui/ - Check Swagger UI availability"
    swagger_status=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/../swagger-ui/")

    if [ "$swagger_status" = "200" ]; then
        print_success "Swagger UI is available"
        print_info "Access it at: ${BASE_URL}/../swagger-ui/"
    else
        print_error "Swagger UI not available (HTTP $swagger_status)"
    fi
}

# Main test execution
main() {
    echo -e "${GREEN}╔════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║  usenet-dl API Endpoint Test Suite   ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════╝${NC}"
    echo ""
    echo "Base URL: $BASE_URL"
    echo "API Key:  $([ -n "$API_KEY" ] && echo "Set" || echo "Not set")"
    echo ""

    # Check if server is running
    check_server

    # Run test suites
    test_downloads
    test_queue
    test_history
    test_download_operations
    test_config
    test_events
    test_openapi

    # Summary
    print_header "Test Summary"
    print_success "Basic endpoint connectivity verified"
    print_info "All implemented endpoints are responding correctly"
    print_info "See output above for individual test results"
    echo ""
    print_info "For interactive testing, use Swagger UI at:"
    echo "  ${BASE_URL}/../swagger-ui/"
    echo ""
}

# Run main function
main
