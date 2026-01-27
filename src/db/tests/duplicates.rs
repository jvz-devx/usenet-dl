use crate::db::*;
use tempfile::NamedTempFile;

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
        status: 4,                          // Complete
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
    assert_eq!(
        by_hash.as_ref().unwrap().name,
        by_name.as_ref().unwrap().name
    );
    assert_eq!(
        by_hash.as_ref().unwrap().category,
        by_job.as_ref().unwrap().category
    );
    assert_eq!(by_hash.as_ref().unwrap().status, 4);

    db.close().await;
}
