//! RSS feed CRUD operations.

use crate::error::DatabaseError;
use crate::{Error, Result};

use super::{
    Database, InsertRssFeedParams, InsertRssFilterParams, RssFeed, RssFilterRow,
    UpdateRssFeedParams,
};

impl Database {
    /// Get all RSS feeds
    pub async fn get_all_rss_feeds(&self) -> Result<Vec<RssFeed>> {
        let feeds = sqlx::query_as::<_, RssFeed>(
            r#"
            SELECT id, name, url, check_interval_secs, category, auto_download,
                   priority, enabled, last_check, last_error, created_at
            FROM rss_feeds
            ORDER BY id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get RSS feeds: {}",
                e
            )))
        })?;

        Ok(feeds)
    }

    /// Get RSS feed by ID
    pub async fn get_rss_feed(&self, id: i64) -> Result<Option<RssFeed>> {
        let feed = sqlx::query_as::<_, RssFeed>(
            r#"
            SELECT id, name, url, check_interval_secs, category, auto_download,
                   priority, enabled, last_check, last_error, created_at
            FROM rss_feeds
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get RSS feed: {}",
                e
            )))
        })?;

        Ok(feed)
    }

    /// Insert a new RSS feed
    pub async fn insert_rss_feed(&self, params: InsertRssFeedParams<'_>) -> Result<i64> {
        let InsertRssFeedParams {
            name,
            url,
            check_interval_secs,
            category,
            auto_download,
            priority,
            enabled,
        } = params;
        let now = chrono::Utc::now().timestamp();

        let result = sqlx::query(
            r#"
            INSERT INTO rss_feeds (name, url, check_interval_secs, category, auto_download,
                                  priority, enabled, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(name)
        .bind(url)
        .bind(check_interval_secs)
        .bind(category)
        .bind(auto_download as i32)
        .bind(priority)
        .bind(enabled as i32)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to insert RSS feed: {}",
                e
            )))
        })?;

        Ok(result.last_insert_rowid())
    }

    /// Update an existing RSS feed
    pub async fn update_rss_feed(&self, params: UpdateRssFeedParams<'_>) -> Result<bool> {
        let UpdateRssFeedParams {
            id,
            name,
            url,
            check_interval_secs,
            category,
            auto_download,
            priority,
            enabled,
        } = params;
        let result = sqlx::query(
            r#"
            UPDATE rss_feeds
            SET name = ?, url = ?, check_interval_secs = ?, category = ?,
                auto_download = ?, priority = ?, enabled = ?
            WHERE id = ?
            "#,
        )
        .bind(name)
        .bind(url)
        .bind(check_interval_secs)
        .bind(category)
        .bind(auto_download as i32)
        .bind(priority)
        .bind(enabled as i32)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update RSS feed: {}",
                e
            )))
        })?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete an RSS feed (cascades to filters and seen items)
    pub async fn delete_rss_feed(&self, id: i64) -> Result<bool> {
        let result = sqlx::query("DELETE FROM rss_feeds WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to delete RSS feed: {}",
                    e
                )))
            })?;

        Ok(result.rows_affected() > 0)
    }

    /// Get all filters for a specific RSS feed
    pub async fn get_rss_filters(&self, feed_id: i64) -> Result<Vec<RssFilterRow>> {
        let filters = sqlx::query_as::<_, RssFilterRow>(
            r#"
            SELECT id, feed_id, name, include_patterns, exclude_patterns,
                   min_size, max_size, max_age_secs
            FROM rss_filters
            WHERE feed_id = ?
            ORDER BY id ASC
            "#,
        )
        .bind(feed_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get RSS filters: {}",
                e
            )))
        })?;

        Ok(filters)
    }

    /// Insert a new RSS filter
    pub async fn insert_rss_filter(&self, params: InsertRssFilterParams<'_>) -> Result<i64> {
        let InsertRssFilterParams {
            feed_id,
            name,
            include_patterns,
            exclude_patterns,
            min_size,
            max_size,
            max_age_secs,
        } = params;
        let result = sqlx::query(
            r#"
            INSERT INTO rss_filters (feed_id, name, include_patterns, exclude_patterns,
                                    min_size, max_size, max_age_secs)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(feed_id)
        .bind(name)
        .bind(include_patterns)
        .bind(exclude_patterns)
        .bind(min_size)
        .bind(max_size)
        .bind(max_age_secs)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to insert RSS filter: {}",
                e
            )))
        })?;

        Ok(result.last_insert_rowid())
    }

    /// Delete all filters for a feed (used during update)
    pub async fn delete_rss_filters(&self, feed_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM rss_filters WHERE feed_id = ?")
            .bind(feed_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to delete RSS filters: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Update last check time and error for an RSS feed
    pub async fn update_rss_feed_check_status(
        &self,
        id: i64,
        last_error: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            UPDATE rss_feeds
            SET last_check = ?, last_error = ?
            WHERE id = ?
            "#,
        )
        .bind(now)
        .bind(last_error)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update RSS feed status: {}",
                e
            )))
        })?;

        Ok(())
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    /// Helper: create a fresh database with migrations applied
    async fn setup_db() -> (Database, NamedTempFile) {
        let temp_file = NamedTempFile::new().unwrap();
        let db = Database::new(temp_file.path()).await.unwrap();
        (db, temp_file)
    }

    /// Helper: insert a feed with sensible defaults, returning its ID
    async fn insert_test_feed(db: &Database, name: &str, url: &str) -> i64 {
        db.insert_rss_feed(InsertRssFeedParams {
            name,
            url,
            check_interval_secs: 900,
            category: Some("movies"),
            auto_download: true,
            priority: 1,
            enabled: true,
        })
        .await
        .unwrap()
    }

    // ---------------------------------------------------------------
    // insert + get round-trip
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn insert_and_get_feed_preserves_all_fields() {
        let (db, _tmp) = setup_db().await;

        let id = db
            .insert_rss_feed(InsertRssFeedParams {
                name: "Linux ISOs",
                url: "https://example.com/linux.rss",
                check_interval_secs: 1800,
                category: Some("linux"),
                auto_download: false,
                priority: 2,
                enabled: false,
            })
            .await
            .unwrap();

        let feed = db.get_rss_feed(id).await.unwrap().expect("feed must exist");

        assert_eq!(feed.id, id);
        assert_eq!(feed.name, "Linux ISOs");
        assert_eq!(feed.url, "https://example.com/linux.rss");
        assert_eq!(feed.check_interval_secs, 1800);
        assert_eq!(feed.category.as_deref(), Some("linux"));
        assert_eq!(feed.auto_download, 0, "false should be stored as 0");
        assert_eq!(feed.priority, 2);
        assert_eq!(feed.enabled, 0, "false should be stored as 0");
        assert!(
            feed.last_check.is_none(),
            "new feed should have no last_check"
        );
        assert!(
            feed.last_error.is_none(),
            "new feed should have no last_error"
        );
        assert!(
            feed.created_at > 0,
            "created_at should be a positive timestamp"
        );

        db.close().await;
    }

    #[tokio::test]
    async fn insert_feed_with_null_category_stores_none() {
        let (db, _tmp) = setup_db().await;

        let id = db
            .insert_rss_feed(InsertRssFeedParams {
                name: "No Cat",
                url: "https://example.com/nocat.rss",
                check_interval_secs: 600,
                category: None,
                auto_download: true,
                priority: 0,
                enabled: true,
            })
            .await
            .unwrap();

        let feed = db.get_rss_feed(id).await.unwrap().unwrap();
        assert!(
            feed.category.is_none(),
            "None category should persist as NULL"
        );

        db.close().await;
    }

    // ---------------------------------------------------------------
    // get_all_rss_feeds
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn get_all_feeds_returns_all_inserted_in_id_order() {
        let (db, _tmp) = setup_db().await;

        let id1 = insert_test_feed(&db, "Feed A", "https://a.com/rss").await;
        let id2 = insert_test_feed(&db, "Feed B", "https://b.com/rss").await;
        let id3 = insert_test_feed(&db, "Feed C", "https://c.com/rss").await;

        let feeds = db.get_all_rss_feeds().await.unwrap();

        assert_eq!(feeds.len(), 3, "should return all 3 feeds");
        assert_eq!(feeds[0].id, id1);
        assert_eq!(feeds[1].id, id2);
        assert_eq!(feeds[2].id, id3);
        assert_eq!(feeds[0].name, "Feed A");
        assert_eq!(feeds[1].name, "Feed B");
        assert_eq!(feeds[2].name, "Feed C");

        db.close().await;
    }

    #[tokio::test]
    async fn get_all_feeds_returns_empty_vec_when_no_feeds_exist() {
        let (db, _tmp) = setup_db().await;

        let feeds = db.get_all_rss_feeds().await.unwrap();
        assert!(feeds.is_empty(), "empty DB should return empty vec");

        db.close().await;
    }

    // ---------------------------------------------------------------
    // update_rss_feed
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn update_feed_persists_changed_fields() {
        let (db, _tmp) = setup_db().await;

        let id = insert_test_feed(&db, "Original", "https://old.com/rss").await;

        let updated = db
            .update_rss_feed(UpdateRssFeedParams {
                id,
                name: "Renamed",
                url: "https://new.com/rss",
                check_interval_secs: 3600,
                category: Some("tv"),
                auto_download: false,
                priority: -1,
                enabled: false,
            })
            .await
            .unwrap();

        assert!(updated, "update should return true for existing feed");

        let feed = db.get_rss_feed(id).await.unwrap().unwrap();
        assert_eq!(feed.name, "Renamed");
        assert_eq!(feed.url, "https://new.com/rss");
        assert_eq!(feed.check_interval_secs, 3600);
        assert_eq!(feed.category.as_deref(), Some("tv"));
        assert_eq!(feed.auto_download, 0);
        assert_eq!(feed.priority, -1);
        assert_eq!(feed.enabled, 0);

        db.close().await;
    }

    #[tokio::test]
    async fn update_nonexistent_feed_returns_false() {
        let (db, _tmp) = setup_db().await;

        let updated = db
            .update_rss_feed(UpdateRssFeedParams {
                id: 99999,
                name: "Ghost",
                url: "https://ghost.com/rss",
                check_interval_secs: 60,
                category: None,
                auto_download: false,
                priority: 0,
                enabled: false,
            })
            .await
            .unwrap();

        assert!(!updated, "updating a non-existent feed must return false");

        db.close().await;
    }

    // ---------------------------------------------------------------
    // delete_rss_feed
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn delete_feed_removes_it_from_database() {
        let (db, _tmp) = setup_db().await;

        let id = insert_test_feed(&db, "Doomed", "https://doomed.com/rss").await;

        let deleted = db.delete_rss_feed(id).await.unwrap();
        assert!(deleted, "delete should return true for existing feed");

        let gone = db.get_rss_feed(id).await.unwrap();
        assert!(gone.is_none(), "feed should be gone after delete");

        db.close().await;
    }

    #[tokio::test]
    async fn delete_nonexistent_feed_returns_false() {
        let (db, _tmp) = setup_db().await;

        let deleted = db.delete_rss_feed(99999).await.unwrap();
        assert!(
            !deleted,
            "deleting non-existent feed must return false, not error"
        );

        db.close().await;
    }

    #[tokio::test]
    async fn delete_feed_cascades_to_filters() {
        let (db, _tmp) = setup_db().await;

        let feed_id = insert_test_feed(&db, "Cascade Me", "https://cascade.com/rss").await;

        // Add two filters to the feed
        db.insert_rss_filter(InsertRssFilterParams {
            feed_id,
            name: "Filter 1",
            include_patterns: Some(r#"["1080p"]"#),
            exclude_patterns: None,
            min_size: None,
            max_size: None,
            max_age_secs: None,
        })
        .await
        .unwrap();

        db.insert_rss_filter(InsertRssFilterParams {
            feed_id,
            name: "Filter 2",
            include_patterns: None,
            exclude_patterns: Some(r#"["CAM"]"#),
            min_size: None,
            max_size: None,
            max_age_secs: None,
        })
        .await
        .unwrap();

        // Delete the feed
        db.delete_rss_feed(feed_id).await.unwrap();

        // Verify filters are gone too (cascade delete)
        let filters = db.get_rss_filters(feed_id).await.unwrap();
        assert!(
            filters.is_empty(),
            "cascade delete should remove all filters for the deleted feed"
        );

        db.close().await;
    }

    // ---------------------------------------------------------------
    // get_rss_feed edge cases
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn get_nonexistent_feed_returns_none() {
        let (db, _tmp) = setup_db().await;

        let feed = db.get_rss_feed(99999).await.unwrap();
        assert!(feed.is_none(), "non-existent feed ID should return None");

        db.close().await;
    }

    // ---------------------------------------------------------------
    // insert_rss_filter + get_rss_filters
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn insert_and_get_filter_preserves_all_fields() {
        let (db, _tmp) = setup_db().await;

        let feed_id = insert_test_feed(&db, "Filtered", "https://filtered.com/rss").await;

        let filter_id = db
            .insert_rss_filter(InsertRssFilterParams {
                feed_id,
                name: "HD Movies",
                include_patterns: Some(r#"["1080p","2160p"]"#),
                exclude_patterns: Some(r#"["CAM","TS"]"#),
                min_size: Some(500_000_000),
                max_size: Some(50_000_000_000),
                max_age_secs: Some(86400),
            })
            .await
            .unwrap();

        let filters = db.get_rss_filters(feed_id).await.unwrap();
        assert_eq!(filters.len(), 1);

        let f = &filters[0];
        assert_eq!(f.id, filter_id);
        assert_eq!(f.feed_id, feed_id);
        assert_eq!(f.name, "HD Movies");
        assert_eq!(f.include_patterns.as_deref(), Some(r#"["1080p","2160p"]"#));
        assert_eq!(f.exclude_patterns.as_deref(), Some(r#"["CAM","TS"]"#));
        assert_eq!(f.min_size, Some(500_000_000));
        assert_eq!(f.max_size, Some(50_000_000_000));
        assert_eq!(f.max_age_secs, Some(86400));

        db.close().await;
    }

    #[tokio::test]
    async fn insert_filter_with_all_optional_fields_null() {
        let (db, _tmp) = setup_db().await;

        let feed_id = insert_test_feed(&db, "Minimal", "https://min.com/rss").await;

        db.insert_rss_filter(InsertRssFilterParams {
            feed_id,
            name: "Catch All",
            include_patterns: None,
            exclude_patterns: None,
            min_size: None,
            max_size: None,
            max_age_secs: None,
        })
        .await
        .unwrap();

        let filters = db.get_rss_filters(feed_id).await.unwrap();
        assert_eq!(filters.len(), 1);

        let f = &filters[0];
        assert!(f.include_patterns.is_none());
        assert!(f.exclude_patterns.is_none());
        assert!(f.min_size.is_none());
        assert!(f.max_size.is_none());
        assert!(f.max_age_secs.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn get_filters_returns_only_filters_for_requested_feed() {
        let (db, _tmp) = setup_db().await;

        let feed1 = insert_test_feed(&db, "Feed 1", "https://f1.com/rss").await;
        let feed2 = insert_test_feed(&db, "Feed 2", "https://f2.com/rss").await;

        db.insert_rss_filter(InsertRssFilterParams {
            feed_id: feed1,
            name: "Feed1 Filter",
            include_patterns: Some(r#"["x264"]"#),
            exclude_patterns: None,
            min_size: None,
            max_size: None,
            max_age_secs: None,
        })
        .await
        .unwrap();

        db.insert_rss_filter(InsertRssFilterParams {
            feed_id: feed2,
            name: "Feed2 Filter",
            include_patterns: Some(r#"["x265"]"#),
            exclude_patterns: None,
            min_size: None,
            max_size: None,
            max_age_secs: None,
        })
        .await
        .unwrap();

        let filters1 = db.get_rss_filters(feed1).await.unwrap();
        assert_eq!(filters1.len(), 1);
        assert_eq!(filters1[0].name, "Feed1 Filter");

        let filters2 = db.get_rss_filters(feed2).await.unwrap();
        assert_eq!(filters2.len(), 1);
        assert_eq!(filters2[0].name, "Feed2 Filter");

        db.close().await;
    }

    #[tokio::test]
    async fn get_filters_for_feed_with_no_filters_returns_empty() {
        let (db, _tmp) = setup_db().await;

        let feed_id = insert_test_feed(&db, "No Filters", "https://nf.com/rss").await;

        let filters = db.get_rss_filters(feed_id).await.unwrap();
        assert!(filters.is_empty());

        db.close().await;
    }

    // ---------------------------------------------------------------
    // delete_rss_filters
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn delete_filters_removes_all_filters_for_feed() {
        let (db, _tmp) = setup_db().await;

        let feed_id = insert_test_feed(&db, "Multi Filter", "https://mf.com/rss").await;

        for i in 0..3 {
            db.insert_rss_filter(InsertRssFilterParams {
                feed_id,
                name: &format!("Filter {}", i),
                include_patterns: None,
                exclude_patterns: None,
                min_size: None,
                max_size: None,
                max_age_secs: None,
            })
            .await
            .unwrap();
        }

        // Verify 3 filters exist
        assert_eq!(db.get_rss_filters(feed_id).await.unwrap().len(), 3);

        // Delete all filters
        db.delete_rss_filters(feed_id).await.unwrap();

        let remaining = db.get_rss_filters(feed_id).await.unwrap();
        assert!(
            remaining.is_empty(),
            "delete_rss_filters should remove all filters for the feed"
        );

        db.close().await;
    }

    #[tokio::test]
    async fn delete_filters_does_not_affect_other_feeds_filters() {
        let (db, _tmp) = setup_db().await;

        let feed1 = insert_test_feed(&db, "Feed 1", "https://f1.com/rss").await;
        let feed2 = insert_test_feed(&db, "Feed 2", "https://f2.com/rss").await;

        db.insert_rss_filter(InsertRssFilterParams {
            feed_id: feed1,
            name: "Feed1 Filter",
            include_patterns: None,
            exclude_patterns: None,
            min_size: None,
            max_size: None,
            max_age_secs: None,
        })
        .await
        .unwrap();

        db.insert_rss_filter(InsertRssFilterParams {
            feed_id: feed2,
            name: "Feed2 Filter",
            include_patterns: None,
            exclude_patterns: None,
            min_size: None,
            max_size: None,
            max_age_secs: None,
        })
        .await
        .unwrap();

        // Delete only feed1's filters
        db.delete_rss_filters(feed1).await.unwrap();

        let f1_filters = db.get_rss_filters(feed1).await.unwrap();
        assert!(f1_filters.is_empty(), "feed1 filters should be deleted");

        let f2_filters = db.get_rss_filters(feed2).await.unwrap();
        assert_eq!(f2_filters.len(), 1, "feed2 filters should not be affected");
        assert_eq!(f2_filters[0].name, "Feed2 Filter");

        db.close().await;
    }

    #[tokio::test]
    async fn delete_filters_on_feed_with_no_filters_does_not_error() {
        let (db, _tmp) = setup_db().await;

        let feed_id = insert_test_feed(&db, "Empty", "https://empty.com/rss").await;

        // Should succeed even with no filters to delete
        db.delete_rss_filters(feed_id).await.unwrap();

        db.close().await;
    }

    // ---------------------------------------------------------------
    // update_rss_feed_check_status
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn update_check_status_sets_last_check_and_clears_error() {
        let (db, _tmp) = setup_db().await;

        let id = insert_test_feed(&db, "Checked", "https://checked.com/rss").await;

        // Initially no last_check
        let before = db.get_rss_feed(id).await.unwrap().unwrap();
        assert!(before.last_check.is_none());

        // Update with no error (successful check)
        db.update_rss_feed_check_status(id, None).await.unwrap();

        let after = db.get_rss_feed(id).await.unwrap().unwrap();
        assert!(
            after.last_check.is_some(),
            "last_check should be set after status update"
        );
        assert!(
            after.last_check.unwrap() > 0,
            "last_check should be a positive timestamp"
        );
        assert!(
            after.last_error.is_none(),
            "last_error should be None on success"
        );

        db.close().await;
    }

    #[tokio::test]
    async fn update_check_status_stores_error_message() {
        let (db, _tmp) = setup_db().await;

        let id = insert_test_feed(&db, "Errored", "https://errored.com/rss").await;

        db.update_rss_feed_check_status(id, Some("Connection timeout"))
            .await
            .unwrap();

        let feed = db.get_rss_feed(id).await.unwrap().unwrap();
        assert_eq!(
            feed.last_error.as_deref(),
            Some("Connection timeout"),
            "error message should be persisted"
        );
        assert!(
            feed.last_check.is_some(),
            "last_check should also be set on error"
        );

        db.close().await;
    }

    #[tokio::test]
    async fn update_check_status_clears_previous_error_on_success() {
        let (db, _tmp) = setup_db().await;

        let id = insert_test_feed(&db, "Recovery", "https://recovery.com/rss").await;

        // First: fail
        db.update_rss_feed_check_status(id, Some("DNS resolution failed"))
            .await
            .unwrap();
        let errored = db.get_rss_feed(id).await.unwrap().unwrap();
        assert!(errored.last_error.is_some());

        // Then: succeed
        db.update_rss_feed_check_status(id, None).await.unwrap();
        let recovered = db.get_rss_feed(id).await.unwrap().unwrap();
        assert!(
            recovered.last_error.is_none(),
            "successful check should clear previous error"
        );

        db.close().await;
    }

    // ---------------------------------------------------------------
    // Multiple filters ordering
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn get_filters_returns_in_id_order() {
        let (db, _tmp) = setup_db().await;

        let feed_id = insert_test_feed(&db, "Ordered", "https://ord.com/rss").await;

        let id_a = db
            .insert_rss_filter(InsertRssFilterParams {
                feed_id,
                name: "Alpha",
                include_patterns: None,
                exclude_patterns: None,
                min_size: None,
                max_size: None,
                max_age_secs: None,
            })
            .await
            .unwrap();

        let id_b = db
            .insert_rss_filter(InsertRssFilterParams {
                feed_id,
                name: "Beta",
                include_patterns: None,
                exclude_patterns: None,
                min_size: None,
                max_size: None,
                max_age_secs: None,
            })
            .await
            .unwrap();

        let filters = db.get_rss_filters(feed_id).await.unwrap();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].id, id_a, "first filter should have lower ID");
        assert_eq!(filters[1].id, id_b, "second filter should have higher ID");
        assert_eq!(filters[0].name, "Alpha");
        assert_eq!(filters[1].name, "Beta");

        db.close().await;
    }
}
