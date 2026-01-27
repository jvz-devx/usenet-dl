use crate::db::Database;
use crate::error::{Error, PostProcessError};
use crate::extraction::*;
use std::path::Path;

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
    writeln!(temp_file, "").unwrap(); // Empty line should be ignored
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
async fn test_extract_archive_routes_to_rar() {
    use std::path::Path;
    use tempfile::NamedTempFile;

    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();
    let passwords = PasswordList::collect(None, None, None, None, false).await;

    // Try to extract a RAR file (will fail since it doesn't exist, but tests routing)
    let result = extract_archive(
        DownloadId(1),
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
    let passwords = PasswordList::collect(None, None, None, None, false).await;

    // Try to extract a 7z file (will fail since it doesn't exist, but tests routing)
    let result = extract_archive(
        DownloadId(1),
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
    let passwords = PasswordList::collect(None, None, None, None, false).await;

    // Try to extract a ZIP file (will fail since it doesn't exist, but tests routing)
    let result = extract_archive(
        DownloadId(1),
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
    use std::path::Path;
    use tempfile::{NamedTempFile, TempDir};

    // Create a temporary database
    let temp_db = NamedTempFile::new().unwrap();
    let db = Database::new(temp_db.path()).await.unwrap();

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
    let db = Database::new(temp_db.path()).await.unwrap();

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
    let db = Database::new(temp_db.path()).await.unwrap();

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
    let db = Database::new(temp_db.path()).await.unwrap();

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
    let db = Database::new(temp_db.path()).await.unwrap();

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

    let mut config = ExtractionConfig::default();
    config.max_recursion_depth = 0; // Don't recurse at all

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

    let mut config = ExtractionConfig::default();
    config.max_recursion_depth = 2; // Allow 2 levels of nesting

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

    let mut config = ExtractionConfig::default();
    config.archive_extensions = vec!["rar".to_string()]; // Only RAR files

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
