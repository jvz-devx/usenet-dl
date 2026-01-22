//! Utility functions for file operations and path manipulation

use crate::config::FileCollisionAction;
use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

/// Get a unique path for a file, handling collisions according to the specified action
///
/// # Arguments
///
/// * `path` - The desired file path
/// * `action` - How to handle file collisions
///
/// # Returns
///
/// Returns the final path to use. For Rename action, this may have a suffix added.
/// For Skip action, returns an error if the file already exists.
/// For Overwrite action, returns the original path unchanged.
///
/// # Examples
///
/// ```
/// use usenet_dl::utils::get_unique_path;
/// use usenet_dl::config::FileCollisionAction;
/// use std::path::Path;
///
/// let path = Path::new("/tmp/movie.mkv");
/// let unique = get_unique_path(path, FileCollisionAction::Rename).unwrap();
/// // If /tmp/movie.mkv exists, returns /tmp/movie (1).mkv
/// // If that exists too, returns /tmp/movie (2).mkv, etc.
/// ```
pub fn get_unique_path(path: &Path, action: FileCollisionAction) -> Result<PathBuf> {
    match action {
        FileCollisionAction::Overwrite => {
            // Always use the original path, overwriting if it exists
            Ok(path.to_path_buf())
        }
        FileCollisionAction::Skip => {
            // Return error if file exists
            if path.exists() {
                return Err(Error::FileCollision {
                    path: path.to_path_buf(),
                    reason: "File already exists and collision action is Skip".to_string(),
                });
            }
            Ok(path.to_path_buf())
        }
        FileCollisionAction::Rename => {
            // If file doesn't exist, use original path
            if !path.exists() {
                return Ok(path.to_path_buf());
            }

            // File exists, need to find a unique name
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| Error::InvalidPath {
                    path: path.to_path_buf(),
                    reason: "Cannot extract file stem".to_string(),
                })?;

            let extension = path.extension().and_then(|e| e.to_str());

            let parent = path.parent().ok_or_else(|| Error::InvalidPath {
                path: path.to_path_buf(),
                reason: "Cannot extract parent directory".to_string(),
            })?;

            // Try adding (1), (2), (3), ... until we find a unique name
            for i in 1..=9999 {
                let new_name = match extension {
                    Some(ext) => format!("{} ({}).{}", stem, i, ext),
                    None => format!("{} ({})", stem, i),
                };
                let new_path = parent.join(new_name);
                if !new_path.exists() {
                    return Ok(new_path);
                }
            }

            // If we've tried 9999 names and they all exist, give up
            Err(Error::FileCollision {
                path: path.to_path_buf(),
                reason: "Could not find unique filename after 9999 attempts".to_string(),
            })
        }
    }
}

