# Trait-Based Plugin Architecture for External Tools

## Overview

This document outlines a modular architecture for handling external tool dependencies (PAR2, UnRAR) in usenet-dl. The design allows the library to function standalone with basic capabilities while enabling enhanced functionality when external binaries are available.

## Goals

1. **Zero required external dependencies** - Library works out of the box
2. **Graceful enhancement** - Better functionality when binaries are available
3. **Tauri sidecar compatibility** - Easy integration with bundled binaries
4. **Clear capability discovery** - Consumers can query what's available
5. **Consistent API** - Same interface regardless of backend

## Architecture

### Core Traits

#### ParityHandler

Handles PAR2 verification and repair operations.

```rust
use std::path::Path;
use async_trait::async_trait;

/// Result of PAR2 verification
#[derive(Debug, Clone)]
pub struct VerifyResult {
    /// Whether all files are intact
    pub is_complete: bool,
    /// Number of damaged/missing blocks
    pub damaged_blocks: u32,
    /// Number of recovery blocks available
    pub recovery_blocks_available: u32,
    /// Whether repair is possible with available recovery data
    pub repairable: bool,
    /// List of damaged files
    pub damaged_files: Vec<String>,
    /// List of missing files
    pub missing_files: Vec<String>,
}

/// Result of PAR2 repair
#[derive(Debug, Clone)]
pub struct RepairResult {
    /// Whether repair was successful
    pub success: bool,
    /// Files that were repaired
    pub repaired_files: Vec<String>,
    /// Files that could not be repaired
    pub failed_files: Vec<String>,
    /// Error message if repair failed
    pub error: Option<String>,
}

/// Capabilities of a parity handler implementation
#[derive(Debug, Clone, Copy)]
pub struct ParityCapabilities {
    /// Can verify file integrity
    pub can_verify: bool,
    /// Can repair damaged files
    pub can_repair: bool,
}

/// Trait for PAR2 parity handling
#[async_trait]
pub trait ParityHandler: Send + Sync {
    /// Verify integrity of files using PAR2
    async fn verify(&self, par2_file: &Path) -> crate::Result<VerifyResult>;
    
    /// Attempt to repair damaged files using PAR2 recovery data
    async fn repair(&self, par2_file: &Path) -> crate::Result<RepairResult>;
    
    /// Query capabilities of this handler
    fn capabilities(&self) -> ParityCapabilities;
    
    /// Human-readable name for logging
    fn name(&self) -> &'static str;
}
```

#### ArchiveExtractor

Handles archive extraction for various formats.

```rust
use std::path::Path;
use async_trait::async_trait;

/// Result of archive extraction
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    /// Whether extraction was successful
    pub success: bool,
    /// Files that were extracted
    pub extracted_files: Vec<String>,
    /// Total bytes extracted
    pub bytes_extracted: u64,
    /// Whether a password was required
    pub password_protected: bool,
    /// Error message if extraction failed
    pub error: Option<String>,
}

/// Capabilities of an archive extractor
#[derive(Debug, Clone)]
pub struct ExtractorCapabilities {
    /// Supported file extensions (lowercase, without dot)
    pub supported_formats: Vec<&'static str>,
    /// Whether password-protected archives are supported
    pub supports_passwords: bool,
}

/// Trait for archive extraction
#[async_trait]
pub trait ArchiveExtractor: Send + Sync {
    /// Extract archive to destination directory
    async fn extract(
        &self,
        archive: &Path,
        destination: &Path,
        password: Option<&str>,
    ) -> crate::Result<ExtractionResult>;
    
    /// Check if this extractor can handle the given file
    fn can_handle(&self, path: &Path) -> bool;
    
    /// Query capabilities of this extractor
    fn capabilities(&self) -> ExtractorCapabilities;
    
    /// Human-readable name for logging
    fn name(&self) -> &'static str;
}
```

### Built-in Implementations

#### BuiltinParityHandler

Pure Rust implementation with verification only.

