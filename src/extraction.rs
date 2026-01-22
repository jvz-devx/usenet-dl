//! Archive extraction with password support
//!
//! This module handles extracting RAR, 7z, and ZIP archives with password attempts.
//! It supports multiple password sources (cached, per-download, NZB meta, global file, empty).

use crate::db::Database;
use crate::error::{Error, Result};
use crate::types::DownloadId;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Password list collector for archive extraction
///
/// Collects passwords from multiple sources in priority order:
/// 1. Cached correct password (from previous successful extraction)
/// 2. Per-download password (user-specified)
/// 3. NZB metadata password (embedded in NZB)
/// 4. Global password file (one password per line)
/// 5. Empty password (optional fallback)
#[derive(Debug)]
pub struct PasswordList {
    passwords: Vec<String>,
}

impl PasswordList {
    /// Collect passwords from all sources, de-duplicated, in priority order
    pub fn collect(
        cached_correct: Option<&str>,
        download_password: Option<&str>,
        nzb_meta_password: Option<&str>,
        global_file: Option<&Path>,
        try_empty: bool,
    ) -> Self {
        let mut seen = std::collections::HashSet::new();
        let mut passwords = Vec::new();

        // Add in priority order, skip duplicates
        for pw in [cached_correct, download_password, nzb_meta_password]
            .into_iter()
            .flatten()
        {
            if seen.insert(pw.to_string()) {
                passwords.push(pw.to_string());
            }
        }

        // Add from file
        if let Some(path) = global_file {
            if let Ok(content) = std::fs::read_to_string(path) {
                for line in content.lines() {
                    let pw = line.trim();
                    if !pw.is_empty() && seen.insert(pw.to_string()) {
                        passwords.push(pw.to_string());
                    }
                }
            }
        }

        // Empty password last
        if try_empty && seen.insert(String::new()) {
            passwords.push(String::new());
        }

        debug!(
            "collected {} unique passwords for extraction",
            passwords.len()
        );

        Self { passwords }
    }

    /// Get an iterator over passwords
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.passwords.iter()
    }

    /// Check if there are any passwords to try
    pub fn is_empty(&self) -> bool {
        self.passwords.is_empty()
    }

    /// Get the number of passwords
    pub fn len(&self) -> usize {
        self.passwords.len()
    }
}

/// Archive extractor for RAR files
pub struct RarExtractor;

impl RarExtractor {
    /// Detect RAR archive files in a directory
    ///
    /// Looks for .rar files or .r00, .r01, etc. (split archives)
    /// Returns the main archive file (first part)
    pub fn detect_rar_files(download_path: &Path) -> Result<Vec<PathBuf>> {
        debug!(?download_path, "detecting RAR archives");

        let mut archives = Vec::new();

        // Read directory
        let entries = std::fs::read_dir(download_path)
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to read directory: {}", e))))?;

        for entry in entries {
            let entry = entry.map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to read entry: {}", e))))?;
            let path = entry.path();

            // Skip directories
            if path.is_dir() {
                continue;
            }

            // Check for .rar extension
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();

                // Main RAR file or first part of split archive
                if ext_str == "rar" || ext_str == "r00" {
                    archives.push(path);
                }
            }
        }

