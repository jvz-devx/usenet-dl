//! Configuration types for usenet-dl

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, time::Duration};

/// Main configuration for UsenetDownloader
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// NNTP server configurations (at least one required)
    pub servers: Vec<ServerConfig>,

    /// Download directory (default: "./downloads")
    #[serde(default = "default_download_dir")]
    pub download_dir: PathBuf,

    /// Temporary directory (default: "./temp")
    #[serde(default = "default_temp_dir")]
    pub temp_dir: PathBuf,

    /// Maximum concurrent downloads (default: 3)
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_downloads: usize,

    /// Speed limit in bytes per second (None = unlimited)
    #[serde(default)]
    pub speed_limit_bps: Option<u64>,

    /// Retry configuration
    #[serde(default)]
    pub retry: RetryConfig,

    /// Default post-processing mode
    #[serde(default)]
    pub default_post_process: PostProcess,

    /// Action to take when post-processing fails
    #[serde(default)]
    pub failed_action: FailedDownloadAction,

    /// Directory for failed downloads (when action is MoveToFailed)
    #[serde(default)]
    pub failed_directory: Option<PathBuf>,

    /// Delete sample files/folders
    #[serde(default = "default_true")]
    pub delete_samples: bool,

    /// Extraction configuration
    #[serde(default)]
    pub extraction: ExtractionConfig,

    /// File collision handling
    #[serde(default)]
    pub file_collision: FileCollisionAction,

    /// Filename deobfuscation configuration
    #[serde(default)]
    pub deobfuscation: DeobfuscationConfig,

    /// Duplicate detection configuration
    #[serde(default)]
    pub duplicate: DuplicateConfig,

    /// Disk space checking configuration
    #[serde(default)]
    pub disk_space: DiskSpaceConfig,

    /// Path to global password file (one password per line)
    #[serde(default)]
    pub password_file: Option<PathBuf>,

    /// Try empty password as fallback
    #[serde(default = "default_true")]
    pub try_empty_password: bool,

    /// Path to unrar executable (auto-detected if None)
    #[serde(default)]
    pub unrar_path: Option<PathBuf>,

    /// Path to 7z executable (auto-detected if None)
    #[serde(default)]
    pub sevenzip_path: Option<PathBuf>,

    /// Database path (default: "./usenet-dl.db")
    #[serde(default = "default_database_path")]
    pub database_path: PathBuf,

    /// REST API configuration
    #[serde(default)]
    pub api: ApiConfig,

    /// Schedule rules for time-based speed limits
    #[serde(default)]
    pub schedule_rules: Vec<ScheduleRule>,

    /// Watch folders for auto-importing NZBs
    #[serde(default)]
    pub watch_folders: Vec<WatchFolderConfig>,

    /// Webhook configurations
    #[serde(default)]
    pub webhooks: Vec<WebhookConfig>,

    /// Script configurations
    #[serde(default)]
    pub scripts: Vec<ScriptConfig>,

    /// Category configurations
    #[serde(default)]
    pub categories: HashMap<String, CategoryConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            servers: vec![],
            download_dir: default_download_dir(),
            temp_dir: default_temp_dir(),
            max_concurrent_downloads: default_max_concurrent(),
            speed_limit_bps: None,
            retry: RetryConfig::default(),
            default_post_process: PostProcess::default(),
            failed_action: FailedDownloadAction::default(),
            failed_directory: None,
            delete_samples: true,
            extraction: ExtractionConfig::default(),
            file_collision: FileCollisionAction::default(),
            deobfuscation: DeobfuscationConfig::default(),
            duplicate: DuplicateConfig::default(),
            disk_space: DiskSpaceConfig::default(),
            password_file: None,
            try_empty_password: true,
            unrar_path: None,
            sevenzip_path: None,
            database_path: default_database_path(),
            api: ApiConfig::default(),
            schedule_rules: vec![],
            watch_folders: vec![],
            webhooks: vec![],
            scripts: vec![],
            categories: HashMap::new(),
        }
    }
}

/// NNTP server configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Server hostname
    pub host: String,

    /// Server port (typically 119 for unencrypted, 563 for TLS)
    pub port: u16,

    /// Use TLS (implicit TLS, not STARTTLS)
    pub tls: bool,

    /// Username for authentication
    pub username: Option<String>,

    /// Password for authentication
    pub password: Option<String>,

    /// Number of connections to maintain
    #[serde(default = "default_connections")]
    pub connections: usize,

    /// Server priority (lower = tried first, for backup servers)
    #[serde(default)]
    pub priority: i32,
}

/// Retry configuration for transient failures
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (default: 5)
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,

    /// Initial delay before first retry (default: 1 second)
    #[serde(default = "default_initial_delay", with = "duration_serde")]
    pub initial_delay: Duration,

    /// Maximum delay between retries (default: 60 seconds)
    #[serde(default = "default_max_delay", with = "duration_serde")]
    pub max_delay: Duration,

    /// Multiplier for exponential backoff (default: 2.0)
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,

    /// Add random jitter to delays (default: true)
    #[serde(default = "default_true")]
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }
}

