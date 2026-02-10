use crate::db::*;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_database_creation() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let db = Database::new(db_path).await.unwrap();

    // Verify tables exist
    let mut conn = db.pool.acquire().await.unwrap();

    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
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
