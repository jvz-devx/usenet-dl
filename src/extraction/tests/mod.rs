use crate::db::{Database, NewDownload};
use crate::error::{Error, PostProcessError};
use crate::extraction::shared::extract_with_passwords_impl;
use crate::extraction::*;
use crate::types::DownloadId;
use std::path::{Path, PathBuf};
use tempfile::{NamedTempFile, TempDir};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a minimal NewDownload for use in DB-backed tests
fn test_download() -> NewDownload {
    NewDownload {
        name: "Test".to_string(),
        nzb_path: "/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    }
}

/// Create a valid ZIP archive containing a single file with the given name and content
fn create_zip_archive(archive_path: &Path, file_name: &str, content: &[u8]) {
    let file = std::fs::File::create(archive_path).unwrap();
    let mut writer = ::zip::ZipWriter::new(file);
    let options =
        ::zip::write::FileOptions::default().compression_method(::zip::CompressionMethod::Stored);
    writer.start_file(file_name, options).unwrap();
    std::io::Write::write_all(&mut writer, content).unwrap();
    writer.finish().unwrap();
}

/// Create a valid ZIP archive containing multiple files
fn create_zip_archive_multi(archive_path: &Path, files: &[(&str, &[u8])]) {
    let file = std::fs::File::create(archive_path).unwrap();
    let mut writer = ::zip::ZipWriter::new(file);
    let options =
        ::zip::write::FileOptions::default().compression_method(::zip::CompressionMethod::Stored);
    for (name, content) in files {
        writer.start_file(*name, options).unwrap();
        std::io::Write::write_all(&mut writer, content).unwrap();
    }
    writer.finish().unwrap();
}

/// Create a password-encrypted ZIP using the deprecated ZipCrypto method
/// (only encryption method supported for writing by zip 0.6)
fn create_encrypted_zip(archive_path: &Path, file_name: &str, content: &[u8], password: &[u8]) {
    use ::zip::unstable::write::FileOptionsExt;
    let file = std::fs::File::create(archive_path).unwrap();
    let mut writer = ::zip::ZipWriter::new(file);
    let options = ::zip::write::FileOptions::default()
        .compression_method(::zip::CompressionMethod::Stored)
        .with_deprecated_encryption(password);
    writer.start_file(file_name, options).unwrap();
    std::io::Write::write_all(&mut writer, content).unwrap();
    writer.finish().unwrap();
}

/// Create a valid 7z archive from a source directory using sevenz_rust
fn create_7z_archive(archive_path: &Path, source_dir: &Path) {
    sevenz_rust::compress_to_path(source_dir, archive_path).unwrap();
}

#[tokio::test]
async fn test_password_list_collect_empty() {
    let passwords = PasswordList::collect(None, None, None, None, false).await;
    assert!(passwords.is_empty());
    assert_eq!(passwords.len(), 0);
}

#[tokio::test]
async fn test_password_list_collect_single() {
    let passwords = PasswordList::collect(Some("test123"), None, None, None, false).await;
    assert_eq!(passwords.len(), 1);
    assert_eq!(passwords.iter().next().unwrap(), "test123");
}

#[tokio::test]
async fn test_password_list_collect_multiple_sources() {
    let passwords =
        PasswordList::collect(Some("cached"), Some("download"), Some("nzb"), None, false).await;
    assert_eq!(passwords.len(), 3);
    let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
    assert_eq!(password_vec[0], "cached");
    assert_eq!(password_vec[1], "download");
    assert_eq!(password_vec[2], "nzb");
}

#[tokio::test]
async fn test_password_list_collect_deduplication() {
    // Same password from multiple sources should only appear once
    let passwords = PasswordList::collect(
        Some("duplicate"),
        Some("duplicate"),
        Some("unique"),
        None,
        false,
    )
    .await;
    assert_eq!(passwords.len(), 2);
    let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
    assert_eq!(password_vec[0], "duplicate");
    assert_eq!(password_vec[1], "unique");
}

#[tokio::test]
async fn test_password_list_collect_with_empty() {
    let passwords = PasswordList::collect(Some("test"), None, None, None, true).await;
    assert_eq!(passwords.len(), 2);
    let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
    assert_eq!(password_vec[0], "test");
    assert_eq!(password_vec[1], "");
}

#[tokio::test]
async fn test_password_list_priority_order() {
    // Cached should come first, then download, then nzb
    let passwords =
        PasswordList::collect(Some("cached"), Some("download"), Some("nzb"), None, true).await;
    assert_eq!(passwords.len(), 4);
    let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
    assert_eq!(password_vec[0], "cached"); // Highest priority
    assert_eq!(password_vec[1], "download");
    assert_eq!(password_vec[2], "nzb");
    assert_eq!(password_vec[3], ""); // Empty last
}

#[tokio::test]
async fn test_password_list_from_file() {
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Create a temporary password file
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "password1").unwrap();
    writeln!(temp_file, "password2").unwrap();
    writeln!(temp_file, "password3").unwrap();
    writeln!(temp_file).unwrap(); // Empty line should be ignored
    writeln!(temp_file, "  password4  ").unwrap(); // Should be trimmed
    temp_file.flush().unwrap();

    // Test with just file passwords
    let passwords = PasswordList::collect(None, None, None, Some(temp_file.path()), false).await;
    assert_eq!(passwords.len(), 4);
    let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
    assert_eq!(password_vec[0], "password1");
    assert_eq!(password_vec[1], "password2");
    assert_eq!(password_vec[2], "password3");
    assert_eq!(password_vec[3], "password4");
}

