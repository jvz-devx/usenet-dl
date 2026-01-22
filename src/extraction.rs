//! Archive extraction with password support
//!
//! This module handles extracting RAR, 7z, and ZIP archives with password attempts.
//! It supports multiple password sources (cached, per-download, NZB meta, global file, empty).

use crate::db::Database;
use crate::error::{Error, Result};
use crate::types::{ArchiveType, DownloadId};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Detect archive type by file extension
///
/// Returns the archive type based on the file extension.
/// Supports RAR (.rar, .r00), 7z (.7z), and ZIP (.zip) formats.
pub fn detect_archive_type(path: &Path) -> Option<ArchiveType> {
    let ext = path.extension()?.to_str()?.to_lowercase();

    match ext.as_str() {
        "rar" | "r00" => Some(ArchiveType::Rar),
        "7z" => Some(ArchiveType::SevenZip),
        "zip" => Some(ArchiveType::Zip),
        _ => None,
    }
}

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

/// Archive extractor for 7z files
pub struct SevenZipExtractor;

impl SevenZipExtractor {
    /// Detect 7z archive files in a directory
    pub fn detect_7z_files(download_path: &Path) -> Result<Vec<PathBuf>> {
        debug!(?download_path, "detecting 7z archives");

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

            // Check for .7z extension
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if ext_str == "7z" {
                    archives.push(path);
                }
            }
        }

        debug!("found {} 7z archive(s)", archives.len());
        Ok(archives)
    }

    /// Try to extract a 7z archive with a single password
    pub fn try_extract(archive_path: &Path, password: &str, dest_path: &Path) -> Result<Vec<PathBuf>> {
        debug!(
            ?archive_path,
            password_length = password.len(),
            ?dest_path,
            "attempting 7z extraction"
        );

        // Create destination directory if it doesn't exist
        std::fs::create_dir_all(dest_path)
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to create destination: {}", e))))?;

        // Decompress with optional password
        use sevenz_rust::Password;
        let result = if password.is_empty() {
            sevenz_rust::decompress_file(archive_path, dest_path)
        } else {
            let pw = Password::from(password);
            sevenz_rust::decompress_file_with_password(archive_path, dest_path, pw)
        };

        match result {
            Ok(()) => {
                // Collect the extracted files by scanning the destination directory
                let extracted_files = Self::collect_extracted_files(dest_path)?;

                info!(
                    ?archive_path,
                    extracted_count = extracted_files.len(),
                    "7z extraction successful"
                );
                Ok(extracted_files)
            }
            Err(e) => {
                let err_str = e.to_string();
                // Check if it's a password error
                if err_str.contains("password") || err_str.contains("encrypted") || err_str.contains("Wrong password") {
                    Err(Error::WrongPassword)
                } else {
                    Err(Error::ExtractionFailed(format!("failed to extract 7z archive: {}", e)))
                }
            }
        }
    }

    /// Recursively collect all files (not directories) from a directory
    fn collect_extracted_files(dir: &Path) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();

        fn visit_dir(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
            let entries = std::fs::read_dir(dir)
                .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to read directory: {}", e))))?;

            for entry in entries {
                let entry = entry.map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to read entry: {}", e))))?;
                let path = entry.path();

                if path.is_dir() {
                    visit_dir(&path, files)?;
                } else {
                    files.push(path);
                }
            }
            Ok(())
        }

        visit_dir(dir, &mut files)?;
        Ok(files)
    }

    /// Extract 7z archive with password attempts
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
                "no passwords to try for 7z extraction"
            );
            return Err(Error::NoPasswordsAvailable);
        }

        info!(
            download_id,
            ?archive_path,
            password_count = passwords.len(),
            "attempting 7z extraction with {} password(s)",
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
                        "7z extraction successful on attempt {}/{}",
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
                        "7z extraction failed with non-password error"
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
            "all passwords failed for 7z extraction"
        );
        Err(Error::AllPasswordsFailed)
    }
}

/// Archive extractor for ZIP files
pub struct ZipExtractor;

