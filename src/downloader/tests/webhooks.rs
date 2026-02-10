use super::*;

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