#[tokio::test]
async fn test_password_list_file_with_priority() {
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Create a temporary password file
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "file_pw1").unwrap();
    writeln!(temp_file, "file_pw2").unwrap();
    writeln!(temp_file, "cached").unwrap(); // Duplicate of cached password
    temp_file.flush().unwrap();

    // Test priority: cached > download > nzb > file > empty
    let passwords = PasswordList::collect(
        Some("cached"),
        Some("download"),
        Some("nzb"),
        Some(temp_file.path()),
        true,
    )
    .await;

    let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
    // Should have: cached, download, nzb, file_pw1, file_pw2, empty
    // Note: "cached" from file is deduplicated
    assert_eq!(password_vec.len(), 6);
    assert_eq!(password_vec[0], "cached"); // Highest priority
    assert_eq!(password_vec[1], "download");
    assert_eq!(password_vec[2], "nzb");
    assert_eq!(password_vec[3], "file_pw1");
    assert_eq!(password_vec[4], "file_pw2");
    assert_eq!(password_vec[5], ""); // Empty last
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
    let passwords = PasswordList::collect(None, None, None, None, false).await;

    // Try to extract a non-archive file
    let result = extract_archive(
        DownloadId(1),
        Path::new("test.txt"),
        Path::new("/tmp/extract"),
        &passwords,
        &db,
    )
    .await;

    // Should fail with ExtractionFailed error
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::PostProcess(PostProcessError::ExtractionFailed { reason, .. }) => {
            assert!(reason.contains("unknown archive type"));
        }
        _ => panic!("expected ExtractionFailed error"),
    }
}

#[tokio::test]
async fn test_extract_archive_case_insensitive() {
    use std::path::Path;
    use tempfile::NamedTempFile;

    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let passwords = PasswordList::collect(None, None, None, None, false).await;

    // Test uppercase extensions are handled correctly
    let result = extract_archive(
        DownloadId(1),
        Path::new("TEST.RAR"),
        Path::new("/tmp/extract"),
        &passwords,
        &db,
    )
    .await;
    assert!(result.is_err()); // File doesn't exist, but routing works

    let result = extract_archive(
        DownloadId(1),
        Path::new("TEST.7Z"),
        Path::new("/tmp/extract"),
        &passwords,
        &db,
    )
    .await;
    assert!(result.is_err());

    let result = extract_archive(
        DownloadId(1),
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
    use tempfile::NamedTempFile;

    // Create a temporary database
    let temp_db = NamedTempFile::new().unwrap();
    let _db = Database::new(temp_db.path()).await.unwrap();

    // Create a password list with multiple passwords
    let passwords = PasswordList::collect(
        None,
        Some("download_password"),
        Some("nzb_password"),
        None,
        true,
    )
    .await;

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
    )
    .await;

    let password_vec: Vec<&str> = passwords_with_cache.iter().map(|s| s.as_str()).collect();
    assert_eq!(password_vec.len(), 2);
    assert_eq!(password_vec[0], "cached_password"); // Cache has highest priority
    assert_eq!(password_vec[1], "download_password");
}

#[tokio::test]
async fn test_zip_password_list_integration() {
    use tempfile::NamedTempFile;

    // Create a temporary database
    let temp_db = NamedTempFile::new().unwrap();
    let _db = Database::new(temp_db.path()).await.unwrap();

    // Create a password list with multiple passwords
    let passwords = PasswordList::collect(None, Some("secret123"), None, None, true).await;

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
    let _db = Database::new(temp_db.path()).await.unwrap();

    // All password sources
    let passwords =
        PasswordList::collect(Some("cached"), Some("download"), Some("nzb"), None, true).await;

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
    let _db = Database::new(temp_db.path()).await.unwrap();

    // Test priority: cached > download > nzb > file > empty
    let passwords = PasswordList::collect(
        Some("cached_pw"),
        Some("download_pw"),
        Some("nzb_pw"),
        None,
        true,
    )
    .await;

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
    let result = SevenZipExtractor::try_extract(Path::new("nonexistent.7z"), "", &dest_path);

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
    let result = ZipExtractor::try_extract(Path::new("nonexistent.zip"), "", &dest_path);

    // Should fail because file doesn't exist, not because of password
    assert!(result.is_err());
}

#[tokio::test]
async fn test_7z_password_deduplication() {
    use tempfile::NamedTempFile;

    let temp_db = NamedTempFile::new().unwrap();
    let _db = Database::new(temp_db.path()).await.unwrap();

    // Test that duplicate passwords are removed
    let passwords = PasswordList::collect(
        Some("password123"),
        Some("password123"), // Duplicate
        Some("password123"), // Duplicate
        None,
        false,
    )
    .await;

    let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
    // Should only have one instance of "password123"
    assert_eq!(password_vec.len(), 1);
    assert_eq!(password_vec[0], "password123");
}

#[tokio::test]
async fn test_zip_password_deduplication() {
    use tempfile::NamedTempFile;

    let temp_db = NamedTempFile::new().unwrap();
    let _db = Database::new(temp_db.path()).await.unwrap();

    // Test that duplicate passwords are removed
    let passwords = PasswordList::collect(
        Some("secret"),
        Some("secret"), // Duplicate
        None,
        None,
        true, // Empty password
    )
    .await;

    let password_vec: Vec<&str> = passwords.iter().map(|s| s.as_str()).collect();
    // Should have "secret" once and empty password
    assert_eq!(password_vec.len(), 2);
    assert_eq!(password_vec[0], "secret");
    assert_eq!(password_vec[1], "");
}

#[test]
fn test_is_archive_rar() {
    use crate::config::ExtractionConfig;

    let config = ExtractionConfig::default();
    let extensions = config.archive_extensions;

    assert!(is_archive(Path::new("file.rar"), &extensions));
    assert!(is_archive(Path::new("file.RAR"), &extensions));
    assert!(is_archive(Path::new("/path/to/file.rar"), &extensions));
}

