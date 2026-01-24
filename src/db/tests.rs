use super::*;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_database_creation() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let db = Database::new(db_path).await.unwrap();

    // Verify tables exist
    let mut conn = db.pool.acquire().await.unwrap();

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();

    assert!(tables.contains(&"downloads".to_string()));
    assert!(tables.contains(&"download_articles".to_string()));
    assert!(tables.contains(&"passwords".to_string()));
    assert!(tables.contains(&"processed_nzbs".to_string()));
    assert!(tables.contains(&"history".to_string()));
    assert!(tables.contains(&"schema_version".to_string()));
    assert!(tables.contains(&"runtime_state".to_string()));
    assert!(tables.contains(&"rss_feeds".to_string()));
    assert!(tables.contains(&"rss_filters".to_string()));
    assert!(tables.contains(&"rss_seen".to_string()));

    db.close().await;
}

#[tokio::test]
async fn test_migration_idempotency() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    // Create database twice
    let db1 = Database::new(db_path).await.unwrap();
    db1.close().await;

    let db2 = Database::new(db_path).await.unwrap();

    // Verify schema version is 3 (latest)
    let mut conn = db2.pool.acquire().await.unwrap();
    let version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM schema_version")
        .fetch_one(&mut *conn)
        .await
        .unwrap();

    assert_eq!(version, 3);

    db2.close().await;
}

#[tokio::test]
async fn test_insert_and_get_download() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Insert a download
    let new_download = NewDownload {
        name: "Test Download".to_string(),
        nzb_path: "/path/to/test.nzb".to_string(),
        nzb_meta_name: Some("Test Meta Name".to_string()),
        nzb_hash: Some("abc123".to_string()),
        job_name: Some("test_job".to_string()),
        category: Some("movies".to_string()),
        destination: "/downloads/movies".to_string(),
        post_process: 4, // UnpackAndCleanup
        priority: 0, // Normal
        status: 0, // Queued
        size_bytes: 1024 * 1024 * 100, // 100 MB
    };

    let id = db.insert_download(&new_download).await.unwrap();
    assert!(id > 0);

    // Get the download
    let download = db.get_download(id).await.unwrap();
    assert!(download.is_some());

    let download = download.unwrap();
    assert_eq!(download.name, "Test Download");
    assert_eq!(download.nzb_path, "/path/to/test.nzb");
    assert_eq!(download.category, Some("movies".to_string()));
    assert_eq!(download.status, 0);
    assert_eq!(download.progress, 0.0);
    assert_eq!(download.size_bytes, 1024 * 1024 * 100);

    db.close().await;
}

#[tokio::test]
async fn test_list_downloads() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Insert multiple downloads
    for i in 0..3 {
        let new_download = NewDownload {
            name: format!("Download {}", i),
            nzb_path: format!("/path/to/test{}.nzb", i),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: i as i32,
            status: 0,
            size_bytes: 1024,
        };
        db.insert_download(&new_download).await.unwrap();
    }

    // List all downloads
    let downloads = db.list_downloads().await.unwrap();
    assert_eq!(downloads.len(), 3);

    // Should be ordered by priority DESC
    assert_eq!(downloads[0].name, "Download 2");
    assert_eq!(downloads[1].name, "Download 1");
    assert_eq!(downloads[2].name, "Download 0");

    db.close().await;
}

#[tokio::test]
async fn test_update_status() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let new_download = NewDownload {
        name: "Test".to_string(),
        nzb_path: "/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0, // Queued
        size_bytes: 1024,
    };

    let id = db.insert_download(&new_download).await.unwrap();

    // Update status to Downloading (1)
    db.update_status(id, 1).await.unwrap();

    let download = db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.status, 1);

    db.close().await;
}

#[tokio::test]
async fn test_update_progress() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let new_download = NewDownload {
        name: "Test".to_string(),
        nzb_path: "/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 1, // Downloading
        size_bytes: 1024 * 1024,
    };

    let id = db.insert_download(&new_download).await.unwrap();

    // Update progress
    db.update_progress(id, 45.5, 1024 * 1024, 500 * 1024).await.unwrap();

    let download = db.get_download(id).await.unwrap().unwrap();
    assert_eq!(download.progress, 45.5);
    assert_eq!(download.speed_bps, 1024 * 1024);
    assert_eq!(download.downloaded_bytes, 500 * 1024);

    db.close().await;
}

#[tokio::test]
async fn test_delete_download() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let new_download = NewDownload {
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
    };

    let id = db.insert_download(&new_download).await.unwrap();

    // Delete the download
    db.delete_download(id).await.unwrap();

    // Should not exist anymore
    let download = db.get_download(id).await.unwrap();
    assert!(download.is_none());

    db.close().await;
}

#[tokio::test]
async fn test_get_incomplete_downloads() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Insert downloads with different statuses
    for (i, status) in [0, 1, 2, 3, 4, 5].iter().enumerate() {
        let new_download = NewDownload {
            name: format!("Download {}", i),
            nzb_path: format!("/test{}.nzb", i),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: *status,
            size_bytes: 1024,
        };
        db.insert_download(&new_download).await.unwrap();
    }

    // Get incomplete (statuses: 0=Queued, 1=Downloading, 3=Processing)
    let incomplete = db.get_incomplete_downloads().await.unwrap();

    // Should only have 3 downloads (status 0, 1, 3)
    assert_eq!(incomplete.len(), 3);

    db.close().await;
}

// Article-level tracking tests

#[tokio::test]
async fn test_insert_and_get_article() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Create a download first
    let new_download = NewDownload {
        name: "Test Download".to_string(),
        nzb_path: "/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0,
        size_bytes: 1024 * 1024,
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Insert an article
    let new_article = NewArticle {
        download_id,
        message_id: "<test@example.com>".to_string(),
        segment_number: 1,
        size_bytes: 512 * 1024,
    };
    let article_id = db.insert_article(&new_article).await.unwrap();
    assert!(article_id > 0);

    // Get the article
    let article = db.get_article_by_message_id(download_id, "<test@example.com>")
        .await.unwrap();
    assert!(article.is_some());

    let article = article.unwrap();
    assert_eq!(article.download_id, download_id);
    assert_eq!(article.message_id, "<test@example.com>");
    assert_eq!(article.segment_number, 1);
    assert_eq!(article.size_bytes, 512 * 1024);
    assert_eq!(article.status, super::article_status::PENDING);
    assert!(article.downloaded_at.is_none());

    db.close().await;
}

