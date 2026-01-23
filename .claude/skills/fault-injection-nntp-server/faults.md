# Fault Injection Catalog

Complete catalog of injectable NNTP fault scenarios. Each fault is identified by a code for easy reference in logs and configuration.

## A. Status Line Malformations

| Code | Fault | Example | Expected Client Behavior |
|------|-------|---------|--------------------------|
| A1 | Response with only `\r` | `200 OK\r` (no `\n`) | Should timeout or handle partial line |
| A2 | NUL bytes in status | `200\x00OK\r\n` | Handle via lossy UTF-8 or reject |
| A3 | Control characters | `200\x01\x02\x03OK\r\n` | Strip or reject |
| A4 | Tab instead of space | `200\tOK\r\n` | May parse incorrectly |
| A5 | Code overflow | `99999 message\r\n` | Parse as 999 or error |
| A6 | Very long line (10KB+) | `200 ` + 10KB + `\r\n` | Buffer overflow protection |
| A7 | Empty response | `` (0 bytes then EOF) | Connection closed error |
| A8 | Whitespace only | `   \r\n` | Invalid response error |
| A9 | Missing code | `OK ready\r\n` | Parse error |
| A10 | Non-ASCII digits | `2O0 OK\r\n` (letter O) | Parse error |
| A11 | Negative code | `-200 OK\r\n` | Parse error |
| A12 | Leading zeros | `007 OK\r\n` | Should parse as 7 |
| A13 | Double space after code | `200  OK\r\n` | May lose first space |
| A14 | No message | `200\r\n` | Valid, empty message |
| A15 | Code only, no CRLF | `200` (then EOF) | Incomplete response |

## B. Truncated/Incomplete Responses

| Code | Fault | Description | Expected Client Behavior |
|------|-------|-------------|--------------------------|
| B1 | Multiline no terminator | Status OK, body sent, no `.` terminator, then disconnect | Timeout or connection closed |
| B2 | Mid-line disconnect | `200 this is a long mess...` then EOF | Incomplete response error |
| B3 | Partial dot-stuffed | `..` sent, then disconnect before line end | Handle partial line |
| B4 | Compressed truncated | GZIP stream cut mid-way | Decompression error |
| B5 | EOF after status | `200 OK\r\n` then immediate EOF (multiline expected) | Should detect missing body |
| B6 | Partial terminator | `.\r` sent, no `\n`, then disconnect | Timeout or error |
| B7 | Body without headers | Article body with no header section | Protocol violation |
| B8 | Headers without body | Headers only, missing blank line and body | May work if only headers requested |

## C. Encoding Issues

| Code | Fault | Example | Expected Client Behavior |
|------|-------|---------|--------------------------|
| C1 | Invalid UTF-8 lone surrogate | `200 \xED\xA0\x80 OK\r\n` | Lossy replacement |
| C2 | Invalid multibyte | `200 \xC0\x80 OK\r\n` | Lossy replacement |
| C3 | `\xFF\xFE` sequence | `200 \xFF\xFE OK\r\n` | Lossy replacement |
| C4 | BOM prefix | `\xEF\xBB\xBF200 OK\r\n` | May fail to parse code |
| C5 | Mixed valid/invalid | `200 valid \xFF invalid\r\n` | Partial replacement |
| C6 | Overlong encoding | `200 \xF0\x82\x82\xAC OK\r\n` | Reject overlong |
| C7 | 5-byte sequence | `200 \xF8\x80\x80\x80\x80\r\n` | Invalid UTF-8 |
| C8 | Truncated multibyte | `200 \xC2\r\n` (incomplete) | Replacement char |
| C9 | Latin-1 assumed | `200 caf\xE9\r\n` | Lossy if expecting UTF-8 |
| C10 | Mixed encodings | UTF-8 headers, Latin-1 body | Partial failures |

## D. Protocol Violations

| Code | Fault | Example | Expected Client Behavior |
|------|-------|---------|--------------------------|
| D1 | Missing space | `200OK\r\n` | Message starts at char 4 |
| D2 | Embedded CRLF | `200 line1\r\nline2\r\n` | Second line left in buffer |
| D3 | Multiple responses | `200 OK\r\n201 Ready\r\n` | Second response unexpected |
| D4 | Code 000 | `000 test\r\n` | Valid parse, undefined meaning |
| D5 | Code > 999 | `1234 test\r\n` | Parse as 123, message "4 test" |
| D6 | Embedded `\r` only | `200 line1\r line2\r\n` | Handle internal CR |
| D7 | Response to wrong command | Send XOVER response to ARTICLE | Protocol state error |
| D8 | Unsolicited response | Server sends data without command | Unexpected data |
| D9 | Wrong response code | 200 instead of 220 for ARTICLE | May work, wrong semantics |
| D10 | Case sensitivity | `200 ok\r\n` vs `200 OK\r\n` | Should be case-insensitive |

## E. Timeout Scenarios

| Code | Fault | Description | Expected Client Behavior |
|------|-------|-------------|--------------------------|
| E1 | Slow greeting | 30+ second delay before greeting | Connection timeout |
| E2 | Slow drip | 1 byte every 5 seconds | Read timeout or very slow |
| E3 | Freeze after status | Status line OK, then freeze | Read timeout (180s for multiline) |
| E4 | Freeze before terminator | All body received, freeze before `.` | Timeout waiting for end |
| E5 | Freeze mid-line | `200 this is ` then freeze | Read timeout |
| E6 | Intermittent freeze | Random freezes during transfer | Multiple timeouts |
| E7 | Slow authentication | Delay after AUTH commands | Auth timeout |
| E8 | Slow group selection | Delay after GROUP command | Command timeout |