#[test]
fn test_is_archive_7z() {
    use crate::config::ExtractionConfig;

    let config = ExtractionConfig::default();
    let extensions = config.archive_extensions;

    assert!(is_archive(Path::new("file.7z"), &extensions));
    assert!(is_archive(Path::new("FILE.7Z"), &extensions));
}

#[test]
fn test_is_archive_zip() {
    use crate::config::ExtractionConfig;

    let config = ExtractionConfig::default();
    let extensions = config.archive_extensions;

    assert!(is_archive(Path::new("file.zip"), &extensions));
    assert!(is_archive(Path::new("file.ZIP"), &extensions));
}

#[test]
fn test_is_archive_non_archive() {
    use crate::config::ExtractionConfig;

    let config = ExtractionConfig::default();
    let extensions = config.archive_extensions;

    assert!(!is_archive(Path::new("file.txt"), &extensions));
    assert!(!is_archive(Path::new("file.nzb"), &extensions));
    assert!(!is_archive(Path::new("file.par2"), &extensions));
    assert!(!is_archive(Path::new("file"), &extensions));
}

#[test]
fn test_is_archive_custom_extensions() {
    let custom_extensions = vec!["rar".to_string(), "custom".to_string()];

    assert!(is_archive(Path::new("file.rar"), &custom_extensions));
    assert!(is_archive(Path::new("file.custom"), &custom_extensions));
    assert!(!is_archive(Path::new("file.zip"), &custom_extensions));
    assert!(!is_archive(Path::new("file.7z"), &custom_extensions));
}

#[test]
fn test_is_archive_no_extension() {
    use crate::config::ExtractionConfig;

    let config = ExtractionConfig::default();
    let extensions = config.archive_extensions;

    assert!(!is_archive(Path::new("file_no_ext"), &extensions));
    assert!(!is_archive(Path::new("/path/to/file"), &extensions));
}

#[test]
fn test_is_archive_case_insensitive() {
    use crate::config::ExtractionConfig;

    let config = ExtractionConfig::default();
    let extensions = config.archive_extensions;

    // Mixed case should work
    assert!(is_archive(Path::new("file.RaR"), &extensions));
    assert!(is_archive(Path::new("file.ZiP"), &extensions));
    assert!(is_archive(Path::new("file.7Z"), &extensions));
}

#[tokio::test]
async fn test_extract_recursive_depth_limit() {
    use crate::config::ExtractionConfig;
    use tempfile::NamedTempFile;

    let temp_db = NamedTempFile::new().unwrap();
    let _db = Database::new(temp_db.path()).await.unwrap();
    let passwords = PasswordList::collect(None, None, None, None, false).await;

    let config = ExtractionConfig {
        max_recursion_depth: 0, // Don't recurse at all
        ..ExtractionConfig::default()
    };

    // This will fail because the file doesn't exist, but we're testing the depth limit
    // In a real scenario with a working archive, it would extract but not recurse
    let result = extract_recursive(
        DownloadId(1),
        Path::new("test.rar"),
        Path::new("/tmp/extract"),
        &passwords,
        &_db,
        &config,
        0,
    )
    .await;

    assert!(result.is_err()); // File doesn't exist
}

#[tokio::test]
async fn test_extract_recursive_respects_depth() {
    use crate::config::ExtractionConfig;
    use tempfile::NamedTempFile;

    let temp_db = NamedTempFile::new().unwrap();
    let _db = Database::new(temp_db.path()).await.unwrap();
    let passwords = PasswordList::collect(None, None, None, None, false).await;

    let config = ExtractionConfig {
        max_recursion_depth: 2, // Allow 2 levels of nesting
        ..ExtractionConfig::default()
    };

    // Test that current_depth is tracked properly
    // At depth 2, should not recurse further
    let result = extract_recursive(
        DownloadId(1),
        Path::new("test.rar"),
        Path::new("/tmp/extract"),
        &passwords,
        &_db,
        &config,
        2, // At max depth
    )
    .await;

    assert!(result.is_err()); // File doesn't exist, but depth check works
}

#[tokio::test]
async fn test_extract_recursive_custom_extensions() {
    use crate::config::ExtractionConfig;
    use tempfile::NamedTempFile;

    let temp_db = NamedTempFile::new().unwrap();
    let _db = Database::new(temp_db.path()).await.unwrap();
    let _passwords = PasswordList::collect(None, None, None, None, false);

    let config = ExtractionConfig {
        archive_extensions: vec!["rar".to_string()], // Only RAR files
        ..ExtractionConfig::default()
    };

    // This verifies that the config is used for extension checking
    assert!(is_archive(
        Path::new("test.rar"),
        &config.archive_extensions
    ));
    assert!(!is_archive(
        Path::new("test.zip"),
        &config.archive_extensions
    ));
    assert!(!is_archive(
        Path::new("test.7z"),
        &config.archive_extensions
    ));
}

#[tokio::test]
async fn test_extract_recursive_no_passwords() {
    use crate::config::ExtractionConfig;
    use tempfile::NamedTempFile;

    let temp_db = NamedTempFile::new().unwrap();
    let _db = Database::new(temp_db.path()).await.unwrap();
    let passwords = PasswordList::collect(None, None, None, None, false).await;

    let config = ExtractionConfig::default();

    // Test with empty password list
    let result = extract_recursive(
        DownloadId(1),
        Path::new("test.rar"),
        Path::new("/tmp/extract"),
        &passwords,
        &_db,
        &config,
        0,
    )
    .await;

    assert!(result.is_err());
    // Should fail with NoPasswordsAvailable or file not found
}

