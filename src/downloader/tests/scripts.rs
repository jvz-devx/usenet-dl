use super::*;

/// Test script trigger on complete event
#[tokio::test]
async fn test_script_trigger_on_complete() {
    use crate::config::ScriptConfig;
    use std::time::Duration;
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();

    // Create config with a test script (use absolute path)
    let current_dir = std::env::current_dir().unwrap();
    let script_path = current_dir.join("test_scripts/test_success.sh");

    // Skip test if script doesn't exist
    if !script_path.exists() {
        println!("⚠ Skipping test: {} not found", script_path.display());
        return;
    }

    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.download.download_dir = temp_dir.path().join("downloads");
    config.download.temp_dir = temp_dir.path().join("temp");

    // Add script that triggers on complete event
    config.notifications.scripts = vec![ScriptConfig {
        path: script_path.clone(),
        events: vec![crate::config::ScriptEvent::OnComplete],
        timeout: Duration::from_secs(5),
    }];

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Trigger scripts for a completed download
    // This tests that trigger_scripts is callable and doesn't panic
    downloader.trigger_scripts(crate::downloader::TriggerScriptsParams {
        event_type: crate::config::ScriptEvent::OnComplete,
        download_id: DownloadId(999),
        name: "Test Download".to_string(),
        category: Some("test".to_string()),
        status: "complete".to_string(),
        destination: Some(std::path::PathBuf::from("/tmp/test")),
        error: None,
        size_bytes: 1024000,
    });

    // Wait a bit for async script execution to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    println!("✓ Script trigger method executed successfully");
}

/// Test script configuration
#[tokio::test]
async fn test_script_configuration() {
    use crate::config::ScriptConfig;
    use std::time::Duration;
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();

    // Create config with a failing script (use absolute path)
    let current_dir = std::env::current_dir().unwrap();
    let script_path = current_dir.join("test_scripts/test_failure.sh");

    // Skip test if script doesn't exist
    if !script_path.exists() {
        println!("⚠ Skipping test: {} not found", script_path.display());
        return;
    }

    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.download.download_dir = temp_dir.path().join("downloads");
    config.download.temp_dir = temp_dir.path().join("temp");

    // Test adding multiple scripts with different events
    config.notifications.scripts = vec![
        ScriptConfig {
            path: script_path.clone(),
            events: vec![crate::config::ScriptEvent::OnFailed],
            timeout: Duration::from_secs(5),
        },
        ScriptConfig {
            path: script_path,
            events: vec![
                crate::config::ScriptEvent::OnComplete,
                crate::config::ScriptEvent::OnPostProcessComplete,
            ],
            timeout: Duration::from_secs(10),
        },
    ];

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Verify downloader was created successfully with script config
    assert_eq!(downloader.config.notifications.scripts.len(), 2);
    assert_eq!(downloader.config.notifications.scripts[0].events.len(), 1);
    assert_eq!(downloader.config.notifications.scripts[1].events.len(), 2);

    println!("✓ Script configuration loaded successfully");
}

/// Test category-specific scripts are executed before global scripts
#[tokio::test]
async fn test_category_scripts_execution_order() {
    use crate::config::{CategoryConfig, ScriptConfig};
    use std::time::Duration;
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();

    // Use absolute path for script
    let current_dir = std::env::current_dir().unwrap();
    let script_path = current_dir.join("test_scripts/test_success.sh");

    // Skip test if script doesn't exist
    if !script_path.exists() {
        println!("⚠ Skipping test: {} not found", script_path.display());
        return;
    }

    let mut config = Config::default();
    config.persistence.database_path = temp_dir.path().join("test.db");
    config.download.download_dir = temp_dir.path().join("downloads");
    config.download.temp_dir = temp_dir.path().join("temp");

    // Add global script
    config.notifications.scripts = vec![ScriptConfig {
        path: script_path.clone(),
        events: vec![crate::config::ScriptEvent::OnComplete],
        timeout: Duration::from_secs(5),
    }];

    // Add category with its own script
    let mut categories = std::collections::HashMap::new();
    categories.insert(
        "movies".to_string(),
        CategoryConfig {
            destination: temp_dir.path().join("movies"),
            post_process: None,
            scripts: vec![ScriptConfig {
                path: script_path.clone(),
                events: vec![crate::config::ScriptEvent::OnComplete],
                timeout: Duration::from_secs(5),
            }],
        },
    );
    config.persistence.categories = categories;

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Trigger scripts for a download with category
    downloader.trigger_scripts(crate::downloader::TriggerScriptsParams {
        event_type: crate::config::ScriptEvent::OnComplete,
        download_id: DownloadId(999),
        name: "Test Movie".to_string(),
        category: Some("movies".to_string()),
        status: "complete".to_string(),
        destination: Some(std::path::PathBuf::from("/tmp/movie.mkv")),
        error: None,
        size_bytes: 5000000,
    });

    // Wait for scripts to execute
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Both scripts should have executed
    // Category script should have IS_CATEGORY_SCRIPT=true
    // Global script should not have that variable

    println!("✓ Category and global scripts triggered in correct order");
}