/// Post-processing mode
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostProcess {
    /// Just download, no post-processing
    None,
    /// Download + PAR2 verify
    Verify,
    /// Download + PAR2 verify/repair
    Repair,
    /// Above + extract archives
    Unpack,
    /// Above + remove intermediate files (default)
    UnpackAndCleanup,
}

impl Default for PostProcess {
    fn default() -> Self {
        PostProcess::UnpackAndCleanup
    }
}

/// Action to take when post-processing fails
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailedDownloadAction {
    /// Keep files in the download directory (default)
    Keep,
    /// Delete all downloaded files
    Delete,
    /// Move to a dedicated failed downloads directory
    MoveToFailed,
}

impl Default for FailedDownloadAction {
    fn default() -> Self {
        FailedDownloadAction::Keep
    }
}

/// Archive extraction configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtractionConfig {
    /// Maximum depth for nested archive extraction (default: 2)
    #[serde(default = "default_max_recursion")]
    pub max_recursion_depth: u32,

    /// File extensions to treat as archives
    #[serde(default = "default_archive_extensions")]
    pub archive_extensions: Vec<String>,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            max_recursion_depth: 2,
            archive_extensions: default_archive_extensions(),
        }
    }
}

/// File collision handling strategy
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileCollisionAction {
    /// Append (1), (2), etc. to filename (default)
    Rename,
    /// Overwrite existing file
    Overwrite,
    /// Skip the file, keep existing
    Skip,
}

impl Default for FileCollisionAction {
    fn default() -> Self {
        FileCollisionAction::Rename
    }
}

/// Obfuscated filename detection and renaming configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeobfuscationConfig {
    /// Enable automatic deobfuscation (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Minimum filename length to consider for deobfuscation (default: 12)
    #[serde(default = "default_min_length")]
    pub min_length: usize,
}

impl Default for DeobfuscationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_length: 12,
        }
    }
}

/// Duplicate detection configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuplicateConfig {
    /// Enable duplicate detection (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// What to do when duplicate detected
    #[serde(default)]
    pub action: DuplicateAction,

    /// Check methods (in order)
    #[serde(default = "default_duplicate_methods")]
    pub methods: Vec<DuplicateMethod>,
}

impl Default for DuplicateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            action: DuplicateAction::default(),
            methods: default_duplicate_methods(),
        }
    }
}

/// Action to take when duplicate detected
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateAction {
    /// Block the download entirely
    Block,
    /// Allow but emit warning event (default)
    Warn,
    /// Allow silently
    Allow,
}

impl Default for DuplicateAction {
    fn default() -> Self {
        DuplicateAction::Warn
    }
}

/// Duplicate detection method
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateMethod {
    /// NZB content hash (most reliable)
    NzbHash,
    /// NZB filename
    NzbName,
    /// Extracted job name (deobfuscated)
    JobName,
}

/// Disk space checking configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiskSpaceConfig {
    /// Enable disk space checking (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Minimum free space to maintain (default: 1 GB)
    #[serde(default = "default_min_free_space")]
    pub min_free_space: u64,

    /// Multiplier for estimated size (default: 2.5)
    #[serde(default = "default_size_multiplier")]
    pub size_multiplier: f64,
}

impl Default for DiskSpaceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_free_space: 1024 * 1024 * 1024, // 1 GB
            size_multiplier: 2.5,
        }
    }
}

/// REST API configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Address to bind to (default: 127.0.0.1:6789)
    #[serde(default = "default_bind_address")]
    pub bind_address: SocketAddr,

    /// Optional API key for authentication
    #[serde(default)]
    pub api_key: Option<String>,

    /// Enable CORS for browser access (default: true)
    #[serde(default = "default_true")]
    pub cors_enabled: bool,

    /// Allowed CORS origins (default: ["*"])
    #[serde(default = "default_cors_origins")]
    pub cors_origins: Vec<String>,

    /// Enable Swagger UI at /swagger-ui (default: true)
    #[serde(default = "default_true")]
    pub swagger_ui: bool,

    /// Rate limiting configuration
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            api_key: None,
            cors_enabled: true,
            cors_origins: default_cors_origins(),
            swagger_ui: true,
            rate_limit: RateLimitConfig::default(),
        }
    }
}

/// Rate limiting configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Enable rate limiting (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Requests per second per IP (default: 100)
    #[serde(default = "default_requests_per_second")]
    pub requests_per_second: u32,

    /// Burst size (default: 200)
    #[serde(default = "default_burst_size")]
    pub burst_size: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_second: 100,
            burst_size: 200,
        }
    }
}

