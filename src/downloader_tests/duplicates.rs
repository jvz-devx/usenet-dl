use super::*;

#[tokio::test]
async fn test_check_duplicate_disabled() {
    let temp_dir = tempdir().unwrap();

    // Create config with duplicate detection disabled
    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.servers = vec![];
    config.processing.duplicate = config::DuplicateConfig {
        enabled: false,
        action: config::DuplicateAction::Warn,
        methods: vec![config::DuplicateMethod::NzbHash],
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Check should return None when disabled
    let nzb_content = b"<nzb>test content</nzb>";
    let result = downloader.check_duplicate(nzb_content, "test.nzb").await;
    assert!(
        result.is_none(),
        "Duplicate check should return None when disabled"
    );
}

#[tokio::test]
async fn test_check_duplicate_nzb_hash_no_match() {
    let temp_dir = tempdir().unwrap();

    // Create config with NzbHash detection
    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.servers = vec![];
    config.processing.duplicate = config::DuplicateConfig {
        enabled: true,
        action: config::DuplicateAction::Warn,
        methods: vec![config::DuplicateMethod::NzbHash],
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Check new NZB that doesn't exist yet
    let nzb_content = b"<nzb>unique content</nzb>";
    let result = downloader.check_duplicate(nzb_content, "unique.nzb").await;
    assert!(result.is_none(), "Should not find duplicate for new NZB");
}

#[tokio::test]
async fn test_check_duplicate_nzb_hash_match() {
    let temp_dir = tempdir().unwrap();

    // Create config with NzbHash detection
    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.servers = vec![];
    config.processing.duplicate = config::DuplicateConfig {
        enabled: true,
        action: config::DuplicateAction::Warn,
        methods: vec![config::DuplicateMethod::NzbHash],
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Calculate hash for test content
    use sha2::{Digest, Sha256};
    let nzb_content = b"<nzb>test content</nzb>";
    let mut hasher = Sha256::new();
    hasher.update(nzb_content);
    let hash = format!("{:x}", hasher.finalize());

    // Add a download with this hash
    let download = db::NewDownload {
        name: "existing.nzb".to_string(),
        nzb_path: "/tmp/existing.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some(hash),
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 0,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    let existing_id = downloader.db.insert_download(&download).await.unwrap();

    // Check for duplicate - should find the existing download
    let result = downloader.check_duplicate(nzb_content, "test.nzb").await;
    assert!(result.is_some(), "Should find duplicate by NZB hash");

    let dup = result.unwrap();
    assert_eq!(dup.existing_id, existing_id);
    assert_eq!(dup.existing_name, "existing.nzb");
    assert_eq!(dup.method, config::DuplicateMethod::NzbHash);
}

#[tokio::test]
async fn test_check_duplicate_nzb_name_match() {
    let temp_dir = tempdir().unwrap();

    // Create config with NzbName detection
    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.servers = vec![];
    config.processing.duplicate = config::DuplicateConfig {
        enabled: true,
        action: config::DuplicateAction::Warn,
        methods: vec![config::DuplicateMethod::NzbName],
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Add a download with specific name
    let download = db::NewDownload {
        name: "movie.nzb".to_string(),
        nzb_path: "/tmp/movie.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 0,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    let existing_id = downloader.db.insert_download(&download).await.unwrap();

    // Check for duplicate by name
    let nzb_content = b"<nzb>some content</nzb>";
    let result = downloader.check_duplicate(nzb_content, "movie.nzb").await;
    assert!(result.is_some(), "Should find duplicate by NZB name");

    let dup = result.unwrap();
    assert_eq!(dup.existing_id, existing_id);
    assert_eq!(dup.existing_name, "movie.nzb");
    assert_eq!(dup.method, config::DuplicateMethod::NzbName);
}

#[tokio::test]
async fn test_check_duplicate_job_name_match() {
    let temp_dir = tempdir().unwrap();

    // Create config with JobName detection
    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.servers = vec![];
    config.processing.duplicate = config::DuplicateConfig {
        enabled: true,
        action: config::DuplicateAction::Warn,
        methods: vec![config::DuplicateMethod::JobName],
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Add a download with specific job name
    let download = db::NewDownload {
        name: "abc123def456.nzb".to_string(), // Obfuscated filename
        nzb_path: "/tmp/abc123.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: Some("My.Movie.2024".to_string()), // Deobfuscated job name
        category: None,
        destination: "/downloads".to_string(),
        post_process: 0,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    let existing_id = downloader.db.insert_download(&download).await.unwrap();

    // Check for duplicate by job name
    let nzb_content = b"<nzb>content</nzb>";
    let result = downloader
        .check_duplicate(nzb_content, "My.Movie.2024.nzb")
        .await;
    assert!(result.is_some(), "Should find duplicate by job name");

    let dup = result.unwrap();
    assert_eq!(dup.existing_id, existing_id);
    assert_eq!(dup.existing_name, "abc123def456.nzb");
    assert_eq!(dup.method, config::DuplicateMethod::JobName);
}

#[tokio::test]
async fn test_check_duplicate_multiple_methods_first_match() {
    let temp_dir = tempdir().unwrap();

    // Create config with multiple detection methods
    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.servers = vec![];
    config.processing.duplicate = config::DuplicateConfig {
        enabled: true,
        action: config::DuplicateAction::Warn,
        methods: vec![
            config::DuplicateMethod::NzbHash, // First (highest priority)
            config::DuplicateMethod::NzbName, // Second
            config::DuplicateMethod::JobName, // Third
        ],
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Calculate hash for test content
    use sha2::{Digest, Sha256};
    let nzb_content = b"<nzb>test content</nzb>";
    let mut hasher = Sha256::new();
    hasher.update(nzb_content);
    let hash = format!("{:x}", hasher.finalize());

    // Add a download that matches by hash (highest priority method)
    let download = db::NewDownload {
        name: "different_name.nzb".to_string(),
        nzb_path: "/tmp/different.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some(hash),
        job_name: Some("different_job".to_string()),
        category: None,
        destination: "/downloads".to_string(),
        post_process: 0,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    let existing_id = downloader.db.insert_download(&download).await.unwrap();

    // Check for duplicate - should find by hash (first method)
    let result = downloader
        .check_duplicate(nzb_content, "some_name.nzb")
        .await;
    assert!(
        result.is_some(),
        "Should find duplicate by first matching method"
    );

    let dup = result.unwrap();
    assert_eq!(dup.existing_id, existing_id);
    assert_eq!(
        dup.method,
        config::DuplicateMethod::NzbHash,
        "Should use first matching method (NzbHash)"
    );
}

#[tokio::test]
async fn test_check_duplicate_no_match_any_method() {
    let temp_dir = tempdir().unwrap();

    // Create config with all detection methods
    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.servers = vec![];
    config.processing.duplicate = config::DuplicateConfig {
        enabled: true,
        action: config::DuplicateAction::Warn,
        methods: vec![
            config::DuplicateMethod::NzbHash,
            config::DuplicateMethod::NzbName,
            config::DuplicateMethod::JobName,
        ],
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Add a download with different hash, name, and job name
    let download = db::NewDownload {
        name: "existing.nzb".to_string(),
        nzb_path: "/tmp/existing.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("abc123".to_string()),
        job_name: Some("Existing.Job".to_string()),
        category: None,
        destination: "/downloads".to_string(),
        post_process: 0,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    downloader.db.insert_download(&download).await.unwrap();

    // Check for duplicate with completely different content, name, and job name
    let nzb_content = b"<nzb>totally different content</nzb>";
    let result = downloader.check_duplicate(nzb_content, "new.nzb").await;
    assert!(
        result.is_none(),
        "Should not find duplicate when nothing matches"
    );
}

#[tokio::test]
async fn test_add_nzb_content_duplicate_warn() {
    let temp_dir = tempdir().unwrap();

    // Create config with duplicate detection enabled (Warn action)
    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.download.download_dir = temp_dir.path().join("downloads");
    config.download.temp_dir = temp_dir.path().join("temp");
    config.processing.duplicate = config::DuplicateConfig {
        enabled: true,
        action: config::DuplicateAction::Warn,
        methods: vec![config::DuplicateMethod::NzbHash],
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Create a valid NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.com" date="1234567890" subject="test.bin (1/1)">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Add first download
    let id1 = downloader
        .add_nzb_content(nzb_content, "test.nzb", DownloadOptions::default())
        .await
        .unwrap();
    assert!(id1.0 > 0, "First download should succeed");

    // Subscribe to events to catch duplicate warning
    let mut events = downloader.subscribe();

    // Try to add the same NZB again (should warn but allow)
    let id2 = downloader
        .add_nzb_content(nzb_content, "test-copy.nzb", DownloadOptions::default())
        .await
        .unwrap();
    assert!(id2 > id1, "Second download should succeed with Warn action");

    // Check that duplicate event was emitted
    let event = tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await;
    if let Ok(Ok(Event::DuplicateDetected {
        id,
        name,
        method,
        existing_name,
    })) = event
    {
        assert_eq!(id, id1, "Event should reference existing download");
        assert_eq!(name, "test-copy.nzb", "Event should have new download name");
        assert_eq!(
            method,
            config::DuplicateMethod::NzbHash,
            "Event should show NzbHash method"
        );
        assert_eq!(
            existing_name, "test.nzb",
            "Event should have existing download name"
        );
    } else {
        panic!("Expected DuplicateDetected event, got: {:?}", event);
    }
}

#[tokio::test]
async fn test_add_nzb_content_duplicate_block() {
    let temp_dir = tempdir().unwrap();

    // Create config with duplicate detection enabled (Block action)
    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.download.download_dir = temp_dir.path().join("downloads");
    config.download.temp_dir = temp_dir.path().join("temp");
    config.processing.duplicate = config::DuplicateConfig {
        enabled: true,
        action: config::DuplicateAction::Block,
        methods: vec![config::DuplicateMethod::NzbHash],
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Create a valid NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.com" date="1234567890" subject="test.bin (1/1)">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Add first download
    let id1 = downloader
        .add_nzb_content(nzb_content, "test.nzb", DownloadOptions::default())
        .await
        .unwrap();
    assert!(id1.0 > 0, "First download should succeed");

    // Subscribe to events to catch duplicate warning
    let mut events = downloader.subscribe();

    // Try to add the same NZB again (should block)
    let result = downloader
        .add_nzb_content(nzb_content, "test-copy.nzb", DownloadOptions::default())
        .await;
    assert!(result.is_err(), "Second download should be blocked");

    // Check error message
    if let Err(Error::Duplicate(msg)) = result {
        assert!(
            msg.contains("Duplicate download detected"),
            "Error should mention duplicate"
        );
        assert!(
            msg.contains("test-copy.nzb"),
            "Error should mention new file name"
        );
        assert!(
            msg.contains("NzbHash"),
            "Error should mention detection method"
        );
    } else {
        panic!("Expected Error::Duplicate, got: {:?}", result);
    }

    // Check that duplicate event was emitted before blocking
    let event = tokio::time::timeout(std::time::Duration::from_millis(100), events.recv()).await;
    if let Ok(Ok(Event::DuplicateDetected {
        id,
        name,
        method,
        existing_name,
    })) = event
    {
        assert_eq!(id, id1, "Event should reference existing download");
        assert_eq!(name, "test-copy.nzb", "Event should have new download name");
        assert_eq!(
            method,
            config::DuplicateMethod::NzbHash,
            "Event should show NzbHash method"
        );
        assert_eq!(
            existing_name, "test.nzb",
            "Event should have existing download name"
        );
    } else {
        panic!("Expected DuplicateDetected event, got: {:?}", event);
    }
}

#[tokio::test]
async fn test_add_nzb_content_duplicate_allow() {
    let temp_dir = tempdir().unwrap();

    // Create config with duplicate detection enabled (Allow action)
    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.download.download_dir = temp_dir.path().join("downloads");
    config.download.temp_dir = temp_dir.path().join("temp");
    config.processing.duplicate = config::DuplicateConfig {
        enabled: true,
        action: config::DuplicateAction::Allow,
        methods: vec![config::DuplicateMethod::NzbHash],
    };

    let downloader = std::sync::Arc::new(UsenetDownloader::new(config).await.unwrap());

    // Create a valid NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.com" date="1234567890" subject="test.bin (1/1)">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Add first download
    let id1 = downloader
        .add_nzb_content(nzb_content, "test.nzb", DownloadOptions::default())
        .await
        .unwrap();
    assert!(id1.0 > 0, "First download should succeed");

    // Try to add the same NZB again (should allow without warning)
    let id2 = downloader
        .add_nzb_content(nzb_content, "test-copy.nzb", DownloadOptions::default())
        .await
        .unwrap();
    assert!(
        id2 > id1,
        "Second download should succeed with Allow action"
    );

    // Note: In Allow mode, the event is still emitted (informational)
    // This is acceptable behavior - the action determines whether to block, not whether to emit
}