#[tokio::test]
async fn test_insert_articles_batch() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Create a download
    let new_download = NewDownload {
        name: "Test Download".to_string(),
        nzb_path: "/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0,
        size_bytes: 1024 * 1024,
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Insert multiple articles in a batch
    let articles: Vec<NewArticle> = (0..100).map(|i| NewArticle {
        download_id,
        message_id: format!("<article{}@example.com>", i),
        segment_number: i,
        size_bytes: 10240,
    }).collect();

    db.insert_articles_batch(&articles).await.unwrap();

    // Verify all articles were inserted
    let count = db.count_articles(download_id).await.unwrap();
    assert_eq!(count, 100);

    // Verify they're all pending
    let pending_count = db.count_articles_by_status(
        download_id,
        super::article_status::PENDING
    ).await.unwrap();
    assert_eq!(pending_count, 100);

    db.close().await;
}

#[tokio::test]
async fn test_update_article_status() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Create a download and article
    let new_download = NewDownload {
        name: "Test".to_string(),
        nzb_path: "/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 1, // Downloading
        size_bytes: 1024,
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    let new_article = NewArticle {
        download_id,
        message_id: "<test@example.com>".to_string(),
        segment_number: 1,
        size_bytes: 1024,
    };
    let article_id = db.insert_article(&new_article).await.unwrap();

    // Update status to DOWNLOADED
    db.update_article_status(article_id, super::article_status::DOWNLOADED)
        .await.unwrap();

    // Verify status was updated
    let article = db.get_article_by_message_id(download_id, "<test@example.com>")
        .await.unwrap().unwrap();
    assert_eq!(article.status, super::article_status::DOWNLOADED);
    assert!(article.downloaded_at.is_some());

    db.close().await;
}

#[tokio::test]
async fn test_get_pending_articles() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Create a download
    let new_download = NewDownload {
        name: "Test".to_string(),
        nzb_path: "/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 1,
        size_bytes: 10240,
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Insert 10 articles
    let articles: Vec<NewArticle> = (0..10).map(|i| NewArticle {
        download_id,
        message_id: format!("<article{}@example.com>", i),
        segment_number: i,
        size_bytes: 1024,
    }).collect();
    db.insert_articles_batch(&articles).await.unwrap();

    // Mark some as downloaded
    for i in 0..5 {
        db.update_article_status_by_message_id(
            download_id,
            &format!("<article{}@example.com>", i),
            super::article_status::DOWNLOADED,
        ).await.unwrap();
    }

    // Mark one as failed
    db.update_article_status_by_message_id(
        download_id,
        "<article5@example.com>",
        super::article_status::FAILED,
    ).await.unwrap();

    // Get pending articles (should be 4 remaining: 6, 7, 8, 9)
    let pending = db.get_pending_articles(download_id).await.unwrap();
    assert_eq!(pending.len(), 4);
    assert_eq!(pending[0].segment_number, 6);
    assert_eq!(pending[1].segment_number, 7);
    assert_eq!(pending[2].segment_number, 8);
    assert_eq!(pending[3].segment_number, 9);

    // Verify counts
    let downloaded_count = db.count_articles_by_status(
        download_id,
        super::article_status::DOWNLOADED
    ).await.unwrap();
    assert_eq!(downloaded_count, 5);

    let failed_count = db.count_articles_by_status(
        download_id,
        super::article_status::FAILED
    ).await.unwrap();
    assert_eq!(failed_count, 1);

    db.close().await;
}

#[tokio::test]
async fn test_delete_articles_cascade() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Create a download
    let new_download = NewDownload {
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
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Insert articles
    let articles: Vec<NewArticle> = (0..5).map(|i| NewArticle {
        download_id,
        message_id: format!("<article{}@example.com>", i),
        segment_number: i,
        size_bytes: 1024,
    }).collect();
    db.insert_articles_batch(&articles).await.unwrap();

    // Verify articles exist
    let count = db.count_articles(download_id).await.unwrap();
    assert_eq!(count, 5);

    // Delete the download (should cascade delete articles)
    db.delete_download(download_id).await.unwrap();

    // Verify articles were deleted via cascade
    let count = db.count_articles(download_id).await.unwrap();
    assert_eq!(count, 0);

    db.close().await;
}

// Password cache tests

#[tokio::test]
async fn test_set_and_get_cached_password() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Create a download
    let new_download = NewDownload {
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
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Initially should have no cached password
    let password = db.get_cached_password(download_id).await.unwrap();
    assert!(password.is_none());

    // Set a password
    db.set_correct_password(download_id, "secret123").await.unwrap();

    // Get the password
    let password = db.get_cached_password(download_id).await.unwrap();
    assert_eq!(password, Some("secret123".to_string()));

    db.close().await;
}

#[tokio::test]
async fn test_update_cached_password() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Create a download
    let new_download = NewDownload {
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
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Set initial password
    db.set_correct_password(download_id, "password1").await.unwrap();

    // Update to new password (should use ON CONFLICT to update)
    db.set_correct_password(download_id, "password2").await.unwrap();

    // Should have the new password
    let password = db.get_cached_password(download_id).await.unwrap();
    assert_eq!(password, Some("password2".to_string()));

    db.close().await;
}

#[tokio::test]
async fn test_delete_cached_password() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Create a download
    let new_download = NewDownload {
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
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Set a password
    db.set_correct_password(download_id, "secret").await.unwrap();

    // Verify it exists
    let password = db.get_cached_password(download_id).await.unwrap();
    assert!(password.is_some());

    // Delete the password
    db.delete_cached_password(download_id).await.unwrap();

    // Should be gone
    let password = db.get_cached_password(download_id).await.unwrap();
    assert!(password.is_none());

    db.close().await;
}

#[tokio::test]
async fn test_password_cascade_delete() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Create a download
    let new_download = NewDownload {
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
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Set a password
    db.set_correct_password(download_id, "password123").await.unwrap();

    // Verify password exists
    let password = db.get_cached_password(download_id).await.unwrap();
    assert_eq!(password, Some("password123".to_string()));

    // Delete the download (should cascade delete password)
    db.delete_download(download_id).await.unwrap();

    // Password should be automatically deleted via CASCADE
    let password = db.get_cached_password(download_id).await.unwrap();
    assert!(password.is_none());

    db.close().await;
}