```rust
/// Built-in PAR2 handler using pure Rust
/// 
/// Capabilities:
/// - ✅ Verification (parse PAR2, verify checksums)
/// - ❌ Repair (requires Reed-Solomon implementation)
pub struct BuiltinParityHandler;

#[async_trait]
impl ParityHandler for BuiltinParityHandler {
    async fn verify(&self, par2_file: &Path) -> crate::Result<VerifyResult> {
        // 1. Parse PAR2 file format (packet-based structure)
        // 2. Extract file descriptions and checksums
        // 3. Verify each file's MD5 and CRC32
        // 4. Count damaged/missing blocks
        // 5. Calculate if repair would be possible
        todo!("Implement PAR2 parsing and verification")
    }
    
    async fn repair(&self, _par2_file: &Path) -> crate::Result<RepairResult> {
        Err(crate::Error::NotSupported(
            "PAR2 repair requires external par2 binary. \
             Set `par2_binary` in config to enable repair.".into()
        ))
    }
    
    fn capabilities(&self) -> ParityCapabilities {
        ParityCapabilities {
            can_verify: true,
            can_repair: false,
        }
    }
    
    fn name(&self) -> &'static str {
        "builtin-parity"
    }
}
```

#### BuiltinArchiveExtractor

Pure Rust implementation supporting ZIP and 7z.

```rust
/// Built-in archive extractor using pure Rust crates
/// 
/// Supported formats:
/// - ✅ ZIP (via `zip` crate)
/// - ✅ 7z (via `sevenz-rust` crate)
/// - ❌ RAR (requires external binary)
pub struct BuiltinArchiveExtractor;

#[async_trait]
impl ArchiveExtractor for BuiltinArchiveExtractor {
    async fn extract(
        &self,
        archive: &Path,
        destination: &Path,
        password: Option<&str>,
    ) -> crate::Result<ExtractionResult> {
        let ext = archive.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());
        
        match ext.as_deref() {
            Some("zip") => self.extract_zip(archive, destination, password).await,
            Some("7z") => self.extract_7z(archive, destination, password).await,
            _ => Err(crate::Error::UnsupportedFormat(
                format!("Built-in extractor doesn't support {:?}", ext)
            )),
        }
    }
    
    fn can_handle(&self, path: &Path) -> bool {
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());
        matches!(ext.as_deref(), Some("zip" | "7z"))
    }
    
    fn capabilities(&self) -> ExtractorCapabilities {
        ExtractorCapabilities {
            supported_formats: vec!["zip", "7z"],
            supports_passwords: true,
        }
    }
    
    fn name(&self) -> &'static str {
        "builtin-extractor"
    }
}
```

### CLI-based Implementations

#### CliParityHandler

External binary implementation with full PAR2 support.

```rust
use std::path::PathBuf;
use tokio::process::Command;

/// CLI-based PAR2 handler using external par2 binary
/// 
/// Capabilities:
/// - ✅ Verification
/// - ✅ Repair
pub struct CliParityHandler {
    binary_path: PathBuf,
}

impl CliParityHandler {
    pub fn new(binary_path: PathBuf) -> Self {
        Self { binary_path }
    }
    
    /// Attempt to find par2 in PATH
    pub fn from_path() -> Option<Self> {
        which::which("par2").ok().map(|p| Self::new(p))
    }
}

#[async_trait]
impl ParityHandler for CliParityHandler {
    async fn verify(&self, par2_file: &Path) -> crate::Result<VerifyResult> {
        let output = Command::new(&self.binary_path)
            .arg("v")
            .arg(par2_file)
            .output()
            .await
            .map_err(|e| crate::Error::ExternalTool(format!("par2: {}", e)))?;
        
        // Parse par2 output to extract verification results
        parse_par2_verify_output(&output.stdout, &output.stderr, output.status.success())
    }
    
    async fn repair(&self, par2_file: &Path) -> crate::Result<RepairResult> {
        let output = Command::new(&self.binary_path)
            .arg("r")
            .arg(par2_file)
            .output()
            .await
            .map_err(|e| crate::Error::ExternalTool(format!("par2: {}", e)))?;
        
        // Parse par2 output to extract repair results
        parse_par2_repair_output(&output.stdout, &output.stderr, output.status.success())
    }
    
    fn capabilities(&self) -> ParityCapabilities {
        ParityCapabilities {
            can_verify: true,
            can_repair: true,
        }
    }
    
    fn name(&self) -> &'static str {
        "cli-par2"
    }
}
```

