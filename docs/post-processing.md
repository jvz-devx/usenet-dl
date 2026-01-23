# Post-Processing

This document describes the post-processing pipeline that handles verification, repair, extraction, and cleanup of downloaded files.

## Overview

The post-processing pipeline is a five-stage sequential process that automatically processes downloaded files:

1. **Verify** - PAR2 verification of downloaded files
2. **Repair** - PAR2 repair (if verification fails)
3. **Extract** - Archive extraction (RAR, 7z, ZIP)
4. **Move** - Move files to final destination
5. **Cleanup** - Remove intermediate files

Each stage is optional and can be configured based on your needs.

## Post-Processing Modes

Configure which stages execute using the `PostProcess` enum:

```rust
pub enum PostProcess {
    None,                // Skip all post-processing
    Verify,              // Only verify
    Repair,              // Verify + repair
    Unpack,              // Verify + repair + extract
    UnpackAndCleanup,    // Full pipeline (default)
}
```

Example configuration:

```rust
use usenet_dl::{UsenetDownloader, Config, PostProcess};

let config = Config {
    post_process: PostProcess::UnpackAndCleanup,
    ..Default::default()
};

let downloader = UsenetDownloader::new(config).await?;
```

## Archive Extraction

### Supported Formats

- **RAR** (`.rar`, `.r00`, `.r01`, etc.) - Via `unrar` crate
- **7-Zip** (`.7z`) - Via `sevenz_rust` crate
- **ZIP** (`.zip`) - Via `zip` crate

### Password Handling

The extraction system tries passwords in priority order:

1. **Cached password** - Previously successful password for this download
2. **Per-download password** - User-specified password for specific download
3. **NZB metadata password** - Password embedded in NZB file
4. **Global password file** - Passwords from configured file (one per line)
5. **Empty password** - Try no password (optional fallback)

Example with passwords:

```rust
use usenet_dl::{UsenetDownloader, Config};

let config = Config {
    password_file: Some("/path/to/passwords.txt".into()),
    try_empty_password: true,
    ..Default::default()
};

let downloader = UsenetDownloader::new(config).await?;

// Add download with specific password
let id = downloader.add_nzb(
    nzb_data,
    Some("secret_password".to_string())
).await?;
```

Password file format (one per line):

```
password1
password2
secret_password
```

### Nested Archives

The extraction system automatically handles nested archives (archives within archives):

```rust
use usenet_dl::Config;

let config = Config {
    max_recursion_depth: 2,  // Default: 2 levels
    ..Default::default()
};
```

For each nested archive:
- Creates unique subdirectory
- Extracts recursively up to `max_recursion_depth`
- Logs failures but continues processing

### Configuration Options

```rust
pub struct ExtractionConfig {
    pub max_recursion_depth: u32,      // Default: 2
    pub archive_extensions: Vec<String>, // RAR, 7Z, ZIP, etc.
}
```

## File Moving

After extraction, files are moved to the final destination directory.

### Collision Handling

When a file already exists at the destination, the system uses the configured action:

```rust
pub enum FileCollisionAction {
    Rename,      // Append (1), (2), etc. to filename (default)
    Overwrite,   // Replace existing file
    Skip,        // Keep existing, fail the move
}
```

Example:

```rust
use usenet_dl::{Config, FileCollisionAction};

let config = Config {
    file_collision_action: FileCollisionAction::Rename,
    ..Default::default()
};
```

With `Rename` action, files are renamed automatically:
```
movie.mkv       → movie.mkv (original)
movie.mkv       → movie (1).mkv (first collision)
movie.mkv       → movie (2).mkv (second collision)
```

## Cleanup

The cleanup stage removes intermediate files after successful extraction.

### Configuration

```rust
pub struct CleanupConfig {
    pub enabled: bool,  // Default: true

    // Extensions to remove
    pub target_extensions: Vec<String>,
    // Default: ["par2", "nzb", "sfv", "srr", "nfo"]

    // Archive extensions to remove after extraction
    pub archive_extensions: Vec<String>,
    // Default: ["rar", "7z", "zip", "r00", "r01", etc.]

    // Delete sample folders
    pub delete_samples: bool,  // Default: true

    // Sample folder names to delete
    pub sample_folder_names: Vec<String>,
    // Default: ["sample", "samples", "covers", "proof"]
}
```

Example:

```rust
use usenet_dl::{Config, CleanupConfig};

let config = Config {
    cleanup: CleanupConfig {
        enabled: true,
        target_extensions: vec![
            "par2".to_string(),
            "nzb".to_string(),
            "sfv".to_string(),
        ],
        delete_samples: true,
        ..Default::default()
    },
    ..Default::default()
};
```

### Cleanup Behavior

The cleanup process:
- Recursively walks the download directory
- Removes all files matching `target_extensions` (case-insensitive)
- Removes archive files after successful extraction
- Optionally deletes sample folders (matched by name)
- Logs failures as warnings (non-fatal)

## Deobfuscation

The deobfuscation system intelligently determines the final filename for extracted content.