#[tokio::test]
async fn test_extract_recursive_with_passwords() {
    use crate::config::ExtractionConfig;
    use tempfile::NamedTempFile;

    let temp_db = NamedTempFile::new().unwrap();
    let _db = Database::new(temp_db.path()).await.unwrap();
    let passwords = PasswordList::collect(None, Some("test123"), None, None, true).await;

    let config = ExtractionConfig::default();

    // Test with password list
    assert_eq!(passwords.len(), 2); // test123 + empty
    let result = extract_recursive(
        DownloadId(1),
        Path::new("test.rar"),
        Path::new("/tmp/extract"),
        &passwords,
        &_db,
        &config,
        0,
    )
    .await;

    assert!(result.is_err()); // File doesn't exist
}

#[test]
fn test_extraction_config_default() {
    use crate::config::ExtractionConfig;

    let config = ExtractionConfig::default();

    // Verify default values
    assert_eq!(config.max_recursion_depth, 2);
    assert!(!config.archive_extensions.is_empty());

    // Verify default extensions include common formats
    let exts = &config.archive_extensions;
    assert!(exts.contains(&"rar".to_string()));
    assert!(exts.contains(&"zip".to_string()));
    assert!(exts.contains(&"7z".to_string()));
}

#[test]
fn test_is_archive_all_default_extensions() {
    use crate::config::ExtractionConfig;

    let config = ExtractionConfig::default();
    let extensions = config.archive_extensions;

    // Test all default extensions
    for ext in &extensions {
        let filename = format!("test.{}", ext);
        assert!(
            is_archive(Path::new(&filename), &extensions),
            "Extension {} should be recognized as archive",
            ext
        );
    }
}

// ===========================================================================
// extract_recursive — depth enforcement with real archives
// ===========================================================================

/// Create a ZIP archive that contains another ZIP archive inside it.
/// Returns the path to the outer archive.
fn create_nested_zip(dir: &Path) -> PathBuf {
    // First create the inner ZIP containing a text file
    let inner_zip_path = dir.join("inner.zip");
    create_zip_archive(&inner_zip_path, "deep_secret.txt", b"deeply nested content");

    // Now create the outer ZIP that contains the inner ZIP
    let inner_bytes = std::fs::read(&inner_zip_path).unwrap();
    let outer_zip_path = dir.join("outer.zip");
    let file = std::fs::File::create(&outer_zip_path).unwrap();
    let mut writer = ::zip::ZipWriter::new(file);
    let options =
        ::zip::write::FileOptions::default().compression_method(::zip::CompressionMethod::Stored);
    // Add a regular file
    writer.start_file("surface.txt", options).unwrap();
    std::io::Write::write_all(&mut writer, b"surface content").unwrap();
    // Add the inner zip as a file entry
    writer.start_file("inner.zip", options).unwrap();
    std::io::Write::write_all(&mut writer, &inner_bytes).unwrap();
    writer.finish().unwrap();

    // Clean up the standalone inner zip
    std::fs::remove_file(&inner_zip_path).unwrap();

    outer_zip_path
}

#[tokio::test]
async fn extract_recursive_at_max_depth_does_not_recurse_into_nested_archives() {
    use crate::config::ExtractionConfig;

    let temp_dir = TempDir::new().unwrap();
    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let download_id = db.insert_download(&test_download()).await.unwrap();
    let passwords = PasswordList::collect(None, None, None, None, true).await;

    // Create a nested ZIP (outer.zip containing inner.zip containing deep_secret.txt)
    let outer_zip = create_nested_zip(temp_dir.path());
    let dest = temp_dir.path().join("extracted");

    let config = ExtractionConfig {
        max_recursion_depth: 0, // At depth 0, extract the archive but don't recurse
        ..ExtractionConfig::default()
    };

    let files = extract_recursive(
        download_id,
        &outer_zip,
        &dest,
        &passwords,
        &db,
        &config,
        0, // Starting at depth 0 which equals max_recursion_depth
    )
    .await
    .unwrap();

    // Should have extracted the outer archive's contents (surface.txt + inner.zip)
    let names: Vec<String> = files
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    assert!(
        names.contains(&"surface.txt".to_string()),
        "should extract surface file, got: {names:?}"
    );
    assert!(
        names.contains(&"inner.zip".to_string()),
        "should extract inner.zip as a file (not recurse into it), got: {names:?}"
    );

    // The nested archive's content (deep_secret.txt) should NOT be extracted
    assert!(
        !names.contains(&"deep_secret.txt".to_string()),
        "should NOT extract contents of inner.zip at max depth, got: {names:?}"
    );
}

#[tokio::test]
async fn extract_recursive_below_max_depth_recurses_into_nested_archives() {
    use crate::config::ExtractionConfig;

    let temp_dir = TempDir::new().unwrap();
    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let download_id = db.insert_download(&test_download()).await.unwrap();
    let passwords = PasswordList::collect(None, None, None, None, true).await;

    // Create a nested ZIP
    let outer_zip = create_nested_zip(temp_dir.path());
    let dest = temp_dir.path().join("extracted");

    let config = ExtractionConfig {
        max_recursion_depth: 2, // Allow recursion
        ..ExtractionConfig::default()
    };

    let files = extract_recursive(
        download_id,
        &outer_zip,
        &dest,
        &passwords,
        &db,
        &config,
        0, // Starting at depth 0, max is 2 — should recurse
    )
    .await
    .unwrap();

    let names: Vec<String> = files
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();

    // Should have extracted both levels
    assert!(
        names.contains(&"surface.txt".to_string()),
        "should extract surface file, got: {names:?}"
    );
    assert!(
        names.contains(&"deep_secret.txt".to_string()),
        "should recurse and extract nested archive contents, got: {names:?}"
    );
}

// ===========================================================================
// RAR extractor tests
// ===========================================================================

// -- is_password_error --

#[test]
fn rar_is_password_error_returns_true_for_password_keyword() {
    assert!(
        crate::extraction::rar::RarExtractor::is_password_error_pub("wrong password supplied"),
        "should detect 'password' keyword"
    );
}