        debug!("found {} RAR archive(s)", archives.len());
        Ok(archives)
    }

    /// Try to extract a RAR archive with a single password
    ///
    /// Returns Ok(extracted_files) on success
    /// Returns Err with ExtractError::WrongPassword if password is incorrect
    /// Returns Err with other errors for corrupt archives, disk full, etc.
    pub fn try_extract(archive_path: &Path, password: &str, dest_path: &Path) -> Result<Vec<PathBuf>> {
        debug!(
            ?archive_path,
            password_length = password.len(),
            ?dest_path,
            "attempting RAR extraction"
        );

        // Create destination directory if it doesn't exist
        std::fs::create_dir_all(dest_path)
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to create destination: {}", e))))?;

        // Create archive with optional password
        let archive = if password.is_empty() {
            unrar::Archive::new(archive_path)
        } else {
            unrar::Archive::with_password(archive_path, password.as_bytes())
        };

        // Open for processing
        let processor = archive.open_for_processing()
            .map_err(|e| {
                // Check if it's a password error
                let err_str = e.to_string();
                if err_str.contains("password") || err_str.contains("encrypted") || err_str.contains("ERAR_BAD_PASSWORD") {
                    Error::WrongPassword
                } else {
                    Error::ExtractionFailed(format!("failed to open RAR archive: {}", e))
                }
            })?;

        let mut extracted_files = Vec::new();

        // Process each entry using the state machine interface
        let mut at_header = processor;
        loop {
            // Read the next header - transitions to BeforeFile state
            let at_file = match at_header.read_header() {
                Ok(Some(entry_processor)) => entry_processor,
                Ok(None) => break, // No more entries
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("password") || err_str.contains("encrypted") || err_str.contains("ERAR_BAD_PASSWORD") {
                        return Err(Error::WrongPassword);
                    } else {
                        return Err(Error::ExtractionFailed(format!("failed to read RAR header: {}", e)));
                    }
                }
            };

            // Get the file header information (available in BeforeFile state)
            let header = at_file.entry();
            let file_path = dest_path.join(&header.filename);

            // Check if it's a file (not a directory)
            if !header.is_directory() {
                // Extract the file - transitions back to BeforeHeader state
                at_header = at_file.extract_to(&file_path)
                    .map_err(|e| {
                        let err_str = e.to_string();
                        if err_str.contains("password") || err_str.contains("encrypted") || err_str.contains("ERAR_BAD_PASSWORD") {
                            Error::WrongPassword
                        } else {
                            Error::ExtractionFailed(format!("failed to extract file: {}", e))
                        }
                    })?;
                extracted_files.push(file_path);
            } else {
                // Skip directory entries - transitions back to BeforeHeader state
                at_header = at_file.skip()
                    .map_err(|e| Error::ExtractionFailed(format!("failed to skip directory: {}", e)))?;
            }
        }

        info!(
            ?archive_path,
            extracted_count = extracted_files.len(),
            "RAR extraction successful"
        );

        Ok(extracted_files)
    }

    /// Extract RAR archive with password attempts
    ///
    /// Tries each password in the list until one works or all fail.
    /// Caches the successful password in the database.
    pub async fn extract_with_passwords(
        download_id: DownloadId,
        archive_path: &Path,
        dest_path: &Path,
        passwords: &PasswordList,
        db: &Database,
    ) -> Result<Vec<PathBuf>> {
        if passwords.is_empty() {
            warn!(
                download_id,
                ?archive_path,
                "no passwords to try for RAR extraction"
            );
            return Err(Error::NoPasswordsAvailable);
        }

        info!(
            download_id,
            ?archive_path,
            password_count = passwords.len(),
            "attempting RAR extraction with {} password(s)",
            passwords.len()
        );

        for (i, password) in passwords.iter().enumerate() {
            debug!(
                download_id,
                attempt = i + 1,
                total = passwords.len(),
                password_length = password.len(),
                "trying password {}/{}",
                i + 1,
                passwords.len()
            );

            match Self::try_extract(archive_path, password, dest_path) {
                Ok(files) => {
                    info!(
                        download_id,
                        ?archive_path,
                        attempt = i + 1,
                        "RAR extraction successful on attempt {}/{}",
                        i + 1,
                        passwords.len()
                    );

                    // Cache successful password
                    if let Err(e) = db.set_correct_password(download_id, password).await {
                        warn!(
                            download_id,
                            error = %e,
                            "failed to cache correct password"
                        );
                    }

                    return Ok(files);
                }
                Err(Error::WrongPassword) => {
                    debug!(
                        download_id,
                        attempt = i + 1,
                        "wrong password, trying next"
                    );
                    continue;
                }
                Err(e) => {
                    // Other error (corrupt archive, disk full, etc.)
                    warn!(
                        download_id,
                        error = %e,
                        ?archive_path,
                        "RAR extraction failed with non-password error"
                    );
                    return Err(e);
                }
            }
        }

        // All passwords failed
        warn!(
            download_id,
            ?archive_path,
            attempted = passwords.len(),
            "all passwords failed for RAR extraction"
        );
        Err(Error::AllPasswordsFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_list_collect_empty() {
        let passwords = PasswordList::collect(None, None, None, None, false);
        assert!(passwords.is_empty());
        assert_eq!(passwords.len(), 0);
    }

    #[test]
    fn test_password_list_collect_single() {
        let passwords = PasswordList::collect(Some("test123"), None, None, None, false);
        assert_eq!(passwords.len(), 1);
        assert_eq!(passwords.passwords[0], "test123");
    }

    #[test]
    fn test_password_list_collect_multiple_sources() {
        let passwords = PasswordList::collect(
            Some("cached"),
            Some("download"),
            Some("nzb"),
            None,
            false,
        );
        assert_eq!(passwords.len(), 3);
        assert_eq!(passwords.passwords[0], "cached");
        assert_eq!(passwords.passwords[1], "download");
        assert_eq!(passwords.passwords[2], "nzb");
    }

    #[test]
    fn test_password_list_collect_deduplication() {
        // Same password from multiple sources should only appear once
        let passwords = PasswordList::collect(
            Some("duplicate"),
            Some("duplicate"),
            Some("unique"),
            None,
            false,
        );
        assert_eq!(passwords.len(), 2);
        assert_eq!(passwords.passwords[0], "duplicate");
        assert_eq!(passwords.passwords[1], "unique");
    }

    #[test]
    fn test_password_list_collect_with_empty() {
        let passwords = PasswordList::collect(Some("test"), None, None, None, true);
        assert_eq!(passwords.len(), 2);
        assert_eq!(passwords.passwords[0], "test");
        assert_eq!(passwords.passwords[1], "");
    }

    #[test]
    fn test_password_list_priority_order() {
        // Cached should come first, then download, then nzb
        let passwords = PasswordList::collect(
            Some("cached"),
            Some("download"),
            Some("nzb"),
            None,
            true,
        );
        assert_eq!(passwords.len(), 4);
        assert_eq!(passwords.passwords[0], "cached"); // Highest priority
        assert_eq!(passwords.passwords[1], "download");
        assert_eq!(passwords.passwords[2], "nzb");
        assert_eq!(passwords.passwords[3], ""); // Empty last
    }

    #[test]
    fn test_detect_rar_files_empty_dir() {
        // Create a temporary directory
        let temp_dir = tempfile::tempdir().unwrap();
        let result = RarExtractor::detect_rar_files(temp_dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_detect_rar_files_with_rar() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create a fake .rar file
        let rar_path = temp_dir.path().join("test.rar");
        std::fs::write(&rar_path, b"fake rar").unwrap();

        let result = RarExtractor::detect_rar_files(temp_dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], rar_path);
    }

    #[test]
    fn test_detect_rar_files_with_r00() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create a fake .r00 file
        let r00_path = temp_dir.path().join("test.r00");
        std::fs::write(&r00_path, b"fake r00").unwrap();

        let result = RarExtractor::detect_rar_files(temp_dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], r00_path);
    }

    #[test]
    fn test_detect_rar_files_ignores_other_extensions() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create various files
        std::fs::write(temp_dir.path().join("test.rar"), b"rar").unwrap();
        std::fs::write(temp_dir.path().join("test.txt"), b"txt").unwrap();
        std::fs::write(temp_dir.path().join("test.nzb"), b"nzb").unwrap();
        std::fs::write(temp_dir.path().join("test.par2"), b"par2").unwrap();

        let result = RarExtractor::detect_rar_files(temp_dir.path()).unwrap();
        assert_eq!(result.len(), 1); // Only .rar file
    }

    #[test]
    fn test_detect_rar_files_multiple_archives() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create multiple RAR files
        std::fs::write(temp_dir.path().join("archive1.rar"), b"rar1").unwrap();
        std::fs::write(temp_dir.path().join("archive2.rar"), b"rar2").unwrap();
        std::fs::write(temp_dir.path().join("split.r00"), b"r00").unwrap();

        let result = RarExtractor::detect_rar_files(temp_dir.path()).unwrap();
        assert_eq!(result.len(), 3);
    }
}