/// Check if a path appears to be a sample file or folder
///
/// Detects sample files/folders using common naming patterns:
/// - Folders named "sample", "samples", "subs", "proof"
/// - Files with "sample" in the name
/// - Common video sample patterns (e.g., "sample.mkv", "moviename-sample.avi")
///
/// # Arguments
///
/// * `path` - The path to check
///
/// # Returns
///
/// Returns `true` if the path appears to be a sample file or folder
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use usenet_dl::utils::is_sample;
///
/// assert!(is_sample(Path::new("/downloads/Movie/Sample")));
/// assert!(is_sample(Path::new("/downloads/Movie/movie-sample.mkv")));
/// assert!(!is_sample(Path::new("/downloads/Movie/movie.mkv")));
/// ```
pub fn is_sample(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Common sample folder/file names
    const SAMPLE_PATTERNS: &[&str] = &[
        "sample",
        "samples",
        "subs",
        "proof",
        "proofs",
        "cover",
        "covers",
        "eac3to",
    ];

    // Check for exact matches (case-insensitive)
    if SAMPLE_PATTERNS.iter().any(|&pattern| name == pattern) {
        return true;
    }

    // Check for "sample" in the filename
    // e.g., "movie-sample.mkv", "sample.avi", "movie.sample.mp4"
    if name.contains("sample") {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_get_unique_path_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        // File doesn't exist, should return original path for all actions
        assert_eq!(
            get_unique_path(&path, FileCollisionAction::Rename).unwrap(),
            path
        );
        assert_eq!(
            get_unique_path(&path, FileCollisionAction::Overwrite).unwrap(),
            path
        );
        assert_eq!(
            get_unique_path(&path, FileCollisionAction::Skip).unwrap(),
            path
        );
    }

    #[test]
    fn test_get_unique_path_rename_with_extension() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        // Create the original file
        fs::write(&path, "original").unwrap();

        // Rename action should add (1) suffix
        let unique = get_unique_path(&path, FileCollisionAction::Rename).unwrap();
        assert_eq!(unique, temp_dir.path().join("test (1).txt"));

        // Create the (1) file and try again
        fs::write(&unique, "first rename").unwrap();
        let unique2 = get_unique_path(&path, FileCollisionAction::Rename).unwrap();
        assert_eq!(unique2, temp_dir.path().join("test (2).txt"));
    }

    #[test]
    fn test_get_unique_path_rename_without_extension() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test");

        // Create the original file
        fs::write(&path, "original").unwrap();

        // Rename action should add (1) suffix
        let unique = get_unique_path(&path, FileCollisionAction::Rename).unwrap();
        assert_eq!(unique, temp_dir.path().join("test (1)"));
    }

    #[test]
    fn test_get_unique_path_overwrite() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        // Create the original file
        fs::write(&path, "original").unwrap();

        // Overwrite action should return original path
        let result = get_unique_path(&path, FileCollisionAction::Overwrite).unwrap();
        assert_eq!(result, path);
    }

    #[test]
    fn test_get_unique_path_skip_existing() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        // Create the original file
        fs::write(&path, "original").unwrap();

        // Skip action should return error if file exists
        let result = get_unique_path(&path, FileCollisionAction::Skip);
        assert!(result.is_err());
        match result {
            Err(Error::FileCollision { path: p, reason: _ }) => {
                assert_eq!(p, path);
            }
            _ => panic!("Expected FileCollision error"),
        }
    }

    #[test]
    fn test_get_unique_path_multiple_dots() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.tar.gz");

        // Create the original file
        fs::write(&path, "original").unwrap();

        // Should handle multiple dots correctly (only last extension)
        let unique = get_unique_path(&path, FileCollisionAction::Rename).unwrap();
        assert_eq!(unique, temp_dir.path().join("test.tar (1).gz"));
    }

    #[test]
    fn test_get_unique_path_sequential() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        // Create files test.txt, test (1).txt, test (2).txt
        fs::write(&path, "original").unwrap();
        fs::write(temp_dir.path().join("test (1).txt"), "first").unwrap();
        fs::write(temp_dir.path().join("test (2).txt"), "second").unwrap();

        // Should find test (3).txt
        let unique = get_unique_path(&path, FileCollisionAction::Rename).unwrap();
        assert_eq!(unique, temp_dir.path().join("test (3).txt"));
    }

    #[test]
    fn test_is_sample_folder_exact_match() {
        // Exact match sample folder names (case-insensitive)
        assert!(is_sample(Path::new("/downloads/Movie/Sample")));
        assert!(is_sample(Path::new("/downloads/Movie/sample")));
        assert!(is_sample(Path::new("/downloads/Movie/SAMPLE")));
        assert!(is_sample(Path::new("/downloads/Movie/Samples")));
        assert!(is_sample(Path::new("/downloads/Movie/Subs")));
        assert!(is_sample(Path::new("/downloads/Movie/Proof")));
        assert!(is_sample(Path::new("/downloads/Movie/Cover")));
    }

    #[test]
    fn test_is_sample_file_with_sample_in_name() {
        // Files with "sample" in the name
        assert!(is_sample(Path::new("/downloads/movie-sample.mkv")));
        assert!(is_sample(Path::new("/downloads/sample.avi")));
        assert!(is_sample(Path::new("/downloads/movie.sample.mp4")));
        assert!(is_sample(Path::new("/downloads/SAMPLE.MKV")));
        assert!(is_sample(Path::new("/downloads/Movie-SAMPLE-Scene.mkv")));
    }

    #[test]
    fn test_is_sample_not_sample() {
        // Normal files/folders that are not samples
        assert!(!is_sample(Path::new("/downloads/Movie/movie.mkv")));
        assert!(!is_sample(Path::new("/downloads/Movie/Video")));
        assert!(!is_sample(Path::new("/downloads/Movie/Season 01")));
        assert!(!is_sample(Path::new("/downloads/Movie/extras")));
        assert!(!is_sample(Path::new("/downloads/Movie.2020.1080p.mkv")));
    }

    #[test]
    fn test_is_sample_edge_cases() {
        // Edge cases - paths that might be confusing
        // "sampling" does NOT contain "sample" - they are different words
        assert!(!is_sample(Path::new("/downloads/sampling-documentary.mkv")));
        assert!(!is_sample(Path::new("/downloads/examples/movie.mkv")));

        // But these DO contain "sample" as substring
        assert!(is_sample(Path::new("/downloads/resampled-audio.mkv"))); // "resampled" = "re" + "sample" + "d"

        // Normal files that should not be detected
        assert!(!is_sample(Path::new("/downloads/Movie.2020.mkv")));

        // Empty path
        assert!(!is_sample(Path::new("")));

        // Just extension
        assert!(!is_sample(Path::new(".mkv")));
    }
}