## F. Connection Issues

| Code | Fault | Description | Expected Client Behavior |
|------|-------|-------------|--------------------------|
| F1 | EOF on greeting | Connect succeeds, immediate EOF | Connection closed error |
| F2 | EOF mid-multiline | Body partially sent, then EOF | Incomplete response |
| F3 | RST mid-read | TCP RST during response | Connection reset error |
| F4 | Graceful close partial | FIN after partial response | Incomplete data |
| F5 | Connection refused | Reject at TCP level | Connection refused |
| F6 | Half-close | Server closes write, keeps reading | May work or timeout |
| F7 | Delayed close | Close connection 10s after last byte | Affects connection pooling |
| F8 | Reconnect required | Close after every command | Connection reuse failure |

## G. Compression Edge Cases

| Code | Fault | Description | Expected Client Behavior |
|------|-------|-------------|--------------------------|
| G1 | GZIP header only | Magic bytes, no compressed data | Decompression error |
| G2 | Corrupt CRC | Valid GZIP with wrong checksum | CRC validation error |
| G3 | Fake marker | `[COMPRESS=GZIP]` but plaintext | Decompression fails on plaintext |
| G4 | Missing marker | Compressed data, no `[COMPRESS=GZIP]` | Garbage if not detected |
| G5 | Truncated deflate | DEFLATE stream incomplete | Decompression error |
| G6 | Decompression bomb | 100 bytes → 1GB expanded | Resource exhaustion |
| G7 | Wrong compression | DEFLATE marked as GZIP | Wrong algorithm |
| G8 | Double compression | Compressed twice | Only first layer decoded |
| G9 | Empty compressed | Valid GZIP of empty content | Should produce empty output |
| G10 | Corrupt mid-stream | Valid start, corrupt middle | Partial decompression |

## H. Multiline Parsing

| Code | Fault | Description | Expected Client Behavior |
|------|-------|-------------|--------------------------|
| H1 | Lone `\r` terminator | `.\r` (no `\n`) | Timeout or partial |
| H2 | Mixed line endings | Some lines `\r\n`, others `\n` | Should handle both |
| H3 | Very long line | 100MB single line | Buffer limits |
| H4 | NUL in body | `data\x00more\r\n` | Preserve or strip |
| H5 | Only dots | `..\r\n..\r\n..\r\n.\r\n` | Destuff to single dots |
| H6 | Dot mid-line | `prefix.suffix\r\n` | NOT terminator |
| H7 | Double terminator | `.\r\n.\r\n` | Second stays in buffer |
| H8 | Empty body | Status, then `.\r\n` immediately | Valid empty response |
| H9 | Unicode in body | Full UTF-8 article content | Proper encoding |
| H10 | Binary in text | Raw bytes in supposedly text response | Handle or error |

## I. Real-World Malformed Responses

| Code | Fault | Description | Expected Client Behavior |
|------|-------|-------------|--------------------------|
| I1 | HTML error page | `<html><body>500 Error</body></html>` | Not NNTP protocol |
| I2 | Letter O for zero | `2OO OK\r\n` | Parse failure |
| I3 | Extra after QUIT | Data sent after 205 response | Should ignore |
| I4 | Double space | `200  OK\r\n` | May affect parsing |
| I5 | Trailing spaces | `200 OK   \r\n` | Should trim |
| I6 | Tab in message | `200 OK\there\r\n` | Preserve or normalize |
| I7 | JSON error | `{"error": "not found"}` | Not NNTP protocol |
| I8 | Empty lines before response | `\r\n\r\n200 OK\r\n` | Skip empty lines |
| I9 | Garbage prefix | `XXX200 OK\r\n` | Parse failure |
| I10 | CRLF CRLF | `200 OK\r\n\r\n` | Extra blank line |

## J. Article-Specific Faults

| Code | Fault | Description | Expected Client Behavior |
|------|-------|-------------|--------------------------|
| J1 | Wrong message-id | Requested `<abc>`, returned `<xyz>` | ID mismatch error |
| J2 | Missing headers | Article with no headers section | Parse failure |
| J3 | Duplicate headers | Two `Subject:` headers | Use first/last/error |
| J4 | Huge headers | 1MB single header line | Buffer limits |
| J5 | No blank line | Headers run into body | Parse confusion |
| J6 | yEnc corruption | Invalid yEnc encoding | Decode failure |
| J7 | Wrong part count | yEnc says 3 parts, only 2 sent | Incomplete file |
| J8 | CRC mismatch | yEnc CRC doesn't match data | Verification failure |
| J9 | Size mismatch | Stated size differs from actual | Verification failure |
| J10 | Interleaved parts | Part 2 arrives before part 1 | Reordering needed |

## Usage in Configuration

Enable faults by setting non-zero probabilities:

```toml
[faults.response]
# Enable faults A1-A5 with 5% probability each
malformed_status_prob = 0.05

[faults.encoding]
# Enable faults C1-C5 with 2% probability
invalid_utf8_prob = 0.02

[faults.timing]
# Enable fault E2 - slow drip at 100 bytes/sec
slow_drip_bytes_per_sec = 100
```

## Fault Selection by Scenario

### Testing Retry Logic
- E1-E8 (timeouts)
- F1-F8 (connection issues)
- B1-B8 (truncated responses)

### Testing Parser Robustness
- A1-A15 (status line)
- D1-D10 (protocol violations)
- I1-I10 (real-world malformed)

### Testing Encoding Handling
- C1-C10 (encoding issues)
- H1-H10 (multiline parsing)

### Testing Compression
- G1-G10 (compression faults)

### Chaos/Stress Testing
- Enable all categories with low probability (1-5%)