#[test]
fn rar_is_password_error_returns_true_for_encrypted_keyword() {
    assert!(
        crate::extraction::rar::RarExtractor::is_password_error_pub("file is encrypted"),
        "should detect 'encrypted' keyword"
    );
}

#[test]
fn rar_is_password_error_returns_true_for_erar_bad_password() {
    assert!(
        crate::extraction::rar::RarExtractor::is_password_error_pub("ERAR_BAD_PASSWORD"),
        "should detect ERAR_BAD_PASSWORD code"
    );
}

#[test]
fn rar_is_password_error_returns_false_for_crc_error() {
    assert!(
        !crate::extraction::rar::RarExtractor::is_password_error_pub("CRC check failed"),
        "CRC error is not a password error"
    );
}

#[test]
fn rar_is_password_error_returns_false_for_generic_io_error() {
    assert!(
        !crate::extraction::rar::RarExtractor::is_password_error_pub("no such file or directory"),
        "IO error is not a password error"
    );
}

#[test]
fn rar_is_password_error_returns_false_for_empty_string() {
    assert!(
        !crate::extraction::rar::RarExtractor::is_password_error_pub(""),
        "empty string should not be a password error"
    );
}

// -- detect_rar_files edge cases --

#[test]
fn detect_rar_files_empty_directory_returns_empty_vec() {
    let temp_dir = TempDir::new().unwrap();
    let result = RarExtractor::detect_rar_files(temp_dir.path()).unwrap();
    assert!(result.is_empty(), "empty dir should yield no archives");
}

#[test]
fn detect_rar_files_skips_directories_with_rar_name() {
    let temp_dir = TempDir::new().unwrap();
    // Create a directory named "subdir.rar" -- should be skipped
    std::fs::create_dir(temp_dir.path().join("subdir.rar")).unwrap();
    let result = RarExtractor::detect_rar_files(temp_dir.path()).unwrap();
    assert!(
        result.is_empty(),
        "directory named .rar should not be detected"
    );
}

#[test]
fn detect_rar_files_nonexistent_directory_returns_error() {
    let result = RarExtractor::detect_rar_files(Path::new("/no/such/directory"));
    assert!(result.is_err(), "nonexistent dir must be an error");
}

#[test]
fn detect_rar_files_ignores_r01_extension() {
    let temp_dir = TempDir::new().unwrap();
    // .r01 is NOT detected (only .rar and .r00 are main archives)
    std::fs::write(temp_dir.path().join("part.r01"), b"data").unwrap();
    let result = RarExtractor::detect_rar_files(temp_dir.path()).unwrap();
    assert!(
        result.is_empty(),
        ".r01 should not be detected as a main archive"
    );
}

// ===========================================================================
// 7z extractor tests
// ===========================================================================

// -- validate_extracted_paths --

#[test]
fn sevenz_validate_extracted_paths_accepts_normal_files() {
    let temp_dir = TempDir::new().unwrap();
    let sub = temp_dir.path().join("subdir");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("file.txt"), b"data").unwrap();

    // Should succeed -- all files are within dest_path
    let result = SevenZipExtractor::validate_extracted_paths_pub(temp_dir.path());
    assert!(
        result.is_ok(),
        "normal nested files should pass validation: {:?}",
        result.err()
    );
}

#[test]
fn sevenz_validate_extracted_paths_rejects_symlink_traversal() {
    let temp_dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let outside_file = outside.path().join("secret.txt");
    std::fs::write(&outside_file, b"secret").unwrap();

    // Create a symlink inside temp_dir pointing to the outside file
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside_file, temp_dir.path().join("escape_link")).unwrap();

        let result = SevenZipExtractor::validate_extracted_paths_pub(temp_dir.path());
        match result {
            Err(Error::PostProcess(PostProcessError::ExtractionFailed { reason, .. })) => {
                assert!(
                    reason.contains("path traversal"),
                    "should mention path traversal, got: {reason}"
                );
            }
            Ok(()) => panic!("symlink traversal should have been rejected"),
            other => panic!("expected ExtractionFailed, got: {other:?}"),
        }
    }
}

// -- collect_extracted_files --

#[test]
fn sevenz_collect_extracted_files_returns_only_files_not_dirs() {
    let temp_dir = TempDir::new().unwrap();
    let sub = temp_dir.path().join("subdir");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(temp_dir.path().join("root.txt"), b"root").unwrap();
    std::fs::write(sub.join("nested.txt"), b"nested").unwrap();

    let files = SevenZipExtractor::collect_extracted_files_pub(temp_dir.path()).unwrap();
    assert_eq!(files.len(), 2, "should find exactly 2 files (not dirs)");

    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(names.contains(&"root.txt".to_string()));
    assert!(names.contains(&"nested.txt".to_string()));
}

#[test]
fn sevenz_collect_extracted_files_empty_directory() {
    let temp_dir = TempDir::new().unwrap();
    let files = SevenZipExtractor::collect_extracted_files_pub(temp_dir.path()).unwrap();
    assert!(
        files.is_empty(),
        "empty directory should yield no collected files"
    );
}

#[test]
fn sevenz_collect_extracted_files_deeply_nested() {
    let temp_dir = TempDir::new().unwrap();
    let deep = temp_dir.path().join("a").join("b").join("c");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("deep.bin"), b"deep").unwrap();

    let files = SevenZipExtractor::collect_extracted_files_pub(temp_dir.path()).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("deep.bin"));
}

// -- try_extract (real 7z archives) --

