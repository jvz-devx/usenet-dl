use crate::db::*;
use tempfile::NamedTempFile;

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
        post_process: 4,               // UnpackAndCleanup
        priority: 0,                   // Normal
        status: 0,                     // Queued
        size_bytes: 1024 * 1024 * 100, // 100 MB
    };

    let id = db.insert_download(&new_download).await.unwrap();
    assert!(id.0 > 0);

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
            priority: i,
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
    db.update_progress(id, 45.5, 1024 * 1024, 500 * 1024)
        .await
        .unwrap();

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