#### CliRarExtractor

External binary implementation for RAR files.

```rust
/// CLI-based RAR extractor using external unrar binary
/// 
/// Supported formats:
/// - ✅ RAR (all versions including RAR5)
pub struct CliRarExtractor {
    binary_path: PathBuf,
}

impl CliRarExtractor {
    pub fn new(binary_path: PathBuf) -> Self {
        Self { binary_path }
    }
    
    /// Attempt to find unrar in PATH
    pub fn from_path() -> Option<Self> {
        which::which("unrar").ok().map(|p| Self::new(p))
    }
}

#[async_trait]
impl ArchiveExtractor for CliRarExtractor {
    async fn extract(
        &self,
        archive: &Path,
        destination: &Path,
        password: Option<&str>,
    ) -> crate::Result<ExtractionResult> {
        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("x")           // Extract with full paths
           .arg("-o+")         // Overwrite existing files
           .arg("-y");         // Assume yes on all queries
        
        if let Some(pw) = password {
            cmd.arg(format!("-p{}", pw));
        } else {
            cmd.arg("-p-");    // Don't prompt for password
        }
        
        cmd.arg(archive)
           .arg(destination);
        
        let output = cmd.output().await
            .map_err(|e| crate::Error::ExternalTool(format!("unrar: {}", e)))?;
        
        parse_unrar_output(&output.stdout, &output.stderr, output.status.success())
    }
    
    fn can_handle(&self, path: &Path) -> bool {
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());
        matches!(ext.as_deref(), Some("rar"))
    }
    
    fn capabilities(&self) -> ExtractorCapabilities {
        ExtractorCapabilities {
            supported_formats: vec!["rar"],
            supports_passwords: true,
        }
    }
    
    fn name(&self) -> &'static str {
        "cli-unrar"
    }
}
```

### Composite Extractor

Combines multiple extractors into one.

```rust
/// Combines multiple extractors, delegating to the appropriate one
pub struct CompositeExtractor {
    extractors: Vec<Arc<dyn ArchiveExtractor>>,
}

impl CompositeExtractor {
    pub fn new(extractors: Vec<Arc<dyn ArchiveExtractor>>) -> Self {
        Self { extractors }
    }
    
    /// Create with default configuration based on available binaries
    pub fn with_config(config: &Config) -> Self {
        let mut extractors: Vec<Arc<dyn ArchiveExtractor>> = vec![
            Arc::new(BuiltinArchiveExtractor),
        ];
        
        // Add RAR support if binary is configured
        if let Some(ref unrar_path) = config.unrar_binary {
            extractors.push(Arc::new(CliRarExtractor::new(unrar_path.clone())));
        } else if let Some(extractor) = CliRarExtractor::from_path() {
            // Fall back to PATH lookup
            extractors.push(Arc::new(extractor));
        }
        
        Self::new(extractors)
    }
}

#[async_trait]
impl ArchiveExtractor for CompositeExtractor {
    async fn extract(
        &self,
        archive: &Path,
        destination: &Path,
        password: Option<&str>,
    ) -> crate::Result<ExtractionResult> {
        for extractor in &self.extractors {
            if extractor.can_handle(archive) {
                tracing::debug!(
                    "Using {} for {:?}",
                    extractor.name(),
                    archive.file_name()
                );
                return extractor.extract(archive, destination, password).await;
            }
        }
        
        Err(crate::Error::UnsupportedFormat(format!(
            "No extractor available for {:?}",
            archive.extension()
        )))
    }
    
    fn can_handle(&self, path: &Path) -> bool {
        self.extractors.iter().any(|e| e.can_handle(path))
    }
    
    fn capabilities(&self) -> ExtractorCapabilities {
        let formats: Vec<&'static str> = self.extractors
            .iter()
            .flat_map(|e| e.capabilities().supported_formats)
            .collect();
        
        ExtractorCapabilities {
            supported_formats: formats,
            supports_passwords: self.extractors
                .iter()
                .any(|e| e.capabilities().supports_passwords),
        }
    }
    
    fn name(&self) -> &'static str {
        "composite-extractor"
    }
}
```