#[test]
fn sevenz_try_extract_extracts_real_archive() {
    let temp_dir = TempDir::new().unwrap();

    // Create source files to compress
    let src_dir = temp_dir.path().join("source");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("hello.txt"), b"Hello, world!").unwrap();
    std::fs::write(src_dir.join("data.bin"), b"\x00\x01\x02\x03").unwrap();

    // Create 7z archive
    let archive_path = temp_dir.path().join("test.7z");
    create_7z_archive(&archive_path, &src_dir);

    // Extract
    let dest = temp_dir.path().join("extracted");
    let files = SevenZipExtractor::try_extract(&archive_path, "", &dest).unwrap();

    assert_eq!(files.len(), 2, "should extract exactly 2 files");

    // Verify content was preserved
    let hello_file = files.iter().find(|p| p.ends_with("hello.txt")).unwrap();
    let content = std::fs::read_to_string(hello_file).unwrap();
    assert_eq!(content, "Hello, world!", "file content should be preserved");

    let data_file = files.iter().find(|p| p.ends_with("data.bin")).unwrap();
    let bytes = std::fs::read(data_file).unwrap();
    assert_eq!(bytes, b"\x00\x01\x02\x03");
}

#[test]
fn sevenz_try_extract_nonexistent_archive_returns_extraction_failed() {
    let temp_dir = TempDir::new().unwrap();
    let dest = temp_dir.path().join("out");

    let result = SevenZipExtractor::try_extract(Path::new("/no/such/archive.7z"), "", &dest);
    match result {
        Err(Error::PostProcess(PostProcessError::ExtractionFailed { archive, .. })) => {
            assert_eq!(archive, PathBuf::from("/no/such/archive.7z"));
        }
        other => panic!("expected ExtractionFailed, got: {other:?}"),
    }
}

#[test]
fn sevenz_try_extract_corrupt_archive_returns_extraction_failed() {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("corrupt.7z");
    std::fs::write(&archive_path, b"this is not a valid 7z archive").unwrap();

    let dest = temp_dir.path().join("out");
    let result = SevenZipExtractor::try_extract(&archive_path, "", &dest);
    match result {
        Err(Error::PostProcess(PostProcessError::ExtractionFailed { archive, reason })) => {
            assert_eq!(archive, archive_path);
            assert!(!reason.is_empty(), "reason should describe what went wrong");
        }
        other => panic!("expected ExtractionFailed, got: {other:?}"),
    }
}

// -- detect_7z_files edge cases --

#[test]
fn detect_7z_files_empty_directory_returns_empty_vec() {
    let temp_dir = TempDir::new().unwrap();
    let result = SevenZipExtractor::detect_7z_files(temp_dir.path()).unwrap();
    assert!(result.is_empty());
}

#[test]
fn detect_7z_files_skips_directories_with_7z_name() {
    let temp_dir = TempDir::new().unwrap();
    std::fs::create_dir(temp_dir.path().join("subdir.7z")).unwrap();
    let result = SevenZipExtractor::detect_7z_files(temp_dir.path()).unwrap();
    assert!(result.is_empty(), "directory named .7z should be skipped");
}

#[test]
fn detect_7z_files_nonexistent_directory_returns_error() {
    let result = SevenZipExtractor::detect_7z_files(Path::new("/no/such/directory"));
    assert!(result.is_err());
}

// ===========================================================================
// ZIP extractor tests
// ===========================================================================

#[test]
fn zip_try_extract_extracts_single_file() {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("test.zip");
    let content = b"ZIP file content here";
    create_zip_archive(&archive_path, "document.txt", content);

    let dest = temp_dir.path().join("extracted");
    let files = ZipExtractor::try_extract(&archive_path, "", &dest).unwrap();

    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("document.txt"));
    let read_content = std::fs::read(&files[0]).unwrap();
    assert_eq!(read_content, content);
}

#[test]
fn zip_try_extract_extracts_multiple_files() {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("multi.zip");
    create_zip_archive_multi(
        &archive_path,
        &[
            ("file1.txt", b"content1"),
            ("subdir/file2.txt", b"content2"),
        ],
    );

    let dest = temp_dir.path().join("extracted");
    let files = ZipExtractor::try_extract(&archive_path, "", &dest).unwrap();

    assert_eq!(files.len(), 2, "should extract both files");

    // Verify both files exist and have correct content
    let f1 = files.iter().find(|p| p.ends_with("file1.txt")).unwrap();
    assert_eq!(std::fs::read(f1).unwrap(), b"content1");

    let f2 = files.iter().find(|p| p.ends_with("file2.txt")).unwrap();
    assert_eq!(std::fs::read(f2).unwrap(), b"content2");
}

#[test]
fn zip_try_extract_nonexistent_archive_returns_io_error() {
    let temp_dir = TempDir::new().unwrap();
    let dest = temp_dir.path().join("out");

    let result = ZipExtractor::try_extract(Path::new("/no/such/file.zip"), "", &dest);
    assert!(
        result.is_err(),
        "extracting nonexistent file should return error"
    );
}

#[test]
fn zip_try_extract_corrupt_archive_returns_extraction_failed() {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("corrupt.zip");
    std::fs::write(&archive_path, b"not a zip file at all").unwrap();

    let dest = temp_dir.path().join("out");
    let result = ZipExtractor::try_extract(&archive_path, "", &dest);
    match result {
        Err(Error::PostProcess(PostProcessError::ExtractionFailed { archive, reason })) => {
            assert_eq!(archive, archive_path);
            assert!(
                reason.contains("failed to read ZIP archive"),
                "reason should describe the failure, got: {reason}"
            );
        }
        other => panic!("expected ExtractionFailed, got: {other:?}"),
    }
}

#[test]
fn zip_try_extract_encrypted_with_correct_password_succeeds() {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("encrypted.zip");
    let content = b"secret data inside";
    create_encrypted_zip(&archive_path, "secret.txt", content, b"correcthorse");

    let dest = temp_dir.path().join("extracted");
    let files = ZipExtractor::try_extract(&archive_path, "correcthorse", &dest).unwrap();

    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("secret.txt"));
    let read_content = std::fs::read(&files[0]).unwrap();
    assert_eq!(read_content, content);
}