### Obfuscation Detection

Files are considered obfuscated if they match any of these patterns:

1. **High entropy** - Random alphanumeric characters (24+ chars)
2. **UUID-like patterns** - `550e8400-e29b-41d4-a716-446655440000`
3. **Pure hex strings** - Long hexadecimal strings (16+ chars)
4. **No vowels** - 8+ consonant-only characters

Examples:
```
x9K2mP7vQ3nL4wR8tY5jH1sD6fG0cB3z.mkv  ✗ Obfuscated (high entropy)
550e8400-e29b-41d4-a716-446655440000.mp4  ✗ Obfuscated (UUID)
Movie.Name.2024.1080p.BluRay.x264.mkv     ✓ Not obfuscated
Episode.S01E01.720p.WEB-DL.mkv            ✓ Not obfuscated
```

### Final Name Determination

The system chooses the final filename in priority order:

1. **Job name** (NZB filename) if not obfuscated
2. **NZB meta title** if present and not obfuscated
3. **Largest non-obfuscated file** from extracted files
4. **Job name** as fallback (even if obfuscated)

This approach follows SABnzbd's naming strategy to provide sensible filenames.

### Configuration

```rust
pub struct DeobfuscationConfig {
    pub enabled: bool,      // Default: true
    pub min_length: usize,  // Default: 12 (minimum length to check)
}
```

Example:

```rust
use usenet_dl::{Config, DeobfuscationConfig};

let config = Config {
    deobfuscation: DeobfuscationConfig {
        enabled: true,
        min_length: 12,
    },
    ..Default::default()
};
```

## Events

Subscribe to post-processing events to track progress:

```rust
use usenet_dl::{UsenetDownloader, Event};

let downloader = UsenetDownloader::new(config).await?;
let mut rx = downloader.subscribe();

tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        match event {
            Event::Verifying { download_id } => {
                println!("Verifying files for {}", download_id);
            }
            Event::VerifyComplete { download_id, damaged } => {
                println!("Verification complete, damaged: {}", damaged);
            }
            Event::Repairing { download_id } => {
                println!("Repairing files for {}", download_id);
            }
            Event::RepairComplete { download_id, success } => {
                println!("Repair complete, success: {}", success);
            }
            Event::Extracting { download_id, archive, percent } => {
                println!("Extracting {}: {}%", archive, percent);
            }
            Event::ExtractComplete { download_id } => {
                println!("Extraction complete");
            }
            Event::Moving { download_id, destination } => {
                println!("Moving files to {:?}", destination);
            }
            Event::Cleaning { download_id } => {
                println!("Cleaning up intermediate files");
            }
            Event::Complete { download_id, final_path } => {
                println!("Download complete: {:?}", final_path);
            }
            Event::Failed { download_id, stage, error, files_kept } => {
                eprintln!("Failed at {:?}: {}", stage, error);
            }
            _ => {}
        }
    }
});
```

## Error Handling

Post-processing errors are reported via the `PostProcessError` type:

- `WrongPassword` - Archive password incorrect
- `NoPasswordsAvailable` - No passwords to try
- `AllPasswordsFailed` - All passwords tried, none worked
- `ExtractionFailed` - Archive corrupt, I/O error, etc.
- `InvalidPath` - Source/dest path issues

When an error occurs:
- A `Failed` event is emitted with the error details
- The download status is set to `Failed`
- Files may be kept or cleaned up depending on configuration

## Re-extraction

If extraction fails due to a wrong password, you can retry with a new password:

```rust
// Add new password and retry extraction
downloader.reextract(download_id, Some("new_password".to_string())).await?;
```

This skips the verify and repair stages and goes directly to extraction with the new password.

## Complete Example

```rust
use usenet_dl::{
    UsenetDownloader, Config, PostProcess, FileCollisionAction,
    CleanupConfig, DeobfuscationConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config {
        // Post-processing mode
        post_process: PostProcess::UnpackAndCleanup,

        // Password handling
        password_file: Some("passwords.txt".into()),
        try_empty_password: true,

        // Archive extraction
        max_recursion_depth: 2,

        // File collision handling
        file_collision_action: FileCollisionAction::Rename,

        // Cleanup configuration
        cleanup: CleanupConfig {
            enabled: true,
            target_extensions: vec![
                "par2".to_string(),
                "nzb".to_string(),
                "sfv".to_string(),
            ],
            delete_samples: true,
            ..Default::default()
        },

        // Deobfuscation
        deobfuscation: DeobfuscationConfig {
            enabled: true,
            min_length: 12,
        },

        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await?;

    // Subscribe to events
    let mut rx = downloader.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            println!("{:?}", event);
        }
    });

    // Add download
    let nzb_data = std::fs::read("file.nzb")?;
    let download_id = downloader.add_nzb(nzb_data, None).await?;

    println!("Started download: {}", download_id);

    Ok(())
}
```

## See Also

- [Configuration Reference](configuration.md) - All configuration options
- [Getting Started](getting-started.md) - Basic usage guide
- [Architecture](architecture.md) - System design overview
