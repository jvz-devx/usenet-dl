# Manual Testing Guides

This directory contains comprehensive guides for manually testing usenet-dl functionality that requires real NNTP servers, indexers, or interactive testing.

## Available Guides

### [API Testing Guide](api-testing.md)

Complete guide for testing the REST API endpoints.

**Covers:**
- REST API endpoint testing with curl and Postman
- Authentication and rate limiting
- Download management operations
- Server configuration testing
- Queue and priority management
- Health checks and monitoring

**Use when:**
- Testing API integration
- Validating endpoint behavior
- Developing API clients
- Debugging API issues

---

### [RSS Feed Testing Guide](rss-testing.md)

Guide for testing RSS feed monitoring and automatic download functionality.

**Covers:**
- RSS feed fetching and parsing
- Filter configuration (title, size, category)
- Automatic download triggers
- Schedule-based feed checking
- Feed history and duplicate detection
- Real indexer integration

**Use when:**
- Configuring RSS feeds from indexers
- Testing automatic download rules
- Validating filter logic
- Debugging RSS feed issues

---

### [Server Testing Guide](server-testing.md)

Guide for testing NNTP server connectivity and health checks.

**Covers:**
- NNTP server connection testing
- Authentication validation
- Server capability detection
- Latency measurement
- Connection troubleshooting
- Multi-server configuration

**Use when:**
- Adding new NNTP servers
- Validating credentials
- Troubleshooting connection issues
- Testing server failover

---

## Prerequisites

All manual testing guides assume you have:

1. **A running usenet-dl instance** (via example binary or custom integration)
2. **NNTP server credentials** (for download and server testing)
3. **Indexer access** (for RSS testing)
4. **HTTP client** (curl, Postman, or similar for API testing)

## Automated Testing

For automated testing with mocked dependencies, see the main test suite:

```bash
cargo test                    # Run all unit and integration tests
cargo test --test e2e_live    # Live NNTP tests (requires .env)
cargo test --test e2e_real_nzb -- --ignored  # Real NZB file tests
```

## When to Use Manual Testing

Use these manual testing guides when:

- **Automated tests are insufficient** - Testing real-world integrations
- **Interactive validation needed** - Verifying UI/API behavior
- **Third-party services involved** - RSS feeds, indexers, NNTP servers
- **Debugging production issues** - Reproducing specific scenarios
- **Configuration validation** - Testing new server/feed configurations

## Contributing

If you discover issues during manual testing:

1. Check if an automated test can reproduce the issue
2. File a bug report with reproduction steps
3. Consider contributing a test case

See [CONTRIBUTING.md](../../docs/contributing.md) for development guidelines.