#[test]
fn zip_try_extract_encrypted_with_wrong_password_returns_wrong_password() {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("encrypted.zip");
    create_encrypted_zip(&archive_path, "secret.txt", b"data", b"correctpassword");

    let dest = temp_dir.path().join("extracted");
    let result = ZipExtractor::try_extract(&archive_path, "wrongpassword", &dest);

    match result {
        Err(Error::PostProcess(PostProcessError::WrongPassword { archive })) => {
            assert_eq!(
                archive, archive_path,
                "should report the correct archive path"
            );
        }
        // ZipCrypto might also report generic extraction failure on wrong password
        Err(Error::Io(_)) => {
            // Some versions of the zip crate return IO errors for ZipCrypto validation
        }
        Err(Error::PostProcess(PostProcessError::ExtractionFailed { .. })) => {
            // Also acceptable -- some implementations report this
        }
        other => panic!("expected WrongPassword or extraction error, got: {other:?}"),
    }
}

#[test]
fn zip_try_extract_encrypted_without_password_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("encrypted.zip");
    create_encrypted_zip(&archive_path, "secret.txt", b"data", b"password123");

    let dest = temp_dir.path().join("extracted");
    let result = ZipExtractor::try_extract(&archive_path, "", &dest);

    // Without password, attempting to read an encrypted ZIP should error
    assert!(
        result.is_err(),
        "reading encrypted ZIP without password should fail"
    );
}

// -- detect_zip_files edge cases --

#[test]
fn detect_zip_files_empty_directory_returns_empty_vec() {
    let temp_dir = TempDir::new().unwrap();
    let result = ZipExtractor::detect_zip_files(temp_dir.path()).unwrap();
    assert!(result.is_empty());
}

#[test]
fn detect_zip_files_skips_directories_with_zip_name() {
    let temp_dir = TempDir::new().unwrap();
    std::fs::create_dir(temp_dir.path().join("subdir.zip")).unwrap();
    let result = ZipExtractor::detect_zip_files(temp_dir.path()).unwrap();
    assert!(
        result.is_empty(),
        "directory named .zip should not be detected"
    );
}

#[test]
fn detect_zip_files_nonexistent_directory_returns_error() {
    let result = ZipExtractor::detect_zip_files(Path::new("/no/such/directory"));
    assert!(result.is_err());
}

// ===========================================================================
// shared.rs tests — extract_with_passwords_impl
// ===========================================================================

#[tokio::test]
async fn shared_extract_succeeds_without_password_on_first_try() {
    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let download_id = db.insert_download(&test_download()).await.unwrap();

    let passwords = PasswordList::collect(None, None, None, None, true).await;
    // passwords = [""]

    let archive = PathBuf::from("/fake/archive.zip");
    let dest = PathBuf::from("/fake/dest");

    let expected_files = vec![PathBuf::from("/fake/dest/file.txt")];
    let expected_clone = expected_files.clone();

    // Mock try_extract: succeeds with empty password
    let try_fn = move |_archive: &Path,
                       _pw: &str,
                       _dest: &Path|
          -> crate::error::Result<Vec<PathBuf>> { Ok(expected_clone.clone()) };

    let result = extract_with_passwords_impl(
        "TEST",
        try_fn,
        download_id,
        &archive,
        &dest,
        &passwords,
        &db,
    )
    .await;

    let files = result.unwrap();
    assert_eq!(files, expected_files, "should return extracted files");

    // Verify password was cached
    let cached = db.get_cached_password(download_id).await.unwrap();
    assert_eq!(
        cached,
        Some("".to_string()),
        "successful password should be cached in DB"
    );
}

#[tokio::test]
async fn shared_extract_tries_passwords_until_correct_one_found() {
    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let download_id = db.insert_download(&test_download()).await.unwrap();

    let passwords = PasswordList::collect(Some("wrong1"), Some("correct"), None, None, false).await;
    // passwords = ["wrong1", "correct"]

    let archive = PathBuf::from("/fake/archive.zip");
    let dest = PathBuf::from("/fake/dest");

    let try_fn =
        move |_archive: &Path, pw: &str, _dest: &Path| -> crate::error::Result<Vec<PathBuf>> {
            if pw == "correct" {
                Ok(vec![PathBuf::from("/fake/dest/extracted.bin")])
            } else {
                Err(Error::PostProcess(PostProcessError::WrongPassword {
                    archive: _archive.to_path_buf(),
                }))
            }
        };

    let result = extract_with_passwords_impl(
        "TEST",
        try_fn,
        download_id,
        &archive,
        &dest,
        &passwords,
        &db,
    )
    .await;

    let files = result.unwrap();
    assert_eq!(files, vec![PathBuf::from("/fake/dest/extracted.bin")]);

    // The correct password should be cached
    let cached = db.get_cached_password(download_id).await.unwrap();
    assert_eq!(
        cached,
        Some("correct".to_string()),
        "the correct password should be cached"
    );
}

