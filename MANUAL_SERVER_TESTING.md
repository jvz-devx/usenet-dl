# Manual Server Health Check Testing Guide

This guide provides instructions for manually testing the server health check functionality with real NNTP servers.

## Overview

The server health check API allows you to test NNTP server connectivity, authentication, and capabilities before adding servers to production. This is useful for:

- Validating server credentials
- Testing network connectivity
- Discovering server capabilities (posting, compression, etc.)
- Troubleshooting connection issues

## Prerequisites

1. **Running usenet-dl instance** - Start the server with your configuration
2. **NNTP server credentials** - You'll need access to at least one NNTP server
3. **HTTP client** - curl, Postman, or any HTTP client

## API Endpoints

### 1. POST /api/v1/servers/test

Test a specific server configuration without adding it to your config.

**Request:**
```bash
curl -X POST http://localhost:6789/api/v1/servers/test \
  -H "Content-Type: application/json" \
  -d '{
    "host": "news.example.com",
    "port": 563,
    "tls": true,
    "username": "your_username",
    "password": "your_password",
    "connections": 10,
    "priority": 0
  }'
```

**Response (Success):**
```json
{
  "success": true,
  "latency": {
    "secs": 0,
    "nanos": 234567890
  },
  "error": null,
  "capabilities": {
    "posting_allowed": false,
    "max_connections": null,
    "compression": true
  }
}
```

**Response (Failure):**
```json
{
  "success": false,
  "latency": {
    "secs": 2,
    "nanos": 123456789
  },
  "error": "Connection refused",
  "capabilities": null
}
```

### 2. GET /api/v1/servers/test

Test all servers currently configured in your config.

**Request:**
```bash
curl http://localhost:6789/api/v1/servers/test
```

**Response:**
```json
[
  [
    "news.server1.com",
    {
      "success": true,
      "latency": {"secs": 0, "nanos": 345678901},
      "error": null,
      "capabilities": {
        "posting_allowed": false,
        "max_connections": null,
        "compression": true
      }
    }
  ],
  [
    "news.server2.com",
    {
      "success": false,
      "latency": {"secs": 1, "nanos": 567890123},
      "error": "Authentication failed",
      "capabilities": null
    }
  ]
]
```

## Test Scenarios

### Scenario 1: Valid Server Connection

**Goal:** Verify successful connection to a working NNTP server

**Steps:**
1. Obtain credentials for a working NNTP server
2. Use POST /servers/test with correct credentials
3. Verify response shows `success: true`
4. Check that `capabilities` are populated
5. Note the latency value

**Expected Result:**
- Success = true
- Latency < 5 seconds (typically < 1 second)
- Error = null
- Capabilities show server features

### Scenario 2: Invalid Credentials

**Goal:** Test authentication failure handling

**Steps:**
1. Use POST /servers/test with incorrect password
2. Verify response shows `success: false`
3. Check error message indicates authentication failure

**Expected Result:**
- Success = false
- Error message mentions authentication/credentials
- Capabilities = null

### Scenario 3: Invalid Hostname

**Goal:** Test connection failure to non-existent server

**Steps:**
1. Use POST /servers/test with made-up hostname (e.g., "nonexistent.invalid")
2. Verify response shows `success: false`
3. Check error message indicates connection failure

**Expected Result:**
- Success = false
- Error message mentions DNS resolution or connection refused
- Capabilities = null
- Latency may be present (showing how long before timeout)

### Scenario 4: Wrong Port

**Goal:** Test connection to wrong port

**Steps:**
1. Use POST /servers/test with correct host but wrong port (e.g., port 80 instead of 563)
2. Verify response shows `success: false`
3. Check error message

**Expected Result:**
- Success = false
- Error message indicates protocol error or connection refused
- Capabilities = null

### Scenario 5: TLS vs Non-TLS

**Goal:** Test TLS misconfiguration

**Steps:**
1. Test with `tls: true` on non-TLS port (119)
2. Test with `tls: false` on TLS port (563)
3. Compare error messages

**Expected Result:**
- Both should fail with TLS-related errors
- Helps diagnose TLS configuration issues

### Scenario 6: Batch Testing

**Goal:** Test multiple servers at once

**Steps:**
1. Add 2-3 servers to your config
2. Use GET /servers/test
3. Verify all servers are tested
4. Check that results are ordered correctly

**Expected Result:**
- Array contains results for all configured servers
- Each result has hostname and test result
- Failed servers don't prevent testing of other servers

## Capability Detection

The health check detects these capabilities:

- **posting_allowed**: Server supports POST or IHAVE (can upload articles)
- **compression**: Server supports COMPRESS or XZVER (can compress transfers)
- **max_connections**: Maximum concurrent connections (not standardized, usually null)

## Common Error Messages

