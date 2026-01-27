use crate::db::*;
use tempfile::NamedTempFile;

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
    db.set_correct_password(download_id, "secret123")
        .await
        .unwrap();

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
    db.set_correct_password(download_id, "password1")
        .await
        .unwrap();

    // Update to new password (should use ON CONFLICT to update)
    db.set_correct_password(download_id, "password2")
        .await
        .unwrap();

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
    db.set_correct_password(download_id, "secret")
        .await
        .unwrap();

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
    db.set_correct_password(download_id, "password123")
        .await
        .unwrap();

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