#[tokio::test]
async fn test_empty_password() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Create a download
    let new_download = NewDownload {
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
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Set an empty password (valid use case for password-less archives)
    db.set_correct_password(download_id, "").await.unwrap();

    // Should be able to retrieve empty password
    let password = db.get_cached_password(download_id).await.unwrap();
    assert_eq!(password, Some("".to_string()));

    db.close().await;
}

// Duplicate detection tests

#[tokio::test]
async fn test_find_by_nzb_hash() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Insert a download with a specific NZB hash
    let new_download = NewDownload {
        name: "Test Download".to_string(),
        nzb_path: "/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: Some("abc123def456".to_string()),
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Find by NZB hash (should find it)
    let found = db.find_by_nzb_hash("abc123def456").await.unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.id, download_id);
    assert_eq!(found.name, "Test Download");
    assert_eq!(found.nzb_hash, Some("abc123def456".to_string()));

    // Try to find with non-existent hash (should return None)
    let not_found = db.find_by_nzb_hash("nonexistent").await.unwrap();
    assert!(not_found.is_none());

    db.close().await;
}

#[tokio::test]
async fn test_find_by_nzb_hash_multiple() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Insert multiple downloads with different hashes
    let downloads = vec![
        ("Download 1", "hash1"),
        ("Download 2", "hash2"),
        ("Download 3", "hash3"),
    ];

    for (name, hash) in &downloads {
        let new_download = NewDownload {
            name: name.to_string(),
            nzb_path: format!("/{}.nzb", name),
            nzb_meta_name: None,
            nzb_hash: Some(hash.to_string()),
            job_name: None,
            category: None,
            destination: "/downloads".to_string(),
            post_process: 4,
            priority: 0,
            status: 0,
            size_bytes: 1024,
        };
        db.insert_download(&new_download).await.unwrap();
    }

    // Find each by hash
    for (name, hash) in &downloads {
        let found = db.find_by_nzb_hash(hash).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, *name);
    }

    db.close().await;
}

#[tokio::test]
async fn test_find_by_name() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Insert a download with a specific name
    let new_download = NewDownload {
        name: "Unique Download Name".to_string(),
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
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Find by exact name (should find it)
    let found = db.find_by_name("Unique Download Name").await.unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.id, download_id);
    assert_eq!(found.name, "Unique Download Name");

    // Case-sensitive: different case should not match
    let not_found = db.find_by_name("unique download name").await.unwrap();
    assert!(not_found.is_none());

    // Different name should not match
    let not_found = db.find_by_name("Different Name").await.unwrap();
    assert!(not_found.is_none());

    db.close().await;
}

#[tokio::test]
async fn test_find_by_name_returns_first_match() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Insert two downloads with the same name (shouldn't happen in practice but test LIMIT 1)
    let new_download1 = NewDownload {
        name: "Same Name".to_string(),
        nzb_path: "/test1.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    let id1 = db.insert_download(&new_download1).await.unwrap();

    let new_download2 = NewDownload {
        name: "Same Name".to_string(),
        nzb_path: "/test2.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0,
        size_bytes: 2048,
    };
    db.insert_download(&new_download2).await.unwrap();

    // Should return the first one (LIMIT 1)
    let found = db.find_by_name("Same Name").await.unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.id, id1);
    assert_eq!(found.nzb_path, "/test1.nzb");

    db.close().await;
}

#[tokio::test]
async fn test_find_by_job_name() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Insert a download with a deobfuscated job name
    let new_download = NewDownload {
        name: "a3f8d9e1b2c4.nzb".to_string(), // Obfuscated name
        nzb_path: "/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: Some("My Movie 2024".to_string()), // Deobfuscated job name
        category: Some("movies".to_string()),
        destination: "/downloads/movies".to_string(),
        post_process: 4,
        priority: 0,
        status: 4, // Complete
        size_bytes: 1024 * 1024 * 1024,
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Find by job name (should find it)
    let found = db.find_by_job_name("My Movie 2024").await.unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.id, download_id);
    assert_eq!(found.job_name, Some("My Movie 2024".to_string()));
    assert_eq!(found.name, "a3f8d9e1b2c4.nzb");

    // Try to find with non-existent job name (should return None)
    let not_found = db.find_by_job_name("Different Movie").await.unwrap();
    assert!(not_found.is_none());

    db.close().await;
}

#[tokio::test]
async fn test_find_by_job_name_null_handling() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Insert downloads with and without job names
    let download_with_job = NewDownload {
        name: "With Job Name".to_string(),
        nzb_path: "/test1.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: Some("actual_job_name".to_string()),
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    db.insert_download(&download_with_job).await.unwrap();

    let download_without_job = NewDownload {
        name: "Without Job Name".to_string(),
        nzb_path: "/test2.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None, // No job name
        category: None,
        destination: "/downloads".to_string(),
        post_process: 4,
        priority: 0,
        status: 0,
        size_bytes: 1024,
    };
    db.insert_download(&download_without_job).await.unwrap();

    // Find by existing job name
    let found = db.find_by_job_name("actual_job_name").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "With Job Name");

    // Try to find with a job name that doesn't exist (should not match NULL)
    let not_found = db.find_by_job_name("nonexistent").await.unwrap();
    assert!(not_found.is_none());

    db.close().await;
}

#[tokio::test]
async fn test_duplicate_detection_priority() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    // Insert a download with all three duplicate detection fields
    let new_download = NewDownload {
        name: "Test.Movie.2024.nzb".to_string(),
        nzb_path: "/test.nzb".to_string(),
        nzb_meta_name: Some("Test Movie 2024".to_string()),
        nzb_hash: Some("hash123abc".to_string()),
        job_name: Some("Test.Movie.2024".to_string()),
        category: Some("movies".to_string()),
        destination: "/downloads/movies".to_string(),
        post_process: 4,
        priority: 0,
        status: 4, // Complete
        size_bytes: 5 * 1024 * 1024 * 1024, // 5 GB
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Test all three detection methods find the same download
    let by_hash = db.find_by_nzb_hash("hash123abc").await.unwrap();
    assert!(by_hash.is_some());
    assert_eq!(by_hash.as_ref().unwrap().id, download_id);

    let by_name = db.find_by_name("Test.Movie.2024.nzb").await.unwrap();
    assert!(by_name.is_some());
    assert_eq!(by_name.as_ref().unwrap().id, download_id);

    let by_job = db.find_by_job_name("Test.Movie.2024").await.unwrap();
    assert!(by_job.is_some());
    assert_eq!(by_job.as_ref().unwrap().id, download_id);

    // All three methods should return the same complete download info
    assert_eq!(by_hash.as_ref().unwrap().name, by_name.as_ref().unwrap().name);
    assert_eq!(by_hash.as_ref().unwrap().category, by_job.as_ref().unwrap().category);
    assert_eq!(by_hash.as_ref().unwrap().status, 4);

    db.close().await;
}