## Configuration Changes

```rust
/// External tool configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalToolsConfig {
    /// Path to par2 binary. If None, only verification is available.
    /// 
    /// In Tauri apps, set this to the sidecar path:
    /// ```ignore
    /// app.path_resolver().resolve_resource("binaries/par2")
    /// ```
    #[serde(default)]
    pub par2_binary: Option<PathBuf>,
    
    /// Path to unrar binary. If None, RAR extraction is disabled.
    /// 
    /// In Tauri apps, set this to the sidecar path:
    /// ```ignore
    /// app.path_resolver().resolve_resource("binaries/unrar")
    /// ```
    #[serde(default)]
    pub unrar_binary: Option<PathBuf>,
    
    /// Whether to search PATH for binaries if paths are not explicitly set.
    /// Default: true
    #[serde(default = "default_true")]
    pub search_path: bool,
}

// In main Config struct:
pub struct Config {
    // ... existing fields ...
    
    /// External tool configuration
    #[serde(default)]
    pub external_tools: ExternalToolsConfig,
}
```

## Integration with UsenetDownloader

```rust
impl UsenetDownloader {
    pub async fn new(config: Config) -> Result<Self> {
        // Initialize parity handler based on config
        let parity_handler: Arc<dyn ParityHandler> = 
            if let Some(ref par2_path) = config.external_tools.par2_binary {
                Arc::new(CliParityHandler::new(par2_path.clone()))
            } else if config.external_tools.search_path {
                CliParityHandler::from_path()
                    .map(|h| Arc::new(h) as Arc<dyn ParityHandler>)
                    .unwrap_or_else(|| Arc::new(BuiltinParityHandler))
            } else {
                Arc::new(BuiltinParityHandler)
            };
        
        // Initialize archive extractor based on config
        let archive_extractor = Arc::new(
            CompositeExtractor::with_config(&config)
        );
        
        // Log capabilities
        let parity_caps = parity_handler.capabilities();
        let extractor_caps = archive_extractor.capabilities();
        
        tracing::info!(
            parity_handler = parity_handler.name(),
            can_verify = parity_caps.can_verify,
            can_repair = parity_caps.can_repair,
            "Parity handler initialized"
        );
        
        tracing::info!(
            extractor = archive_extractor.name(),
            formats = ?extractor_caps.supported_formats,
            "Archive extractor initialized"
        );
        
        // ... rest of initialization
    }
    
    /// Query current capabilities
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            parity: self.parity_handler.capabilities(),
            extraction: self.archive_extractor.capabilities(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub parity: ParityCapabilities,
    pub extraction: ExtractorCapabilities,
}
```

## Tauri Integration Example

```rust
// src-tauri/src/main.rs

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let par2_path = app.path_resolver()
                .resolve_resource("binaries/par2")
                .expect("par2 binary not found");
            
            let unrar_path = app.path_resolver()
                .resolve_resource("binaries/unrar")
                .expect("unrar binary not found");
            
            let config = usenet_dl::Config {
                external_tools: usenet_dl::ExternalToolsConfig {
                    par2_binary: Some(par2_path),
                    unrar_binary: Some(unrar_path),
                    search_path: false, // Don't search PATH, use bundled only
                },
                // ... other config
                ..Default::default()
            };
            
            let downloader = usenet_dl::UsenetDownloader::new(config).await?;
            
            // Log capabilities
            let caps = downloader.capabilities();
            println!("PAR2 repair available: {}", caps.parity.can_repair);
            println!("RAR extraction available: {}", 
                caps.extraction.supported_formats.contains(&"rar"));
            
            app.manage(downloader);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running app");
}
```

**tauri.conf.json:**
```json
{
  "tauri": {
    "bundle": {
      "externalBin": [
        "binaries/par2",
        "binaries/unrar"
      ]
    }
  }
}
```

**Binary structure:**
```
src-tauri/
└── binaries/
    ├── par2-x86_64-unknown-linux-gnu
    ├── par2-x86_64-pc-windows-msvc.exe
    ├── par2-aarch64-apple-darwin
    ├── par2-x86_64-apple-darwin
    ├── unrar-x86_64-unknown-linux-gnu
    ├── unrar-x86_64-pc-windows-msvc.exe
    ├── unrar-aarch64-apple-darwin
    └── unrar-x86_64-apple-darwin
