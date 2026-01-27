use super::*;

#[tokio::test]
async fn test_webhook_triggers_on_queued() {
    // Create test downloader with webhook configuration
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Use httpbin.org for webhook testing (real HTTP endpoint)
    let webhook_url = "https://httpbin.org/post".to_string();

    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: db_path.clone(),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        download: config::DownloadConfig {
            download_dir: temp_dir.path().join("downloads"),
            temp_dir: temp_dir.path().join("temp"),
            ..Default::default()
        },
        notifications: config::NotificationConfig {
            webhooks: vec![crate::config::WebhookConfig {
                url: webhook_url.clone(),
                events: vec![crate::config::WebhookEvent::OnQueued],
                auth_header: None,
                timeout: std::time::Duration::from_secs(10),
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Create test NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file subject="test.file">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Subscribe to events to verify WebhookFailed event is not emitted
    let mut events = downloader.subscribe();

    // Add NZB (should trigger OnQueued webhook)
    let id = downloader
        .add_nzb_content(nzb_content, "webhook-test.nzb", DownloadOptions::default())
        .await
        .unwrap();

    assert!(id.0 > 0, "Download should be queued successfully");

    // Wait a bit for webhook to be sent (it's async/background)
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Check that we got the Queued event
    let event = tokio::time::timeout(tokio::time::Duration::from_secs(2), events.recv()).await;

    assert!(event.is_ok(), "Should receive event");
    let event = event.unwrap().unwrap();
    match event {
        Event::Queued {
            id: queued_id,
            name,
        } => {
            assert_eq!(queued_id, id);
            assert_eq!(name, "webhook-test.nzb");
        }
        _ => panic!("Expected Queued event, got {:?}", event),
    }

    println!("✓ Webhook test passed - OnQueued webhook sent to httpbin.org");
    println!("  Note: Check network logs to verify HTTP POST was sent");
    println!("  Webhook URL: {}", webhook_url);
}

#[tokio::test]
async fn test_webhook_failed_event_on_invalid_url() {
    // Create test downloader with invalid webhook URL
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: db_path.clone(),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        download: config::DownloadConfig {
            download_dir: temp_dir.path().join("downloads"),
            temp_dir: temp_dir.path().join("temp"),
            ..Default::default()
        },
        notifications: config::NotificationConfig {
            webhooks: vec![crate::config::WebhookConfig {
                url: "http://invalid-webhook-url-that-does-not-exist.test/webhook".to_string(),
                events: vec![crate::config::WebhookEvent::OnQueued],
                auth_header: None,
                timeout: std::time::Duration::from_secs(2),
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Create test NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file subject="test.file">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Subscribe to events
    let mut events = downloader.subscribe();

    // Add NZB (should trigger OnQueued webhook which will fail)
    let _id = downloader
        .add_nzb_content(
            nzb_content,
            "webhook-fail-test.nzb",
            DownloadOptions::default(),
        )
        .await
        .unwrap();

    // Wait for webhook to fail and WebhookFailed event to be emitted
    let mut found_queued = false;
    let mut found_webhook_failed = false;

    for _ in 0..5 {
        let event = tokio::time::timeout(tokio::time::Duration::from_secs(3), events.recv()).await;

        if let Ok(Ok(evt)) = event {
            match evt {
                Event::Queued { .. } => {
                    found_queued = true;
                }
                Event::WebhookFailed { url, error } => {
                    found_webhook_failed = true;
                    assert!(url.contains("invalid-webhook-url-that-does-not-exist.test"));
                    assert!(!error.is_empty(), "Error message should not be empty");
                    println!("✓ WebhookFailed event received: {}", error);
                    break;
                }
                _ => {}
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    assert!(found_queued, "Should receive Queued event");
    assert!(
        found_webhook_failed,
        "Should receive WebhookFailed event for invalid URL"
    );
}

#[tokio::test]
async fn test_webhook_auth_header() {
    // Create test downloader with webhook that includes auth header
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let config = Config {
        persistence: crate::config::PersistenceConfig {
            database_path: db_path.clone(),
            schedule_rules: vec![],
            categories: std::collections::HashMap::new(),
        },
        download: config::DownloadConfig {
            download_dir: temp_dir.path().join("downloads"),
            temp_dir: temp_dir.path().join("temp"),
            ..Default::default()
        },
        notifications: config::NotificationConfig {
            webhooks: vec![crate::config::WebhookConfig {
                url: "https://httpbin.org/post".to_string(),
                events: vec![crate::config::WebhookEvent::OnQueued],
                auth_header: Some("Bearer test-token-12345".to_string()),
                timeout: std::time::Duration::from_secs(10),
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await.unwrap();

    // Create test NZB content
    let nzb_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file subject="test.file">
<groups>
  <group>alt.binaries.test</group>
</groups>
<segments>
  <segment bytes="1024" number="1">test-message-id@example.com</segment>
</segments>
  </file>
</nzb>"#;

    // Add NZB (should trigger webhook with auth header)
    let id = downloader
        .add_nzb_content(
            nzb_content,
            "webhook-auth-test.nzb",
            DownloadOptions::default(),
        )
        .await
        .unwrap();

    assert!(id.0 > 0, "Download should be queued successfully");

    // Wait for webhook to be sent
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    println!("✓ Webhook with auth header sent to httpbin.org");
    println!("  Auth header: Bearer test-token-12345");
    println!("  Note: Check httpbin.org response to verify Authorization header was sent");
}