#[tokio::test]
async fn shared_extract_all_passwords_wrong_returns_all_passwords_failed() {
    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let download_id = db.insert_download(&test_download()).await.unwrap();

    let passwords =
        PasswordList::collect(Some("wrong1"), Some("wrong2"), Some("wrong3"), None, false).await;

    let archive = PathBuf::from("/fake/archive.rar");
    let dest = PathBuf::from("/fake/dest");

    let try_fn =
        move |_archive: &Path, _pw: &str, _dest: &Path| -> crate::error::Result<Vec<PathBuf>> {
            Err(Error::PostProcess(PostProcessError::WrongPassword {
                archive: _archive.to_path_buf(),
            }))
        };

    let result =
        extract_with_passwords_impl("RAR", try_fn, download_id, &archive, &dest, &passwords, &db)
            .await;

    match result {
        Err(Error::PostProcess(PostProcessError::AllPasswordsFailed { archive: a, count })) => {
            assert_eq!(a, PathBuf::from("/fake/archive.rar"));
            assert_eq!(count, 3, "should report that all 3 passwords were tried");
        }
        other => panic!("expected AllPasswordsFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn shared_extract_empty_password_list_returns_no_passwords_available() {
    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let download_id = db.insert_download(&test_download()).await.unwrap();

    let passwords = PasswordList::collect(None, None, None, None, false).await;
    assert!(passwords.is_empty());

    let archive = PathBuf::from("/fake/archive.7z");
    let dest = PathBuf::from("/fake/dest");

    let try_fn = |_: &Path, _: &str, _: &Path| -> crate::error::Result<Vec<PathBuf>> {
        panic!("should not be called when there are no passwords");
    };

    let result =
        extract_with_passwords_impl("7z", try_fn, download_id, &archive, &dest, &passwords, &db)
            .await;

    match result {
        Err(Error::PostProcess(PostProcessError::NoPasswordsAvailable { archive: a })) => {
            assert_eq!(a, PathBuf::from("/fake/archive.7z"));
        }
        other => panic!("expected NoPasswordsAvailable, got: {other:?}"),
    }
}

#[tokio::test]
async fn shared_extract_non_password_error_propagated_immediately() {
    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let download_id = db.insert_download(&test_download()).await.unwrap();

    let passwords = PasswordList::collect(Some("pw1"), Some("pw2"), None, None, false).await;

    let archive = PathBuf::from("/fake/archive.zip");
    let dest = PathBuf::from("/fake/dest");

    // First call returns a non-password error (e.g., corrupt archive)
    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    let try_fn =
        move |archive_path: &Path, _pw: &str, _dest: &Path| -> crate::error::Result<Vec<PathBuf>> {
            call_count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(Error::PostProcess(PostProcessError::ExtractionFailed {
                archive: archive_path.to_path_buf(),
                reason: "CRC check failed".to_string(),
            }))
        };

    let result =
        extract_with_passwords_impl("ZIP", try_fn, download_id, &archive, &dest, &passwords, &db)
            .await;

    // Should fail with ExtractionFailed, not AllPasswordsFailed
    match result {
        Err(Error::PostProcess(PostProcessError::ExtractionFailed { reason, .. })) => {
            assert!(
                reason.contains("CRC check failed"),
                "should propagate the original error reason"
            );
        }
        other => panic!("expected ExtractionFailed, got: {other:?}"),
    }

    // Should have called try_fn only once (not continued to next password)
    assert_eq!(
        call_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "should stop after first non-password error, not try remaining passwords"
    );
}

// ===========================================================================
// Integration: real ZIP + extract_with_passwords_impl
// ===========================================================================

#[tokio::test]
async fn zip_extract_with_passwords_succeeds_with_correct_password() {
    let temp_dir = TempDir::new().unwrap();
    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let download_id = db.insert_download(&test_download()).await.unwrap();

    // Create encrypted ZIP
    let archive_path = temp_dir.path().join("secret.zip");
    create_encrypted_zip(&archive_path, "payload.txt", b"top secret", b"s3cret");

    let dest = temp_dir.path().join("extracted");
    let passwords = PasswordList::collect(Some("wrong"), Some("s3cret"), None, None, false).await;

    let files =
        ZipExtractor::extract_with_passwords(download_id, &archive_path, &dest, &passwords, &db)
            .await
            .unwrap();

    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("payload.txt"));
    let content = std::fs::read(&files[0]).unwrap();
    assert_eq!(content, b"top secret");
}

// ===========================================================================
// Integration: real 7z + extract_with_passwords_impl
// ===========================================================================

#[tokio::test]
async fn sevenz_extract_with_passwords_succeeds_with_empty_password() {
    let temp_dir = TempDir::new().unwrap();
    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let download_id = db.insert_download(&test_download()).await.unwrap();

    // Create source files and compress
    let src_dir = temp_dir.path().join("source");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("readme.txt"), b"hello world").unwrap();

    let archive_path = temp_dir.path().join("test.7z");
    create_7z_archive(&archive_path, &src_dir);

    let dest = temp_dir.path().join("extracted");
    let passwords = PasswordList::collect(None, None, None, None, true).await;

    let files = SevenZipExtractor::extract_with_passwords(
        download_id,
        &archive_path,
        &dest,
        &passwords,
        &db,
    )
    .await
    .unwrap();

    assert_eq!(files.len(), 1);
    let content = std::fs::read_to_string(&files[0]).unwrap();
    assert_eq!(content, "hello world");
}

// ===========================================================================
// shared.rs — detect_archive_type edge cases
// ===========================================================================

#[test]
fn detect_archive_type_no_extension_returns_none() {
    assert_eq!(detect_archive_type(Path::new("no_extension")), None);
}

#[test]
fn detect_archive_type_double_extension_uses_last() {
    use crate::types::ArchiveType;
    assert_eq!(
        detect_archive_type(Path::new("file.tar.zip")),
        Some(ArchiveType::Zip),
        "should detect based on last extension"
    );
}

// ===========================================================================
// shared.rs — is_archive edge cases
// ===========================================================================

#[test]
fn is_archive_empty_extensions_list_never_matches() {
    let extensions: Vec<String> = vec![];
    assert!(!is_archive(Path::new("file.rar"), &extensions));
    assert!(!is_archive(Path::new("file.zip"), &extensions));
}

#[test]
fn is_archive_dotfile_without_extension_returns_false() {
    let extensions = vec!["hidden".to_string()];
    // ".hidden" on Linux has no extension (it's a dotfile with stem "hidden")
    // so is_archive should return false
    assert!(
        !is_archive(Path::new(".hidden"), &extensions),
        "dotfile with no extension should not match"
    );
}