/// Schedule rule for time-based actions
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScheduleRule {
    /// Human-readable name
    pub name: String,

    /// Days this rule applies (empty = all days)
    #[serde(default)]
    pub days: Vec<Weekday>,

    /// Start time (HH:MM)
    pub start_time: String,

    /// End time (HH:MM)
    pub end_time: String,

    /// Action to take during this window
    pub action: ScheduleAction,

    /// Whether rule is active
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Day of week for schedule rules
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

/// Action to take during scheduled time window
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScheduleAction {
    /// Set speed limit (bytes per second)
    SpeedLimit { limit_bps: u64 },
    /// Unlimited speed
    Unlimited,
    /// Pause all downloads
    Pause,
}

/// Watch folder configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchFolderConfig {
    /// Directory to watch for NZB files
    pub path: PathBuf,

    /// What to do with NZB after adding to queue
    #[serde(default)]
    pub after_import: WatchFolderAction,

    /// Category to assign (None = use default)
    #[serde(default)]
    pub category: Option<String>,

    /// Scan interval (default: 5 seconds)
    #[serde(default = "default_scan_interval", with = "duration_serde")]
    pub scan_interval: Duration,
}

/// Action to take with NZB file after import
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchFolderAction {
    /// Delete NZB file
    Delete,
    /// Move to a 'processed' subfolder (default)
    MoveToProcessed,
    /// Keep in place
    Keep,
}

impl Default for WatchFolderAction {
    fn default() -> Self {
        WatchFolderAction::MoveToProcessed
    }
}

/// Webhook configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// URL to POST to
    pub url: String,

    /// Events that trigger this webhook
    pub events: Vec<WebhookEvent>,

    /// Optional authentication header value
    #[serde(default)]
    pub auth_header: Option<String>,

    /// Timeout for webhook requests (default: 30 seconds)
    #[serde(default = "default_webhook_timeout", with = "duration_serde")]
    pub timeout: Duration,
}

/// Webhook trigger event
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebhookEvent {
    OnComplete,
    OnFailed,
    OnQueued,
}

/// Script execution configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptConfig {
    /// Path to script/executable
    pub path: PathBuf,

    /// Events that trigger this script
    pub events: Vec<ScriptEvent>,

    /// Timeout for script execution (default: 5 minutes)
    #[serde(default = "default_script_timeout", with = "duration_serde")]
    pub timeout: Duration,
}

/// Script trigger event
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptEvent {
    OnComplete,
    OnFailed,
    OnPostProcessComplete,
}

/// Category configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CategoryConfig {
    /// Destination directory for this category
    pub destination: PathBuf,

    /// Override default post-processing
    #[serde(default)]
    pub post_process: Option<PostProcess>,

    /// Category-specific watch folder
    #[serde(default)]
    pub watch_folder: Option<WatchFolderConfig>,

    /// Category-specific scripts
    #[serde(default)]
    pub scripts: Vec<ScriptConfig>,
}

// Default value functions
fn default_download_dir() -> PathBuf {
    PathBuf::from("downloads")
}

fn default_temp_dir() -> PathBuf {
    PathBuf::from("temp")
}

fn default_max_concurrent() -> usize {
    3
}

fn default_database_path() -> PathBuf {
    PathBuf::from("usenet-dl.db")
}

fn default_connections() -> usize {
    10
}

fn default_true() -> bool {
    true
}

fn default_max_attempts() -> u32 {
    5
}

fn default_initial_delay() -> Duration {
    Duration::from_secs(1)
}

fn default_max_delay() -> Duration {
    Duration::from_secs(60)
}

fn default_backoff_multiplier() -> f64 {
    2.0
}

fn default_max_recursion() -> u32 {
    2
}

fn default_archive_extensions() -> Vec<String> {
    vec![
        "rar".into(),
        "zip".into(),
        "7z".into(),
        "tar".into(),
        "gz".into(),
        "bz2".into(),
    ]
}

fn default_min_length() -> usize {
    12
}

fn default_duplicate_methods() -> Vec<DuplicateMethod> {
    vec![DuplicateMethod::NzbHash, DuplicateMethod::JobName]
}

fn default_min_free_space() -> u64 {
    1024 * 1024 * 1024 // 1 GB
}

fn default_size_multiplier() -> f64 {
    2.5
}

fn default_bind_address() -> SocketAddr {
    "127.0.0.1:6789".parse().unwrap()
}

fn default_cors_origins() -> Vec<String> {
    vec!["*".into()]
}

fn default_requests_per_second() -> u32 {
    100
}

fn default_burst_size() -> u32 {
    200
}

fn default_scan_interval() -> Duration {
    Duration::from_secs(5)
}

fn default_webhook_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_script_timeout() -> Duration {
    Duration::from_secs(300) // 5 minutes
}

// Duration serialization helper
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

// Conversion from our ServerConfig to nntp-rs's ServerConfig
impl From<ServerConfig> for nntp_rs::ServerConfig {
    fn from(config: ServerConfig) -> Self {
        nntp_rs::ServerConfig {
            host: config.host,
            port: config.port,
            tls: config.tls,
            allow_insecure_tls: false,
            username: config.username.unwrap_or_default(),
            password: config.password.unwrap_or_default(),
        }
    }
}
