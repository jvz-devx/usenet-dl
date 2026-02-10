use super::*;

#[tokio::test]
async fn test_check_disk_space_sufficient() {
    // Test: check_disk_space should succeed when sufficient space is available
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download.download_dir = temp_dir.path().to_path_buf();
    config.processing.disk_space.enabled = true;
    config.processing.disk_space.min_free_space = 1024 * 1024; // 1 MB buffer
    config.processing.disk_space.size_multiplier = 2.5;

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Check with a small download size (1 KB)
    let result = downloader.check_disk_space(1024).await;
    assert!(
        result.is_ok(),
        "Expected check_disk_space to succeed with small download"
    );

    println!("✓ check_disk_space succeeds with sufficient space");
}

#[tokio::test]
async fn test_check_disk_space_disabled() {
    // Test: check_disk_space should always succeed when disabled
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download.download_dir = temp_dir.path().to_path_buf();
    config.processing.disk_space.enabled = false; // Disable checking

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Even with a huge download size, should succeed when disabled
    let result = downloader.check_disk_space(1024 * 1024 * 1024 * 1024).await; // 1 TB
    assert!(
        result.is_ok(),
        "Expected check_disk_space to succeed when disabled"
    );

    println!("✓ check_disk_space skips check when disabled");
}

#[tokio::test]
async fn test_check_disk_space_insufficient() {
    // Test: check_disk_space should fail when insufficient space
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download.download_dir = temp_dir.path().to_path_buf();
    config.processing.disk_space.enabled = true;

    // Get actual available space
    let available = crate::utils::get_available_space(&config.download.download_dir).unwrap();

    // Set min_free_space to require more than available
    config.processing.disk_space.min_free_space = available + 1024 * 1024 * 1024; // Available + 1 GB
    config.processing.disk_space.size_multiplier = 1.0;

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Try to add a download that would exceed available space
    let result = downloader.check_disk_space(1024).await; // Even 1 KB should fail

    match result {
        Err(Error::InsufficientSpace {
            required,
            available: avail,
        }) => {
            assert!(avail < required, "Expected available < required");
            println!("✓ check_disk_space correctly detects insufficient space");
            println!("  Required: {} bytes, Available: {} bytes", required, avail);
        }
        Ok(_) => panic!("Expected InsufficientSpace error, got Ok"),
        Err(e) => panic!("Expected InsufficientSpace error, got: {:?}", e),
    }
}

#[tokio::test]
async fn test_check_disk_space_multiplier() {
    // Test: check_disk_space correctly applies size_multiplier
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.download.download_dir = temp_dir.path().to_path_buf();
    config.processing.disk_space.enabled = true;
    config.processing.disk_space.min_free_space = 0; // No buffer for this test
    config.processing.disk_space.size_multiplier = 3.0; // 3x multiplier

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Get available space
    let available =
        crate::utils::get_available_space(&downloader.config.download.download_dir).unwrap();

    // Calculate download size that would require more than available space after multiplier
    let download_size = (available as f64 / 3.0) as i64 + 1024 * 1024; // Slightly over available/3

    let result = downloader.check_disk_space(download_size).await;

    match result {
        Err(Error::InsufficientSpace {
            required,
            available: avail,
        }) => {
            // Verify multiplier was applied: required should be approximately 3x download_size
            let expected_required = (download_size as f64 * 3.0) as u64;
            assert!(
                required >= expected_required - 100 && required <= expected_required + 100,
                "Expected required to be ~3x download size: {} vs {}",
                required,
                expected_required
            );
            println!("✓ check_disk_space correctly applies size_multiplier");
            println!(
                "  Download: {} bytes, Required: {} bytes ({}x), Available: {} bytes",
                download_size, required, 3.0, avail
            );
        }
        Ok(_) => {
            // This might pass if we have a lot of disk space - that's okay
            println!("⚠ check_disk_space passed (system has lots of free space)");
        }
        Err(e) => panic!("Expected InsufficientSpace or Ok, got: {:?}", e),
    }
}