// ==================== History Tests ====================

#[tokio::test]
async fn test_insert_history() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let now = chrono::Utc::now().timestamp();
    let entry = NewHistoryEntry {
        name: "Test.Movie.2024".to_string(),
        category: Some("movies".to_string()),
        destination: Some(PathBuf::from("/downloads/movies/Test.Movie.2024")),
        status: 4, // Complete
        size_bytes: 5 * 1024 * 1024 * 1024, // 5 GB
        download_time_secs: 3600, // 1 hour
        completed_at: now,
    };

    let id = db.insert_history(&entry).await.unwrap();
    assert!(id > 0);

    // Verify the entry was inserted
    let retrieved = db.get_history_entry(id).await.unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, id);
    assert_eq!(retrieved.name, "Test.Movie.2024");
    assert_eq!(retrieved.category, Some("movies".to_string()));
    assert_eq!(retrieved.destination, Some(PathBuf::from("/downloads/movies/Test.Movie.2024")));
    assert_eq!(retrieved.status, Status::Complete);
    assert_eq!(retrieved.size_bytes, 5 * 1024 * 1024 * 1024);
    assert_eq!(retrieved.download_time.as_secs(), 3600);
    assert_eq!(retrieved.completed_at.timestamp(), now);

    db.close().await;
}

#[tokio::test]
async fn test_query_history_pagination() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let now = chrono::Utc::now().timestamp();

    // Insert 5 history entries
    for i in 0..5 {
        let entry = NewHistoryEntry {
            name: format!("Download.{}", i),
            category: None,
            destination: None,
            status: 4, // Complete
            size_bytes: 1024 * 1024,
            download_time_secs: 60,
            completed_at: now - (i as i64 * 60), // Different timestamps
        };
        db.insert_history(&entry).await.unwrap();
    }

    // Query all with pagination
    let page1 = db.query_history(None, 3, 0).await.unwrap();
    assert_eq!(page1.len(), 3);
    assert_eq!(page1[0].name, "Download.0"); // Most recent first

    let page2 = db.query_history(None, 3, 3).await.unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].name, "Download.3");

    // Test count
    let count = db.count_history(None).await.unwrap();
    assert_eq!(count, 5);

    db.close().await;
}

#[tokio::test]
async fn test_query_history_status_filter() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let now = chrono::Utc::now().timestamp();

    // Insert 3 complete and 2 failed entries
    for i in 0..5 {
        let entry = NewHistoryEntry {
            name: format!("Download.{}", i),
            category: None,
            destination: None,
            status: if i < 3 { 4 } else { 5 }, // 4=Complete, 5=Failed
            size_bytes: 1024 * 1024,
            download_time_secs: 60,
            completed_at: now - (i as i64),
        };
        db.insert_history(&entry).await.unwrap();
    }

    // Query only complete
    let complete = db.query_history(Some(4), 10, 0).await.unwrap();
    assert_eq!(complete.len(), 3);
    assert!(complete.iter().all(|e| e.status == Status::Complete));

    // Query only failed
    let failed = db.query_history(Some(5), 10, 0).await.unwrap();
    assert_eq!(failed.len(), 2);
    assert!(failed.iter().all(|e| e.status == Status::Failed));

    // Count by status
    let complete_count = db.count_history(Some(4)).await.unwrap();
    assert_eq!(complete_count, 3);

    let failed_count = db.count_history(Some(5)).await.unwrap();
    assert_eq!(failed_count, 2);

    db.close().await;
}

#[tokio::test]
async fn test_delete_history_before() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let now = chrono::Utc::now().timestamp();
    let thirty_days_ago = now - (30 * 24 * 60 * 60);

    // Insert 3 old entries and 2 recent entries
    for i in 0..5 {
        let entry = NewHistoryEntry {
            name: format!("Download.{}", i),
            category: None,
            destination: None,
            status: 4,
            size_bytes: 1024 * 1024,
            download_time_secs: 60,
            completed_at: if i < 3 { thirty_days_ago - 1000 } else { now },
        };
        db.insert_history(&entry).await.unwrap();
    }

    // Delete old entries
    let deleted = db.delete_history_before(thirty_days_ago).await.unwrap();
    assert_eq!(deleted, 3);

    // Verify only recent entries remain
    let remaining = db.query_history(None, 10, 0).await.unwrap();
    assert_eq!(remaining.len(), 2);

    db.close().await;
}

#[tokio::test]
async fn test_delete_history_by_status() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let now = chrono::Utc::now().timestamp();

    // Insert 3 complete and 2 failed entries
    for i in 0..5 {
        let entry = NewHistoryEntry {
            name: format!("Download.{}", i),
            category: None,
            destination: None,
            status: if i < 3 { 4 } else { 5 },
            size_bytes: 1024 * 1024,
            download_time_secs: 60,
            completed_at: now,
        };
        db.insert_history(&entry).await.unwrap();
    }

    // Delete all failed entries
    let deleted = db.delete_history_by_status(5).await.unwrap();
    assert_eq!(deleted, 2);

    // Verify only complete entries remain
    let remaining = db.query_history(None, 10, 0).await.unwrap();
    assert_eq!(remaining.len(), 3);
    assert!(remaining.iter().all(|e| e.status == Status::Complete));

    db.close().await;
}

#[tokio::test]
async fn test_clear_history() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let now = chrono::Utc::now().timestamp();

    // Insert 5 entries
    for i in 0..5 {
        let entry = NewHistoryEntry {
            name: format!("Download.{}", i),
            category: None,
            destination: None,
            status: 4,
            size_bytes: 1024 * 1024,
            download_time_secs: 60,
            completed_at: now,
        };
        db.insert_history(&entry).await.unwrap();
    }

    // Verify entries exist
    let count = db.count_history(None).await.unwrap();
    assert_eq!(count, 5);

    // Clear all
    let deleted = db.clear_history().await.unwrap();
    assert_eq!(deleted, 5);

    // Verify all gone
    let remaining = db.query_history(None, 10, 0).await.unwrap();
    assert_eq!(remaining.len(), 0);

    let count = db.count_history(None).await.unwrap();
    assert_eq!(count, 0);

    db.close().await;
}