| Error Message | Cause | Solution |
|--------------|-------|----------|
| "Connection refused" | Server not running or firewall blocking | Check hostname, port, firewall |
| "Authentication failed" | Wrong username/password | Verify credentials |
| "DNS resolution failed" | Invalid hostname | Check hostname spelling |
| "TLS handshake failed" | TLS misconfiguration | Check TLS setting matches server |
| "Timeout" | Network issues or slow server | Check network, increase timeout |
| "Protocol error" | Wrong port or non-NNTP service | Verify port number (119/563) |

## Performance Benchmarks

Typical latency ranges:

- **Local network**: < 10ms
- **Same datacenter**: 10-50ms
- **Same country**: 50-200ms
- **International**: 200-500ms
- **> 1 second**: Investigate network issues

## Integration with Swagger UI

You can test the server health check directly from Swagger UI:

1. Navigate to http://localhost:6789/swagger-ui/
2. Find the "servers" section
3. Click "POST /api/v1/servers/test"
4. Click "Try it out"
5. Fill in the server configuration JSON
6. Click "Execute"
7. View the response

## Automated Testing Script

Here's a bash script to test multiple configurations:

```bash
#!/bin/bash

# Test script for server health checks
BASE_URL="http://localhost:6789/api/v1"

echo "Testing Server Health Checks"
echo "=============================="

# Test 1: Valid server
echo -e "\n1. Testing valid server..."
curl -s -X POST "$BASE_URL/servers/test" \
  -H "Content-Type: application/json" \
  -d '{
    "host": "news.example.com",
    "port": 563,
    "tls": true,
    "username": "your_username",
    "password": "your_password",
    "connections": 10,
    "priority": 0
  }' | jq '.'

# Test 2: Invalid hostname
echo -e "\n2. Testing invalid hostname..."
curl -s -X POST "$BASE_URL/servers/test" \
  -H "Content-Type: application/json" \
  -d '{
    "host": "nonexistent.invalid",
    "port": 563,
    "tls": true,
    "username": null,
    "password": null,
    "connections": 10,
    "priority": 0
  }' | jq '.'

# Test 3: Test all configured servers
echo -e "\n3. Testing all configured servers..."
curl -s "$BASE_URL/servers/test" | jq '.'

echo -e "\n=============================="
echo "Testing complete!"
```

Make it executable: `chmod +x test_server_health.sh`

## Troubleshooting

### Issue: All tests timeout

**Possible causes:**
- usenet-dl server not running
- Firewall blocking outbound NNTP connections
- ISP blocking NNTP ports

**Solution:**
- Verify usenet-dl is running: `curl http://localhost:6789/health`
- Check firewall rules
- Try different port numbers
- Use VPN if ISP blocks NNTP

### Issue: Authentication always fails

**Possible causes:**
- Wrong credentials
- Server requires specific username format
- Account not activated

**Solution:**
- Verify credentials in server's web portal
- Check if username needs domain (e.g., `user@example.com`)
- Contact server provider

### Issue: Capabilities not detected

**Possible causes:**
- Server doesn't advertise capabilities
- Old NNTP server version

**Solution:**
- This is normal for some servers
- Capabilities will be null but connection still works
- Server functionality not affected

## Example Real-World Test Session

```bash
# 1. Test primary server
$ curl -X POST http://localhost:6789/api/v1/servers/test \
  -H "Content-Type: application/json" \
  -d '{
    "host": "news.eweka.nl",
    "port": 563,
    "tls": true,
    "username": "myuser",
    "password": "mypass",
    "connections": 20,
    "priority": 0
  }' | jq '.'

{
  "success": true,
  "latency": {
    "secs": 0,
    "nanos": 156789012
  },
  "error": null,
  "capabilities": {
    "posting_allowed": false,
    "max_connections": null,
    "compression": true
  }
}

# 2. Test backup server
$ curl -X POST http://localhost:6789/api/v1/servers/test \
  -H "Content-Type: application/json" \
  -d '{
    "host": "news.usenetserver.com",
    "port": 563,
    "tls": true,
    "username": "myuser2",
    "password": "mypass2",
    "connections": 10,
    "priority": 1
  }' | jq '.'

{
  "success": true,
  "latency": {
    "secs": 0,
    "nanos": 234567890
  },
  "error": null,
  "capabilities": {
    "posting_allowed": false,
    "max_connections": null,
    "compression": false
  }
}
```

## Next Steps

After successful testing:

1. Add working servers to your config file
2. Monitor connection stability in production
3. Set up multiple backup servers for redundancy
4. Configure connection limits based on your subscription
5. Test server priorities to optimize download speed

## Support

If you encounter issues:

1. Check the usenet-dl logs for detailed error messages
2. Verify network connectivity: `ping news.example.com`
3. Test with basic NNTP client: `telnet news.example.com 119`
4. Contact your NNTP provider's support
5. File a bug report with test results and logs
