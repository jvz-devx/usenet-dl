use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Default, Clone)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub faults: FaultConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    #[serde(default = "default_greeting")]
    pub greeting: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            max_connections: default_max_connections(),
            greeting: default_greeting(),
        }
    }
}

fn default_port() -> u16 {
    1190
}

fn default_max_connections() -> usize {
    10
}

fn default_greeting() -> String {
    "200 fault-nntp-server ready".to_string()
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct FaultConfig {
    #[serde(default)]
    pub connection: ConnectionFaults,
    #[serde(default)]
    pub response: ResponseFaults,
    #[serde(default)]
    pub encoding: EncodingFaults,
    #[serde(default)]
    pub timing: TimingFaults,
    #[serde(default)]
    pub compression: CompressionFaults,
    #[serde(default)]
    pub multiline: MultilineFaults,
    #[serde(default)]
    pub article: ArticleFaults,
}

/// Connection-level faults (F1-F8)
#[derive(Debug, Deserialize, Default, Clone)]
pub struct ConnectionFaults {
    /// Probability to reject connection at TCP level
    #[serde(default)]
    pub reject_prob: f64,
    /// Milliseconds to hang before sending greeting
    #[serde(default)]
    pub hang_on_connect_ms: u64,
    /// Close connection immediately after greeting
    #[serde(default)]
    pub close_after_greeting: bool,
    /// Probability to send EOF immediately (no greeting)
    #[serde(default)]
    pub eof_on_greeting_prob: f64,
    /// Probability to send RST mid-connection
    #[serde(default)]
    pub rst_mid_connection_prob: f64,
    /// Close after every N commands (0 = disabled)
    #[serde(default)]
    pub close_after_commands: usize,
}

/// Response format faults (A1-A15, D1-D10)
#[derive(Debug, Deserialize, Default, Clone)]
pub struct ResponseFaults {
    /// Probability of malformed status line (A1-A15)
    #[serde(default)]
    pub malformed_status_prob: f64,
    /// Probability of invalid response code
    #[serde(default)]
    pub invalid_code_prob: f64,
    /// Probability to truncate response mid-stream (B1-B8)
    #[serde(default)]
    pub truncate_prob: f64,
    /// Probability to omit dot terminator on multiline
    #[serde(default)]
    pub missing_terminator_prob: f64,
    /// Probability of embedded CRLF in status message
    #[serde(default)]
    pub embedded_crlf_prob: f64,
    /// Probability to send multiple responses
    #[serde(default)]
    pub multiple_responses_prob: f64,
    /// Specific malformation type (empty = random)
    /// Options: "no_newline", "nul_bytes", "control_chars", "tab_space",
    ///          "code_overflow", "long_line", "missing_code", "letter_o"
    #[serde(default)]
    pub malformation_type: Option<String>,
}

/// Encoding faults (C1-C10)
#[derive(Debug, Deserialize, Default, Clone)]
pub struct EncodingFaults {
    /// Probability to inject invalid UTF-8 sequences
    #[serde(default)]
    pub invalid_utf8_prob: f64,
    /// Probability to inject NUL bytes
    #[serde(default)]
    pub nul_bytes_prob: f64,
    /// Probability to use wrong line endings (\n instead of \r\n)
    #[serde(default)]
    pub wrong_line_endings_prob: f64,
    /// Probability to prefix with BOM
    #[serde(default)]
    pub bom_prefix_prob: f64,
    /// Probability to use Latin-1 instead of UTF-8
    #[serde(default)]
    pub latin1_prob: f64,
}

/// Timing faults (E1-E8)
#[derive(Debug, Deserialize, Default, Clone)]
pub struct TimingFaults {
    /// Send data at this rate (bytes/sec, 0 = no limit)
    #[serde(default)]
    pub slow_drip_bytes_per_sec: usize,
    /// Probability to freeze mid-response
    #[serde(default)]
    pub freeze_mid_response_prob: f64,
    /// How long to freeze (ms)
    #[serde(default = "default_freeze_duration")]
    pub freeze_duration_ms: u64,
    /// Freeze before sending terminator
    #[serde(default)]
    pub freeze_before_terminator: bool,
    /// Delay before each response (ms)
    #[serde(default)]
    pub response_delay_ms: u64,
}

fn default_freeze_duration() -> u64 {
    5000
}

/// Compression faults (G1-G10)
#[derive(Debug, Deserialize, Default, Clone)]
pub struct CompressionFaults {
    /// Probability to corrupt GZIP data
    #[serde(default)]
    pub corrupt_gzip_prob: f64,
    /// Probability to truncate compressed stream
    #[serde(default)]
    pub truncate_compressed_prob: f64,
    /// Probability to send [COMPRESS=GZIP] but plaintext
    #[serde(default)]
    pub fake_marker_prob: f64,
    /// Probability to send compressed without marker
    #[serde(default)]
    pub missing_marker_prob: f64,
    /// Probability to send decompression bomb
    #[serde(default)]
    pub decompression_bomb_prob: f64,
    /// Max size for decompression bomb (bytes)
    #[serde(default = "default_bomb_size")]
    pub bomb_expanded_size: usize,
}

fn default_bomb_size() -> usize {
    10 * 1024 * 1024 // 10MB default
}

/// Multiline parsing faults (H1-H10)
#[derive(Debug, Deserialize, Default, Clone)]
pub struct MultilineFaults {
    /// Probability to use lone \r as line terminator
    #[serde(default)]
    pub lone_cr_prob: f64,
    /// Probability to mix line ending styles
    #[serde(default)]
    pub mixed_line_endings_prob: f64,
    /// Send very long lines (bytes, 0 = disabled)
    #[serde(default)]
    pub very_long_line_bytes: usize,
    /// Probability to inject NUL in body
    #[serde(default)]
    pub nul_in_body_prob: f64,
    /// Probability to send double terminator
    #[serde(default)]
    pub double_terminator_prob: f64,
}

/// Article-specific faults (J1-J10)
#[derive(Debug, Deserialize, Default, Clone)]
pub struct ArticleFaults {
    /// Probability to return wrong message-id
    #[serde(default)]
    pub wrong_message_id_prob: f64,
    /// Probability to omit headers
    #[serde(default)]
    pub missing_headers_prob: f64,
    /// Probability to duplicate headers
    #[serde(default)]
    pub duplicate_headers_prob: f64,
    /// Probability to corrupt yEnc data
    #[serde(default)]
    pub yenc_corruption_prob: f64,
    /// Probability of CRC mismatch
    #[serde(default)]
    pub crc_mismatch_prob: f64,
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| ConfigError::Io(e.to_string()))?;
        toml::from_str(&content).map_err(|e| ConfigError::Parse(e.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(String),
    #[error("Parse error: {0}")]
    Parse(String),
}