#[tokio::test]
async fn test_get_history_entry_not_found() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let result = db.get_history_entry(9999).await.unwrap();
    assert!(result.is_none());

    db.close().await;
}

#[tokio::test]
async fn test_history_ordering() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let db = Database::new(db_path).await.unwrap();

    let now = chrono::Utc::now().timestamp();

    // Insert entries with different timestamps
    let timestamps = vec![now - 1000, now, now - 500];
    for (i, ts) in timestamps.iter().enumerate() {
        let entry = NewHistoryEntry {
            name: format!("Download.{}", i),
            category: None,
            destination: None,
            status: 4,
            size_bytes: 1024 * 1024,
            download_time_secs: 60,
            completed_at: *ts,
        };
        db.insert_history(&entry).await.unwrap();
    }

    // Query should return most recent first
    let results = db.query_history(None, 10, 0).await.unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].name, "Download.1"); // now (most recent)
    assert_eq!(results[1].name, "Download.2"); // now - 500
    assert_eq!(results[2].name, "Download.0"); // now - 1000 (oldest)

    db.close().await;
}

#[tokio::test]
async fn test_shutdown_state_initial() {
    // Test initial shutdown state after migration
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // After migration, shutdown state should be "false" (unclean)
    let was_unclean = db.was_unclean_shutdown().await.unwrap();
    assert!(was_unclean, "Initial state should indicate unclean shutdown");

    db.close().await;
}

#[tokio::test]
async fn test_shutdown_state_clean_lifecycle() {
    // Test clean start and shutdown sequence
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Mark clean start (application started)
    db.set_clean_start().await.unwrap();
    let was_unclean = db.was_unclean_shutdown().await.unwrap();
    assert!(was_unclean, "After clean start, should still indicate unclean (not yet shut down)");

    // Mark clean shutdown (application shutting down gracefully)
    db.set_clean_shutdown().await.unwrap();
    let was_unclean = db.was_unclean_shutdown().await.unwrap();
    assert!(!was_unclean, "After clean shutdown, should indicate clean");

    db.close().await;
}

#[tokio::test]
async fn test_shutdown_state_unclean_detection() {
    // Test unclean shutdown detection (crash scenario)
    let temp_file = NamedTempFile::new().unwrap();

    // First session: start but don't shut down cleanly (simulating crash)
    {
        let db = Database::new(temp_file.path()).await.unwrap();
        db.set_clean_start().await.unwrap();
        // Intentionally NOT calling set_clean_shutdown() - simulates crash
        db.close().await;
    }

    // Second session: detect unclean shutdown
    {
        let db = Database::new(temp_file.path()).await.unwrap();
        let was_unclean = db.was_unclean_shutdown().await.unwrap();
        assert!(was_unclean, "Should detect unclean shutdown from previous session");

        // Now do a clean shutdown
        db.set_clean_start().await.unwrap();
        db.set_clean_shutdown().await.unwrap();
        db.close().await;
    }

    // Third session: should be clean now
    {
        let db = Database::new(temp_file.path()).await.unwrap();
        let was_unclean = db.was_unclean_shutdown().await.unwrap();
        assert!(!was_unclean, "Should detect clean shutdown from previous session");
        db.close().await;
    }
}

