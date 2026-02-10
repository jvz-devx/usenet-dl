use crate::db::*;
use tempfile::NamedTempFile;

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
        "#,
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
    let filter_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM rss_filters WHERE feed_id = ?")
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
        "#,
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
        "#,
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
        "#,
    )
    .bind(feed_id)
    .bind("item-guid-123")
    .bind(chrono::Utc::now().timestamp())
    .execute(&mut *conn)
    .await;

    assert!(
        result.is_err(),
        "Duplicate insert should fail due to primary key constraint"
    );

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
            "#,
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
            "#,
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
            "#,
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
        sqlx::query_scalar("SELECT COUNT(*) FROM rss_seen WHERE feed_id = ? AND guid = ?")
            .bind(feed_id)
            .bind(guid)
            .fetch_one(&mut *conn)
            .await
            .unwrap()
    };

    assert_eq!(
        count, 1,
        "Should have exactly one record even after multiple marks"
    );

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
            "#,
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
            "#,
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
            "#,
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