```

## API Additions

### REST API Endpoint

```
GET /api/v1/capabilities
```

**Response:**
```json
{
  "parity": {
    "can_verify": true,
    "can_repair": true,
    "handler": "cli-par2"
  },
  "extraction": {
    "supported_formats": ["zip", "7z", "rar"],
    "supports_passwords": true,
    "handler": "composite-extractor"
  }
}
```

### Events

```rust
pub enum Event {
    // ... existing events ...
    
    /// Emitted when repair is skipped due to missing capability
    RepairSkipped {
        id: DownloadId,
        reason: String,
    },
    
    /// Emitted when extraction is skipped due to unsupported format
    ExtractionSkipped {
        id: DownloadId,
        archive: String,
        reason: String,
    },
}
```

## Migration Path

### Phase 1: Trait Infrastructure
1. Define `ParityHandler` and `ArchiveExtractor` traits
2. Implement `BuiltinParityHandler` (verification only stub)
3. Implement `BuiltinArchiveExtractor` (existing zip/7z code)
4. Add `ExternalToolsConfig` to `Config`

### Phase 2: CLI Implementations
1. Implement `CliParityHandler`
2. Implement `CliRarExtractor`
3. Implement `CompositeExtractor`
4. Update `UsenetDownloader` to use traits

### Phase 3: Built-in PAR2 Verification
1. Implement PAR2 file format parser
2. Implement checksum verification (MD5, CRC32)
3. Full `BuiltinParityHandler` implementation

### Phase 4: Polish
1. Add `/capabilities` API endpoint
2. Add capability-related events
3. Update documentation
4. Add integration tests

## Testing Strategy

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_builtin_extractor_zip() {
        let extractor = BuiltinArchiveExtractor;
        assert!(extractor.can_handle(Path::new("test.zip")));
        assert!(!extractor.can_handle(Path::new("test.rar")));
    }
    
    #[tokio::test]
    async fn test_builtin_parity_returns_not_supported_for_repair() {
        let handler = BuiltinParityHandler;
        let result = handler.repair(Path::new("test.par2")).await;
        assert!(matches!(result, Err(crate::Error::NotSupported(_))));
    }
    
    #[tokio::test]
    async fn test_composite_extractor_delegates_correctly() {
        let composite = CompositeExtractor::new(vec![
            Arc::new(BuiltinArchiveExtractor),
        ]);
        
        assert!(composite.can_handle(Path::new("test.zip")));
        assert!(composite.can_handle(Path::new("test.7z")));
        assert!(!composite.can_handle(Path::new("test.rar")));
    }
    
    #[tokio::test]
    #[ignore] // Requires par2 binary
    async fn test_cli_parity_handler() {
        let handler = CliParityHandler::from_path()
            .expect("par2 not found in PATH");
        
        // Create test files and par2...
        let caps = handler.capabilities();
        assert!(caps.can_verify);
        assert!(caps.can_repair);
    }
}
```

## Summary

This architecture provides:

| Feature | Standalone | With Binaries |
|---------|------------|---------------|
| ZIP extraction | ✅ Pure Rust | ✅ Pure Rust |
| 7z extraction | ✅ Pure Rust | ✅ Pure Rust |
| RAR extraction | ❌ | ✅ CLI |
| PAR2 verify | ✅ Pure Rust | ✅ CLI |
| PAR2 repair | ❌ | ✅ CLI |

The library remains fully functional for the most common cases (ZIP, 7z) without any external dependencies, while allowing enhanced functionality when binaries are available (RAR, PAR2 repair).

This is particularly valuable for:
- **Tauri apps**: Bundle binaries as sidecars for "just works" experience
- **Server deployments**: Install binaries via package manager
- **Minimal installs**: Use built-in only when dependencies are problematic
- **Testing**: Mock implementations for unit tests
