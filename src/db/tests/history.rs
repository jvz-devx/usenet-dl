use crate::db::*;
use crate::types::Status;
use std::path::PathBuf;
use tempfile::NamedTempFile;

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
        status: 4,                          // Complete
        size_bytes: 5 * 1024 * 1024 * 1024, // 5 GB
        download_time_secs: 3600,           // 1 hour
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
    assert_eq!(
        retrieved.destination,
        Some(PathBuf::from("/downloads/movies/Test.Movie.2024"))
    );
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
    let timestamps = [now - 1000, now, now - 500];
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