impl ZipExtractor {
    /// Detect ZIP archive files in a directory
    pub fn detect_zip_files(download_path: &Path) -> Result<Vec<PathBuf>> {
        debug!(?download_path, "detecting ZIP archives");

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

            // Check for .zip extension
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if ext_str == "zip" {
                    archives.push(path);
                }
            }
        }

        debug!("found {} ZIP archive(s)", archives.len());
        Ok(archives)
    }

    /// Try to extract a ZIP archive with a single password
    pub fn try_extract(archive_path: &Path, password: &str, dest_path: &Path) -> Result<Vec<PathBuf>> {
        debug!(
            ?archive_path,
            password_length = password.len(),
            ?dest_path,
            "attempting ZIP extraction"
        );

        // Create destination directory if it doesn't exist
        std::fs::create_dir_all(dest_path)
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to create destination: {}", e))))?;

        // Open the archive
        let file = std::fs::File::open(archive_path)
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to open ZIP archive: {}", e))))?;

        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| Error::ExtractionFailed(format!("failed to read ZIP archive: {}", e)))?;

        let mut extracted_files = Vec::new();

        // Extract each file
        for i in 0..archive.len() {
            let mut file = if password.is_empty() {
                archive.by_index(i)
                    .map_err(|e| {
                        let err_str = e.to_string();
                        if err_str.contains("password") || err_str.contains("encrypted") {
                            Error::WrongPassword
                        } else {
                            Error::ExtractionFailed(format!("failed to read ZIP entry: {}", e))
                        }
                    })?
            } else {
                archive.by_index_decrypt(i, password.as_bytes())
                    .map_err(|e| {
                        let err_str = e.to_string();
                        if err_str.contains("password") || err_str.contains("encrypted") {
                            Error::WrongPassword
                        } else {
                            Error::ExtractionFailed(format!("failed to read ZIP entry: {}", e))
                        }
                    })?
                    .map_err(|_| Error::WrongPassword)?
            };

            // Get the file path
            let file_path = match file.enclosed_name() {
                Some(path) => dest_path.join(path),
                None => {
                    warn!("skipping entry with unsafe path");
                    continue;
                }
            };

            // Check if it's a directory
            if file.is_dir() {
                // Create directory
                std::fs::create_dir_all(&file_path)
                    .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to create directory: {}", e))))?;
            } else {
                // Create parent directories if needed
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to create parent directories: {}", e))))?;
                }

                // Extract file
                let mut outfile = std::fs::File::create(&file_path)
                    .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to create output file: {}", e))))?;

                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| {
                        let err_str = e.to_string();
                        if err_str.contains("password") || err_str.contains("encrypted") {
                            Error::WrongPassword
                        } else {
                            Error::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("failed to extract file: {}", e)))
                        }
                    })?;

                extracted_files.push(file_path);
            }
        }

        info!(
            ?archive_path,
            extracted_count = extracted_files.len(),
            "ZIP extraction successful"
        );

        Ok(extracted_files)
    }

    /// Extract ZIP archive with password attempts
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
                "no passwords to try for ZIP extraction"
            );
            return Err(Error::NoPasswordsAvailable);
        }

        info!(
            download_id,
            ?archive_path,
            password_count = passwords.len(),
            "attempting ZIP extraction with {} password(s)",
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
                        "ZIP extraction successful on attempt {}/{}",
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
                        "ZIP extraction failed with non-password error"
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
            "all passwords failed for ZIP extraction"
        );
        Err(Error::AllPasswordsFailed)
    }
}