#[tokio::test]
async fn test_rss_tables_schema() {
    // Test RSS feed tables schema
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Verify rss_feeds table schema
    let mut conn = db.pool.acquire().await.unwrap();

    // Test inserting into rss_feeds
    let result = sqlx::query(
        r#"
        INSERT INTO rss_feeds (name, url, check_interval_secs, category, auto_download, priority, enabled, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#
    )
    .bind("Test Feed")
    .bind("https://example.com/rss")
    .bind(900)
    .bind("movies")
    .bind(1)
    .bind(0)
    .bind(1)
    .bind(chrono::Utc::now().timestamp())
    .execute(&mut *conn)
    .await;

    assert!(result.is_ok(), "Should insert into rss_feeds table");
    let feed_id = result.unwrap().last_insert_rowid();

    // Test inserting into rss_filters
    let result = sqlx::query(
        r#"
        INSERT INTO rss_filters (feed_id, name, include_patterns, exclude_patterns, min_size, max_size, max_age_secs)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#
    )
    .bind(feed_id)
    .bind("Test Filter")
    .bind(r#"["pattern1","pattern2"]"#)
    .bind(r#"["exclude1"]"#)
    .bind(1000000)
    .bind(5000000000i64)
    .bind(86400)
    .execute(&mut *conn)
    .await;

    assert!(result.is_ok(), "Should insert into rss_filters table");

    // Test inserting into rss_seen
    let result = sqlx::query(
        r#"
        INSERT INTO rss_seen (feed_id, guid, seen_at)
        VALUES (?, ?, ?)
        "#
    )
    .bind(feed_id)
    .bind("https://example.com/item1")
    .bind(chrono::Utc::now().timestamp())
    .execute(&mut *conn)
    .await;

    assert!(result.is_ok(), "Should insert into rss_seen table");

    // Test foreign key constraint: deleting feed should cascade to filters and seen items
    let result = sqlx::query("DELETE FROM rss_feeds WHERE id = ?")
        .bind(feed_id)
        .execute(&mut *conn)
        .await;

    assert!(result.is_ok(), "Should delete feed");

    // Verify cascade delete worked
    let filter_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rss_filters WHERE feed_id = ?")
        .bind(feed_id)
        .fetch_one(&mut *conn)
        .await
        .unwrap();

    assert_eq!(filter_count, 0, "Filters should be deleted by cascade");

    let seen_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rss_seen WHERE feed_id = ?")
        .bind(feed_id)
        .fetch_one(&mut *conn)
        .await
        .unwrap();

    assert_eq!(seen_count, 0, "Seen items should be deleted by cascade");

    db.close().await;
}

#[tokio::test]
async fn test_rss_seen_primary_key_constraint() {
    // Test rss_seen composite primary key
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    let mut conn = db.pool.acquire().await.unwrap();

    // Insert a feed
    let feed_id = sqlx::query(
        r#"
        INSERT INTO rss_feeds (name, url, check_interval_secs, created_at)
        VALUES (?, ?, ?, ?)
        "#
    )
    .bind("Test Feed")
    .bind("https://example.com/rss")
    .bind(900)
    .bind(chrono::Utc::now().timestamp())
    .execute(&mut *conn)
    .await
    .unwrap()
    .last_insert_rowid();

    // Insert first seen item
    let result = sqlx::query(
        r#"
        INSERT INTO rss_seen (feed_id, guid, seen_at)
        VALUES (?, ?, ?)
        "#
    )
    .bind(feed_id)
    .bind("item-guid-123")
    .bind(chrono::Utc::now().timestamp())
    .execute(&mut *conn)
    .await;

    assert!(result.is_ok(), "First insert should succeed");

    // Try to insert duplicate (same feed_id and guid)
    let result = sqlx::query(
        r#"
        INSERT INTO rss_seen (feed_id, guid, seen_at)
        VALUES (?, ?, ?)
        "#
    )
    .bind(feed_id)
    .bind("item-guid-123")
    .bind(chrono::Utc::now().timestamp())
    .execute(&mut *conn)
    .await;

    assert!(result.is_err(), "Duplicate insert should fail due to primary key constraint");

    db.close().await;
}

#[tokio::test]
async fn test_rss_feeds_default_values() {
    // Test RSS feed default values
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    let mut conn = db.pool.acquire().await.unwrap();

    // Insert feed with minimal values (testing defaults)
    let feed_id = sqlx::query(
        r#"
        INSERT INTO rss_feeds (name, url, created_at)
        VALUES (?, ?, ?)
        "#
    )
    .bind("Test Feed")
    .bind("https://example.com/rss")
    .bind(chrono::Utc::now().timestamp())
    .execute(&mut *conn)
    .await
    .unwrap()
    .last_insert_rowid();

    // Fetch the feed and verify defaults
    let (check_interval, auto_download, priority, enabled): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT check_interval_secs, auto_download, priority, enabled FROM rss_feeds WHERE id = ?"
    )
    .bind(feed_id)
    .fetch_one(&mut *conn)
    .await
    .unwrap();

    assert_eq!(check_interval, 900, "Default check_interval should be 900 seconds");
    assert_eq!(auto_download, 1, "Default auto_download should be 1 (true)");
    assert_eq!(priority, 0, "Default priority should be 0");
    assert_eq!(enabled, 1, "Default enabled should be 1 (true)");

    db.close().await;
}

#[tokio::test]
async fn test_is_rss_item_seen_returns_false_for_new_item() {
    // Test is_rss_item_seen returns false for new items
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Insert a feed
    let feed_id = {
        let mut conn = db.pool.acquire().await.unwrap();
        sqlx::query(
            r#"
            INSERT INTO rss_feeds (name, url, check_interval_secs, created_at)
            VALUES (?, ?, ?, ?)
            "#
        )
        .bind("Test Feed")
        .bind("https://example.com/rss")
        .bind(900)
        .bind(chrono::Utc::now().timestamp())
        .execute(&mut *conn)
        .await
        .unwrap()
        .last_insert_rowid()
    }; // Drop connection here

    // Check a GUID that hasn't been seen
    let seen = db.is_rss_item_seen(feed_id, "new-item-guid").await.unwrap();
    assert!(!seen, "New item should not be marked as seen");

    db.close().await;
}

#[tokio::test]
async fn test_mark_rss_item_seen_and_check() {
    // Test marking an item as seen and checking it
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Insert a feed
    let feed_id = {
        let mut conn = db.pool.acquire().await.unwrap();
        sqlx::query(
            r#"
            INSERT INTO rss_feeds (name, url, check_interval_secs, created_at)
            VALUES (?, ?, ?, ?)
            "#
        )
        .bind("Test Feed")
        .bind("https://example.com/rss")
        .bind(900)
        .bind(chrono::Utc::now().timestamp())
        .execute(&mut *conn)
        .await
        .unwrap()
        .last_insert_rowid()
    }; // Drop connection here

    let guid = "item-guid-456";

    // Initially not seen
    let seen_before = db.is_rss_item_seen(feed_id, guid).await.unwrap();
    assert!(!seen_before, "Item should not be seen before marking");

    // Mark as seen
    db.mark_rss_item_seen(feed_id, guid).await.unwrap();

    // Now should be seen
    let seen_after = db.is_rss_item_seen(feed_id, guid).await.unwrap();
    assert!(seen_after, "Item should be marked as seen after marking");

    db.close().await;
}

#[tokio::test]
async fn test_mark_rss_item_seen_idempotent() {
    // Test that marking the same item multiple times is idempotent
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Insert a feed
    let feed_id = {
        let mut conn = db.pool.acquire().await.unwrap();
        sqlx::query(
            r#"
            INSERT INTO rss_feeds (name, url, check_interval_secs, created_at)
            VALUES (?, ?, ?, ?)
            "#
        )
        .bind("Test Feed")
        .bind("https://example.com/rss")
        .bind(900)
        .bind(chrono::Utc::now().timestamp())
        .execute(&mut *conn)
        .await
        .unwrap()
        .last_insert_rowid()
    }; // Drop connection here

    let guid = "duplicate-test-guid";

    // Mark first time
    db.mark_rss_item_seen(feed_id, guid).await.unwrap();

    // Mark second time (should not error)
    db.mark_rss_item_seen(feed_id, guid).await.unwrap();

    // Mark third time (should not error)
    db.mark_rss_item_seen(feed_id, guid).await.unwrap();

    // Still should be seen
    let seen = db.is_rss_item_seen(feed_id, guid).await.unwrap();
    assert!(seen, "Item should still be marked as seen");

    // Verify only one record exists
    let count: i64 = {
        let mut conn = db.pool.acquire().await.unwrap();
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM rss_seen WHERE feed_id = ? AND guid = ?"
        )
        .bind(feed_id)
        .bind(guid)
        .fetch_one(&mut *conn)
        .await
        .unwrap()
    };

    assert_eq!(count, 1, "Should have exactly one record even after multiple marks");

    db.close().await;
}

#[tokio::test]
async fn test_rss_item_seen_different_feeds() {
    // Test that same GUID in different feeds are tracked separately
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Insert two feeds
    let (feed1_id, feed2_id) = {
        let mut conn = db.pool.acquire().await.unwrap();

        let feed1_id = sqlx::query(
            r#"
            INSERT INTO rss_feeds (name, url, check_interval_secs, created_at)
            VALUES (?, ?, ?, ?)
            "#
        )
        .bind("Feed 1")
        .bind("https://example.com/rss1")
        .bind(900)
        .bind(chrono::Utc::now().timestamp())
        .execute(&mut *conn)
        .await
        .unwrap()
        .last_insert_rowid();

        let feed2_id = sqlx::query(
            r#"
            INSERT INTO rss_feeds (name, url, check_interval_secs, created_at)
            VALUES (?, ?, ?, ?)
            "#
        )
        .bind("Feed 2")
        .bind("https://example.com/rss2")
        .bind(900)
        .bind(chrono::Utc::now().timestamp())
        .execute(&mut *conn)
        .await
        .unwrap()
        .last_insert_rowid();

        (feed1_id, feed2_id)
    }; // Drop connection here

    let same_guid = "shared-guid";

    // Mark seen in feed1
    db.mark_rss_item_seen(feed1_id, same_guid).await.unwrap();

    // Should be seen in feed1
    let seen_feed1 = db.is_rss_item_seen(feed1_id, same_guid).await.unwrap();
    assert!(seen_feed1, "Item should be seen in feed1");

    // Should NOT be seen in feed2
    let seen_feed2 = db.is_rss_item_seen(feed2_id, same_guid).await.unwrap();
    assert!(!seen_feed2, "Same GUID should not be seen in feed2");

    // Mark seen in feed2
    db.mark_rss_item_seen(feed2_id, same_guid).await.unwrap();

    // Now should be seen in both
    let seen_feed2_after = db.is_rss_item_seen(feed2_id, same_guid).await.unwrap();
    assert!(seen_feed2_after, "Item should now be seen in feed2");

    db.close().await;
}

#[tokio::test]
async fn test_rss_item_seen_with_different_guids() {
    // Test tracking multiple different items in same feed
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Insert a feed
    let feed_id = {
        let mut conn = db.pool.acquire().await.unwrap();
        sqlx::query(
            r#"
            INSERT INTO rss_feeds (name, url, check_interval_secs, created_at)
            VALUES (?, ?, ?, ?)
            "#
        )
        .bind("Test Feed")
        .bind("https://example.com/rss")
        .bind(900)
        .bind(chrono::Utc::now().timestamp())
        .execute(&mut *conn)
        .await
        .unwrap()
        .last_insert_rowid()
    }; // Drop connection here

    let guid1 = "item-1";
    let guid2 = "item-2";
    let guid3 = "item-3";

    // Mark guid1 and guid2 as seen
    db.mark_rss_item_seen(feed_id, guid1).await.unwrap();
    db.mark_rss_item_seen(feed_id, guid2).await.unwrap();

    // Check all three
    let seen1 = db.is_rss_item_seen(feed_id, guid1).await.unwrap();
    let seen2 = db.is_rss_item_seen(feed_id, guid2).await.unwrap();
    let seen3 = db.is_rss_item_seen(feed_id, guid3).await.unwrap();

    assert!(seen1, "Item 1 should be seen");
    assert!(seen2, "Item 2 should be seen");
    assert!(!seen3, "Item 3 should not be seen");

    db.close().await;
}

// Batch article status update tests

#[tokio::test]
async fn test_batch_update_empty_input() {
    // Verify empty batch is handled gracefully
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Empty batch should succeed without error
    let result = db.update_articles_status_batch(&[]).await;
    assert!(result.is_ok(), "Empty batch should succeed");

    db.close().await;
}

#[tokio::test]
async fn test_batch_update_single_article() {
    // Verify batch update works with single article
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Create download and article
    let new_download = NewDownload {
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
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    let new_article = NewArticle {
        download_id,
        message_id: "<test@example.com>".to_string(),
        segment_number: 1,
        size_bytes: 1024,
    };
    let article_id = db.insert_article(&new_article).await.unwrap();

    // Batch update single article to DOWNLOADED
    let updates = vec![(article_id, super::article_status::DOWNLOADED)];
    db.update_articles_status_batch(&updates).await.unwrap();

    // Verify status updated
    let article = db.get_article_by_message_id(download_id, "<test@example.com>")
        .await.unwrap().unwrap();
    assert_eq!(article.status, super::article_status::DOWNLOADED);
    assert!(article.downloaded_at.is_some(), "downloaded_at should be set");

    db.close().await;
}

#[tokio::test]
async fn test_batch_update_multiple_articles() {
    // Verify batch update correctly updates multiple articles
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Create download
    let new_download = NewDownload {
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
        size_bytes: 1024 * 10,
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Create 10 articles
    let articles: Vec<NewArticle> = (0..10).map(|i| NewArticle {
        download_id,
        message_id: format!("<article{}@example.com>", i),
        segment_number: i,
        size_bytes: 1024,
    }).collect();
    db.insert_articles_batch(&articles).await.unwrap();

    // Get article IDs
    let all_articles = db.get_articles(download_id).await.unwrap();
    assert_eq!(all_articles.len(), 10);

    // Batch update all to DOWNLOADED
    let updates: Vec<(i64, i32)> = all_articles.iter()
        .map(|a| (a.id, super::article_status::DOWNLOADED))
        .collect();
    db.update_articles_status_batch(&updates).await.unwrap();

    // Verify all updated
    let downloaded_count = db.count_articles_by_status(
        download_id,
        super::article_status::DOWNLOADED
    ).await.unwrap();
    assert_eq!(downloaded_count, 10);

    // Verify all have downloaded_at timestamp
    let all_articles = db.get_articles(download_id).await.unwrap();
    for article in all_articles {
        assert_eq!(article.status, super::article_status::DOWNLOADED);
        assert!(article.downloaded_at.is_some(), "Article {} should have downloaded_at", article.segment_number);
    }

    db.close().await;
}

#[tokio::test]
async fn test_batch_update_mixed_statuses() {
    // Verify batch update handles mixed statuses correctly
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Create download and articles
    let new_download = NewDownload {
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
        size_bytes: 1024 * 5,
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    let articles: Vec<NewArticle> = (0..5).map(|i| NewArticle {
        download_id,
        message_id: format!("<article{}@example.com>", i),
        segment_number: i,
        size_bytes: 1024,
    }).collect();
    db.insert_articles_batch(&articles).await.unwrap();

    let all_articles = db.get_articles(download_id).await.unwrap();

    // Mixed batch: 3 DOWNLOADED, 2 FAILED
    let updates = vec![
        (all_articles[0].id, super::article_status::DOWNLOADED),
        (all_articles[1].id, super::article_status::DOWNLOADED),
        (all_articles[2].id, super::article_status::DOWNLOADED),
        (all_articles[3].id, super::article_status::FAILED),
        (all_articles[4].id, super::article_status::FAILED),
    ];
    db.update_articles_status_batch(&updates).await.unwrap();

    // Verify counts
    let downloaded_count = db.count_articles_by_status(
        download_id,
        super::article_status::DOWNLOADED
    ).await.unwrap();
    assert_eq!(downloaded_count, 3);

    let failed_count = db.count_articles_by_status(
        download_id,
        super::article_status::FAILED
    ).await.unwrap();
    assert_eq!(failed_count, 2);

    // Verify DOWNLOADED articles have timestamp, FAILED articles don't set timestamp
    let all_articles = db.get_articles(download_id).await.unwrap();
    for (i, article) in all_articles.iter().enumerate() {
        if i < 3 {
            assert_eq!(article.status, super::article_status::DOWNLOADED);
            assert!(article.downloaded_at.is_some(), "Downloaded article {} should have timestamp", i);
        } else {
            assert_eq!(article.status, super::article_status::FAILED);
            // FAILED status preserves existing downloaded_at (should be None for new articles)
            assert!(article.downloaded_at.is_none(), "Failed article {} should not have timestamp", i);
        }
    }

    db.close().await;
}

#[tokio::test]
async fn test_batch_update_large_batch() {
    // Verify batch update works efficiently with 100+ articles
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Create download
    let new_download = NewDownload {
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
        size_bytes: 1024 * 150,
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    // Create 150 articles
    let articles: Vec<NewArticle> = (0..150).map(|i| NewArticle {
        download_id,
        message_id: format!("<article{}@example.com>", i),
        segment_number: i,
        size_bytes: 1024,
    }).collect();
    db.insert_articles_batch(&articles).await.unwrap();

    // Get article IDs
    let all_articles = db.get_articles(download_id).await.unwrap();
    assert_eq!(all_articles.len(), 150);

    // Batch update all to DOWNLOADED in single transaction
    let updates: Vec<(i64, i32)> = all_articles.iter()
        .map(|a| (a.id, super::article_status::DOWNLOADED))
        .collect();

    let start = std::time::Instant::now();
    db.update_articles_status_batch(&updates).await.unwrap();
    let batch_duration = start.elapsed();

    // Verify all updated
    let downloaded_count = db.count_articles_by_status(
        download_id,
        super::article_status::DOWNLOADED
    ).await.unwrap();
    assert_eq!(downloaded_count, 150);

    // Performance check: batch update should be fast (< 100ms for 150 updates)
    // This is conservative - actual performance should be much better
    assert!(
        batch_duration.as_millis() < 100,
        "Batch update of 150 articles took {}ms (expected < 100ms)",
        batch_duration.as_millis()
    );

    println!("Batch updated 150 articles in {}ms", batch_duration.as_millis());

    db.close().await;
}

#[tokio::test]
async fn test_batch_update_preserves_downloaded_at_on_non_downloaded_status() {
    // Verify that updating to FAILED preserves existing downloaded_at
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Create download and article
    let new_download = NewDownload {
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
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    let new_article = NewArticle {
        download_id,
        message_id: "<test@example.com>".to_string(),
        segment_number: 1,
        size_bytes: 1024,
    };
    let article_id = db.insert_article(&new_article).await.unwrap();

    // First update to DOWNLOADED (sets timestamp)
    db.update_articles_status_batch(&[(article_id, super::article_status::DOWNLOADED)])
        .await.unwrap();

    let article = db.get_article_by_message_id(download_id, "<test@example.com>")
        .await.unwrap().unwrap();
    let original_timestamp = article.downloaded_at;
    assert!(original_timestamp.is_some());

    // Wait a moment to ensure timestamp would be different
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Update to FAILED (should preserve timestamp)
    db.update_articles_status_batch(&[(article_id, super::article_status::FAILED)])
        .await.unwrap();

    // Verify status changed but timestamp preserved
    let article = db.get_article_by_message_id(download_id, "<test@example.com>")
        .await.unwrap().unwrap();
    assert_eq!(article.status, super::article_status::FAILED);
    assert_eq!(article.downloaded_at, original_timestamp, "Timestamp should be preserved");

    db.close().await;
}

#[tokio::test]
async fn test_batch_update_vs_individual_performance() {
    // Compare batch update vs individual updates performance
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Create download and 100 articles
    let new_download = NewDownload {
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
        size_bytes: 1024 * 100,
    };
    let download_id = db.insert_download(&new_download).await.unwrap();

    let articles: Vec<NewArticle> = (0..100).map(|i| NewArticle {
        download_id,
        message_id: format!("<article{}@example.com>", i),
        segment_number: i,
        size_bytes: 1024,
    }).collect();
    db.insert_articles_batch(&articles).await.unwrap();

    let all_articles = db.get_articles(download_id).await.unwrap();

    // Test individual updates (first 50 articles)
    let individual_start = std::time::Instant::now();
    for article in all_articles.iter().take(50) {
        db.update_article_status(article.id, super::article_status::DOWNLOADED)
            .await.unwrap();
    }
    let individual_duration = individual_start.elapsed();

    // Test batch update (remaining 50 articles)
    let updates: Vec<(i64, i32)> = all_articles.iter()
        .skip(50)
        .map(|a| (a.id, super::article_status::DOWNLOADED))
        .collect();

    let batch_start = std::time::Instant::now();
    db.update_articles_status_batch(&updates).await.unwrap();
    let batch_duration = batch_start.elapsed();

    // Verify all updated
    let downloaded_count = db.count_articles_by_status(
        download_id,
        super::article_status::DOWNLOADED
    ).await.unwrap();
    assert_eq!(downloaded_count, 100);

    // Batch should be significantly faster (at least 10x)
    let speedup = individual_duration.as_micros() as f64 / batch_duration.as_micros() as f64;

    println!("Individual updates (50 articles): {}ms", individual_duration.as_millis());
    println!("Batch update (50 articles): {}ms", batch_duration.as_millis());
    println!("Speedup: {:.1}x", speedup);

    assert!(
        speedup >= 10.0,
        "Batch update should be at least 10x faster than individual updates (got {:.1}x)",
        speedup
    );

    db.close().await;
}
