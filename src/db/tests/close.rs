use crate::db::*;
use crate::types::DownloadId;
use tempfile::NamedTempFile;

/// Verify that querying the database after closing the pool returns an error
/// rather than hanging or panicking.
#[tokio::test]
async fn test_get_download_after_pool_close_returns_error() {
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Insert a download so there's data to query
    let new_download = NewDownload {
        name: "Test".to_string(),
        nzb_path: "/test.nzb".to_string(),
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
    let id = db.insert_download(&new_download).await.unwrap();

    // Verify the download exists before closing
    let before = db.get_download(id).await.unwrap();
    assert!(before.is_some(), "download should exist before close");

    // Close the pool (but keep the Database struct alive)
    db.pool().close().await;

    // Querying after close should return an error, not hang or panic
    let result = db.get_download(id).await;
    assert!(
        result.is_err(),
        "get_download after pool close should return an error, got: {:?}",
        result
    );
}

/// Verify that listing downloads after closing the pool returns an error
#[tokio::test]
async fn test_list_downloads_after_pool_close_returns_error() {
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    db.pool().close().await;

    let result = db.list_downloads().await;
    assert!(
        result.is_err(),
        "list_downloads after pool close should return an error, got: {:?}",
        result
    );
}

/// Verify that inserting a download after closing the pool returns an error
#[tokio::test]
async fn test_insert_download_after_pool_close_returns_error() {
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    db.pool().close().await;

    let new_download = NewDownload {
        name: "After Close".to_string(),
        nzb_path: "/test.nzb".to_string(),
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

    let result = db.insert_download(&new_download).await;
    assert!(
        result.is_err(),
        "insert_download after pool close should return an error, got: {:?}",
        result
    );
}

/// Verify that updating status after closing the pool returns an error
#[tokio::test]
async fn test_update_status_after_pool_close_returns_error() {
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    db.pool().close().await;

    let result = db.update_status(DownloadId(1), 1).await;
    assert!(
        result.is_err(),
        "update_status after pool close should return an error, got: {:?}",
        result
    );
}
