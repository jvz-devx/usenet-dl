use crate::db::*;
use tempfile::NamedTempFile;

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
        file_index: 0,
        size_bytes: 512 * 1024,
    };
    let article_id = db.insert_article(&new_article).await.unwrap();
    assert!(article_id > 0);

    // Get the article
    let article = db
        .get_article_by_message_id(download_id, "<test@example.com>")
        .await
        .unwrap();
    assert!(article.is_some());

    let article = article.unwrap();
    assert_eq!(article.download_id, download_id);
    assert_eq!(article.message_id, "<test@example.com>");
    assert_eq!(article.segment_number, 1);
    assert_eq!(article.size_bytes, 512 * 1024);
    assert_eq!(article.status, article_status::PENDING);
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
    let articles: Vec<NewArticle> = (0..100)
        .map(|i| NewArticle {
            download_id,
            message_id: format!("<article{}@example.com>", i),
            segment_number: i,
            file_index: 0,
            size_bytes: 10240,
        })
        .collect();

    db.insert_articles_batch(&articles).await.unwrap();

    // Verify all articles were inserted
    let count = db.count_articles(download_id).await.unwrap();
    assert_eq!(count, 100);

    // Verify they're all pending
    let pending_count = db
        .count_articles_by_status(download_id, article_status::PENDING)
        .await
        .unwrap();
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
        file_index: 0,
        size_bytes: 1024,
    };
    let article_id = db.insert_article(&new_article).await.unwrap();

    // Update status to DOWNLOADED
    db.update_article_status(article_id, article_status::DOWNLOADED)
        .await
        .unwrap();

    // Verify status was updated
    let article = db
        .get_article_by_message_id(download_id, "<test@example.com>")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(article.status, article_status::DOWNLOADED);
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
    let articles: Vec<NewArticle> = (0..10)
        .map(|i| NewArticle {
            download_id,
            message_id: format!("<article{}@example.com>", i),
            segment_number: i,
            file_index: 0,
            size_bytes: 1024,
        })
        .collect();
    db.insert_articles_batch(&articles).await.unwrap();

    // Mark some as downloaded
    for i in 0..5 {
        db.update_article_status_by_message_id(
            download_id,
            &format!("<article{}@example.com>", i),
            article_status::DOWNLOADED,
        )
        .await
        .unwrap();
    }

    // Mark one as failed
    db.update_article_status_by_message_id(
        download_id,
        "<article5@example.com>",
        article_status::FAILED,
    )
    .await
    .unwrap();

    // Get pending articles (should be 4 remaining: 6, 7, 8, 9)
    let pending = db.get_pending_articles(download_id).await.unwrap();
    assert_eq!(pending.len(), 4);
    assert_eq!(pending[0].segment_number, 6);
    assert_eq!(pending[1].segment_number, 7);
    assert_eq!(pending[2].segment_number, 8);
    assert_eq!(pending[3].segment_number, 9);

    // Verify counts
    let downloaded_count = db
        .count_articles_by_status(download_id, article_status::DOWNLOADED)
        .await
        .unwrap();
    assert_eq!(downloaded_count, 5);

    let failed_count = db
        .count_articles_by_status(download_id, article_status::FAILED)
        .await
        .unwrap();
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
    let articles: Vec<NewArticle> = (0..5)
        .map(|i| NewArticle {
            download_id,
            message_id: format!("<article{}@example.com>", i),
            segment_number: i,
            file_index: 0,
            size_bytes: 1024,
        })
        .collect();
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
        file_index: 0,
        size_bytes: 1024,
    };
    let article_id = db.insert_article(&new_article).await.unwrap();

    // Batch update single article to DOWNLOADED
    let updates = vec![(article_id, article_status::DOWNLOADED)];
    db.update_articles_status_batch(&updates).await.unwrap();

    // Verify status updated
    let article = db
        .get_article_by_message_id(download_id, "<test@example.com>")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(article.status, article_status::DOWNLOADED);
    assert!(
        article.downloaded_at.is_some(),
        "downloaded_at should be set"
    );

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
    let articles: Vec<NewArticle> = (0..10)
        .map(|i| NewArticle {
            download_id,
            message_id: format!("<article{}@example.com>", i),
            segment_number: i,
            file_index: 0,
            size_bytes: 1024,
        })
        .collect();
    db.insert_articles_batch(&articles).await.unwrap();

    // Get article IDs
    let all_articles = db.get_articles(download_id).await.unwrap();
    assert_eq!(all_articles.len(), 10);

    // Batch update all to DOWNLOADED
    let updates: Vec<(i64, i32)> = all_articles
        .iter()
        .map(|a| (a.id, article_status::DOWNLOADED))
        .collect();
    db.update_articles_status_batch(&updates).await.unwrap();

    // Verify all updated
    let downloaded_count = db
        .count_articles_by_status(download_id, article_status::DOWNLOADED)
        .await
        .unwrap();
    assert_eq!(downloaded_count, 10);

    // Verify all have downloaded_at timestamp
    let all_articles = db.get_articles(download_id).await.unwrap();
    for article in all_articles {
        assert_eq!(article.status, article_status::DOWNLOADED);
        assert!(
            article.downloaded_at.is_some(),
            "Article {} should have downloaded_at",
            article.segment_number
        );
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

    let articles: Vec<NewArticle> = (0..5)
        .map(|i| NewArticle {
            download_id,
            message_id: format!("<article{}@example.com>", i),
            segment_number: i,
            file_index: 0,
            size_bytes: 1024,
        })
        .collect();
    db.insert_articles_batch(&articles).await.unwrap();

    let all_articles = db.get_articles(download_id).await.unwrap();

    // Mixed batch: 3 DOWNLOADED, 2 FAILED
    let updates = vec![
        (all_articles[0].id, article_status::DOWNLOADED),
        (all_articles[1].id, article_status::DOWNLOADED),
        (all_articles[2].id, article_status::DOWNLOADED),
        (all_articles[3].id, article_status::FAILED),
        (all_articles[4].id, article_status::FAILED),
    ];
    db.update_articles_status_batch(&updates).await.unwrap();

    // Verify counts
    let downloaded_count = db
        .count_articles_by_status(download_id, article_status::DOWNLOADED)
        .await
        .unwrap();
    assert_eq!(downloaded_count, 3);

    let failed_count = db
        .count_articles_by_status(download_id, article_status::FAILED)
        .await
        .unwrap();
    assert_eq!(failed_count, 2);

    // Verify DOWNLOADED articles have timestamp, FAILED articles don't set timestamp
    let all_articles = db.get_articles(download_id).await.unwrap();
    for (i, article) in all_articles.iter().enumerate() {
        if i < 3 {
            assert_eq!(article.status, article_status::DOWNLOADED);
            assert!(
                article.downloaded_at.is_some(),
                "Downloaded article {} should have timestamp",
                i
            );
        } else {
            assert_eq!(article.status, article_status::FAILED);
            // FAILED status preserves existing downloaded_at (should be None for new articles)
            assert!(
                article.downloaded_at.is_none(),
                "Failed article {} should not have timestamp",
                i
            );
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
    let articles: Vec<NewArticle> = (0..150)
        .map(|i| NewArticle {
            download_id,
            message_id: format!("<article{}@example.com>", i),
            segment_number: i,
            file_index: 0,
            size_bytes: 1024,
        })
        .collect();
    db.insert_articles_batch(&articles).await.unwrap();

    // Get article IDs
    let all_articles = db.get_articles(download_id).await.unwrap();
    assert_eq!(all_articles.len(), 150);

    // Batch update all to DOWNLOADED in single transaction
    let updates: Vec<(i64, i32)> = all_articles
        .iter()
        .map(|a| (a.id, article_status::DOWNLOADED))
        .collect();

    let start = std::time::Instant::now();
    db.update_articles_status_batch(&updates).await.unwrap();
    let batch_duration = start.elapsed();

    // Verify all updated
    let downloaded_count = db
        .count_articles_by_status(download_id, article_status::DOWNLOADED)
        .await
        .unwrap();
    assert_eq!(downloaded_count, 150);

    // Performance check: batch update should be fast (< 100ms for 150 updates)
    // This is conservative - actual performance should be much better
    assert!(
        batch_duration.as_millis() < 100,
        "Batch update of 150 articles took {}ms (expected < 100ms)",
        batch_duration.as_millis()
    );

    println!(
        "Batch updated 150 articles in {}ms",
        batch_duration.as_millis()
    );

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
        file_index: 0,
        size_bytes: 1024,
    };
    let article_id = db.insert_article(&new_article).await.unwrap();

    // First update to DOWNLOADED (sets timestamp)
    db.update_articles_status_batch(&[(article_id, article_status::DOWNLOADED)])
        .await
        .unwrap();

    let article = db
        .get_article_by_message_id(download_id, "<test@example.com>")
        .await
        .unwrap()
        .unwrap();
    let original_timestamp = article.downloaded_at;
    assert!(original_timestamp.is_some());

    // Wait a moment to ensure timestamp would be different
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Update to FAILED (should preserve timestamp)
    db.update_articles_status_batch(&[(article_id, article_status::FAILED)])
        .await
        .unwrap();

    // Verify status changed but timestamp preserved
    let article = db
        .get_article_by_message_id(download_id, "<test@example.com>")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(article.status, article_status::FAILED);
    assert_eq!(
        article.downloaded_at, original_timestamp,
        "Timestamp should be preserved"
    );

    db.close().await;
}