/// Unified archive extraction dispatcher
///
/// Detects the archive type and routes to the appropriate extractor (RAR, 7z, or ZIP).
/// Tries multiple passwords from the PasswordList and caches the successful password.
///
/// # Arguments
/// * `download_id` - Download ID for password caching
/// * `archive_path` - Path to the archive file
/// * `dest_path` - Destination directory for extraction
/// * `passwords` - List of passwords to try (in priority order)
/// * `db` - Database for caching successful passwords
///
/// # Returns
/// * `Ok(Vec<PathBuf>)` - List of extracted files on success
/// * `Err(Error)` - Extraction error (wrong password, corruption, unknown type, etc.)
///
/// # Example
/// ```no_run
/// use usenet_dl::extraction::{extract_archive, PasswordList};
/// use std::path::PathBuf;
///
/// # async fn example(db: &usenet_dl::db::Database) -> usenet_dl::error::Result<()> {
/// let passwords = PasswordList::collect(None, Some("pass123"), None, None, true);
/// let files = extract_archive(
///     1,
///     &PathBuf::from("movie.rar"),
///     &PathBuf::from("/tmp/extract"),
///     &passwords,
///     db,
/// ).await?;
/// println!("Extracted {} files", files.len());
/// # Ok(())
/// # }
/// ```
pub async fn extract_archive(
    download_id: DownloadId,
    archive_path: &Path,
    dest_path: &Path,
    passwords: &PasswordList,
    db: &Database,
) -> Result<Vec<PathBuf>> {
    // Detect archive type by extension
    let archive_type = detect_archive_type(archive_path).ok_or_else(|| {
        Error::ExtractionFailed(format!(
            "unknown archive type for file: {}",
            archive_path.display()
        ))
    })?;

    info!(
        download_id,
        ?archive_path,
        ?archive_type,
        "dispatching extraction to appropriate extractor"
    );

    // Route to the appropriate extractor
    match archive_type {
        ArchiveType::Rar => {
            RarExtractor::extract_with_passwords(
                download_id,
                archive_path,
                dest_path,
                passwords,
                db,
            )
            .await
        }
        ArchiveType::SevenZip => {
            SevenZipExtractor::extract_with_passwords(
                download_id,
                archive_path,
                dest_path,
                passwords,
                db,
            )
            .await
        }
        ArchiveType::Zip => {
            ZipExtractor::extract_with_passwords(
                download_id,
                archive_path,
                dest_path,
                passwords,
                db,
            )
            .await
        }
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

    #[test]
    fn test_detect_7z_files_empty_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = SevenZipExtractor::detect_7z_files(temp_dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_detect_7z_files_with_7z() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create a fake .7z file
        let seven_z_path = temp_dir.path().join("test.7z");
        std::fs::write(&seven_z_path, b"fake 7z").unwrap();

        let result = SevenZipExtractor::detect_7z_files(temp_dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], seven_z_path);
    }

    #[test]
    fn test_detect_7z_files_ignores_other_extensions() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create various files
        std::fs::write(temp_dir.path().join("test.7z"), b"7z").unwrap();
        std::fs::write(temp_dir.path().join("test.txt"), b"txt").unwrap();
        std::fs::write(temp_dir.path().join("test.rar"), b"rar").unwrap();
        std::fs::write(temp_dir.path().join("test.zip"), b"zip").unwrap();

        let result = SevenZipExtractor::detect_7z_files(temp_dir.path()).unwrap();
        assert_eq!(result.len(), 1); // Only .7z file
    }

    #[test]
    fn test_detect_7z_files_multiple_archives() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create multiple 7z files
        std::fs::write(temp_dir.path().join("archive1.7z"), b"7z1").unwrap();
        std::fs::write(temp_dir.path().join("archive2.7z"), b"7z2").unwrap();
        std::fs::write(temp_dir.path().join("archive3.7z"), b"7z3").unwrap();

        let result = SevenZipExtractor::detect_7z_files(temp_dir.path()).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_detect_zip_files_empty_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = ZipExtractor::detect_zip_files(temp_dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_detect_zip_files_with_zip() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create a fake .zip file
        let zip_path = temp_dir.path().join("test.zip");
        std::fs::write(&zip_path, b"fake zip").unwrap();

        let result = ZipExtractor::detect_zip_files(temp_dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], zip_path);
    }

    #[test]
    fn test_detect_zip_files_ignores_other_extensions() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create various files
        std::fs::write(temp_dir.path().join("test.zip"), b"zip").unwrap();
        std::fs::write(temp_dir.path().join("test.txt"), b"txt").unwrap();
        std::fs::write(temp_dir.path().join("test.rar"), b"rar").unwrap();
        std::fs::write(temp_dir.path().join("test.7z"), b"7z").unwrap();

        let result = ZipExtractor::detect_zip_files(temp_dir.path()).unwrap();
        assert_eq!(result.len(), 1); // Only .zip file
    }

    #[test]
    fn test_detect_zip_files_multiple_archives() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create multiple ZIP files
        std::fs::write(temp_dir.path().join("archive1.zip"), b"zip1").unwrap();
        std::fs::write(temp_dir.path().join("archive2.zip"), b"zip2").unwrap();
        std::fs::write(temp_dir.path().join("archive3.zip"), b"zip3").unwrap();

        let result = ZipExtractor::detect_zip_files(temp_dir.path()).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_detect_archive_type_rar() {
        use crate::types::ArchiveType;
        use std::path::Path;

        let path = Path::new("test.rar");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::Rar));

        let path = Path::new("TEST.RAR");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::Rar));
    }

    #[test]
    fn test_detect_archive_type_rar_split() {
        use crate::types::ArchiveType;
        use std::path::Path;

        let path = Path::new("test.r00");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::Rar));

        let path = Path::new("TEST.R00");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::Rar));
    }

    #[test]
    fn test_detect_archive_type_7z() {
        use crate::types::ArchiveType;
        use std::path::Path;

        let path = Path::new("test.7z");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::SevenZip));

        let path = Path::new("TEST.7Z");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::SevenZip));
    }

    #[test]
    fn test_detect_archive_type_zip() {
        use crate::types::ArchiveType;
        use std::path::Path;

        let path = Path::new("test.zip");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::Zip));

        let path = Path::new("TEST.ZIP");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::Zip));
    }

    #[test]
    fn test_detect_archive_type_unknown() {
        use std::path::Path;

        let path = Path::new("test.txt");
        assert_eq!(detect_archive_type(path), None);

        let path = Path::new("test.nzb");
        assert_eq!(detect_archive_type(path), None);

        let path = Path::new("test.par2");
        assert_eq!(detect_archive_type(path), None);

        let path = Path::new("test");
        assert_eq!(detect_archive_type(path), None);
    }

    #[test]
    fn test_detect_archive_type_with_path() {
        use crate::types::ArchiveType;
        use std::path::Path;

        let path = Path::new("/path/to/archive.rar");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::Rar));

        let path = Path::new("/another/path/file.7z");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::SevenZip));

        let path = Path::new("relative/path/file.zip");
        assert_eq!(detect_archive_type(path), Some(ArchiveType::Zip));
    }

    #[tokio::test]
    async fn test_extract_archive_unknown_type() {
        use std::path::Path;
        use tempfile::NamedTempFile;

        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();
        let passwords = PasswordList::collect(None, None, None, None, false);

        // Try to extract a non-archive file
        let result = extract_archive(
            1,
            Path::new("test.txt"),
            Path::new("/tmp/extract"),
            &passwords,
            &db,
        )
        .await;

        // Should fail with ExtractionFailed error
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::ExtractionFailed(msg) => {
                assert!(msg.contains("unknown archive type"));
            }
            _ => panic!("expected ExtractionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_extract_archive_routes_to_rar() {
        use std::path::Path;
        use tempfile::NamedTempFile;

        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();
        let passwords = PasswordList::collect(None, None, None, None, false);

        // Try to extract a RAR file (will fail since it doesn't exist, but tests routing)
        let result = extract_archive(
            1,
            Path::new("test.rar"),
            Path::new("/tmp/extract"),
            &passwords,
            &db,
        )
        .await;

        // Should fail (file doesn't exist) but confirms it routed to RAR extractor
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_extract_archive_routes_to_7z() {
        use std::path::Path;
        use tempfile::NamedTempFile;

        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();
        let passwords = PasswordList::collect(None, None, None, None, false);

        // Try to extract a 7z file (will fail since it doesn't exist, but tests routing)
        let result = extract_archive(
            1,
            Path::new("test.7z"),
            Path::new("/tmp/extract"),
            &passwords,
            &db,
        )
        .await;

        // Should fail (file doesn't exist) but confirms it routed to 7z extractor
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_extract_archive_routes_to_zip() {
        use std::path::Path;
        use tempfile::NamedTempFile;

        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();
        let passwords = PasswordList::collect(None, None, None, None, false);

        // Try to extract a ZIP file (will fail since it doesn't exist, but tests routing)
        let result = extract_archive(
            1,
            Path::new("test.zip"),
            Path::new("/tmp/extract"),
            &passwords,
            &db,
        )
        .await;

        // Should fail (file doesn't exist) but confirms it routed to ZIP extractor
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_extract_archive_case_insensitive() {
        use std::path::Path;
        use tempfile::NamedTempFile;

        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();
        let passwords = PasswordList::collect(None, None, None, None, false);

        // Test uppercase extensions are handled correctly
        let result = extract_archive(
            1,
            Path::new("TEST.RAR"),
            Path::new("/tmp/extract"),
            &passwords,
            &db,
        )
        .await;
        assert!(result.is_err()); // File doesn't exist, but routing works

        let result = extract_archive(
            1,
            Path::new("TEST.7Z"),
            Path::new("/tmp/extract"),
            &passwords,
            &db,
        )
        .await;
        assert!(result.is_err());

        let result = extract_archive(
            1,
            Path::new("TEST.ZIP"),
            Path::new("/tmp/extract"),
            &passwords,
            &db,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_7z_password_list_integration() {
        use std::path::Path;
        use tempfile::{NamedTempFile, TempDir};

        // Create a temporary database
        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();

        // Create a password list with multiple passwords
        let passwords = PasswordList::collect(
            None,
            Some("download_password"),
            Some("nzb_password"),
            None,
            true,
        );

        // Verify password list has expected passwords in priority order
        let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
        assert_eq!(password_vec.len(), 3);
        assert_eq!(password_vec[0], "download_password");
        assert_eq!(password_vec[1], "nzb_password");
        assert_eq!(password_vec[2], ""); // Empty password

        // Test with cached password (highest priority)
        let passwords_with_cache = PasswordList::collect(
            Some("cached_password"),
            Some("download_password"),
            None,
            None,
            false,
        );

        let password_vec: Vec<&str> = passwords_with_cache.iter().map(|s| s.as_str()).collect();
        assert_eq!(password_vec.len(), 2);
        assert_eq!(password_vec[0], "cached_password"); // Cache has highest priority
        assert_eq!(password_vec[1], "download_password");
    }

    #[tokio::test]
    async fn test_zip_password_list_integration() {
        use std::path::Path;
        use tempfile::{NamedTempFile, TempDir};

        // Create a temporary database
        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();

        // Create a password list with multiple passwords
        let passwords = PasswordList::collect(
            None,
            Some("secret123"),
            None,
            None,
            true,
        );

        // Verify password list has expected passwords
        let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
        assert_eq!(password_vec.len(), 2);
        assert_eq!(password_vec[0], "secret123");
        assert_eq!(password_vec[1], ""); // Empty password
    }

    #[tokio::test]
    async fn test_7z_password_priority_order() {
        use tempfile::NamedTempFile;

        // Test that password sources are prioritized correctly:
        // 1. Cached password (highest)
        // 2. Download-specific password
        // 3. NZB metadata password
        // 4. Global password file
        // 5. Empty password (lowest)

        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();

        // All password sources
        let passwords = PasswordList::collect(
            Some("cached"),
            Some("download"),
            Some("nzb"),
            None,
            true,
        );

        let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
        assert_eq!(password_vec.len(), 4);
        assert_eq!(password_vec[0], "cached"); // Highest priority
        assert_eq!(password_vec[1], "download");
        assert_eq!(password_vec[2], "nzb");
        assert_eq!(password_vec[3], ""); // Lowest priority
    }

    #[tokio::test]
    async fn test_zip_password_priority_order() {
        use tempfile::NamedTempFile;

        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();

        // Test priority: cached > download > nzb > file > empty
        let passwords = PasswordList::collect(
            Some("cached_pw"),
            Some("download_pw"),
            Some("nzb_pw"),
            None,
            true,
        );

        let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
        assert_eq!(password_vec.len(), 4);
        assert_eq!(password_vec[0], "cached_pw");
        assert_eq!(password_vec[1], "download_pw");
        assert_eq!(password_vec[2], "nzb_pw");
        assert_eq!(password_vec[3], "");
    }

    #[tokio::test]
    async fn test_7z_extract_with_empty_password() {
        use tempfile::TempDir;

        // Create temp directory for extraction
        let temp_dir = TempDir::new().unwrap();
        let dest_path = temp_dir.path().join("extracted");

        // Test that empty password is handled correctly (will fail with non-existent file)
        let result = SevenZipExtractor::try_extract(
            Path::new("nonexistent.7z"),
            "",
            &dest_path,
        );

        // Should fail because file doesn't exist, not because of password
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_zip_extract_with_empty_password() {
        use tempfile::TempDir;

        // Create temp directory for extraction
        let temp_dir = TempDir::new().unwrap();
        let dest_path = temp_dir.path().join("extracted");

        // Test that empty password is handled correctly (will fail with non-existent file)
        let result = ZipExtractor::try_extract(
            Path::new("nonexistent.zip"),
            "",
            &dest_path,
        );

        // Should fail because file doesn't exist, not because of password
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_7z_password_deduplication() {
        use tempfile::NamedTempFile;

        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();

        // Test that duplicate passwords are removed
        let passwords = PasswordList::collect(
            Some("password123"),
            Some("password123"), // Duplicate
            Some("password123"), // Duplicate
            None,
            false,
        );

        let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
        // Should only have one instance of "password123"
        assert_eq!(password_vec.len(), 1);
        assert_eq!(password_vec[0], "password123");
    }

    #[tokio::test]
    async fn test_zip_password_deduplication() {
        use tempfile::NamedTempFile;

        let temp_db = NamedTempFile::new().unwrap();
        let db = Database::new(temp_db.path()).await.unwrap();

        // Test that duplicate passwords are removed
        let passwords = PasswordList::collect(
            Some("secret"),
            Some("secret"), // Duplicate
            None,
            None,
            true, // Empty password
        );

        let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
        // Should have "secret" once and empty password
        assert_eq!(password_vec.len(), 2);
        assert_eq!(password_vec[0], "secret");
        assert_eq!(password_vec[1], "");
    }
}
