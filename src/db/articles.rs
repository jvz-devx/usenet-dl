//! Article-level tracking operations for download resume support.

use crate::error::DatabaseError;
use crate::types::DownloadId;
use crate::{Error, Result};

use super::{article_status, Article, Database, NewArticle};

impl Database {
    /// Insert a single article
    pub async fn insert_article(&self, article: &NewArticle) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO download_articles (
                download_id, message_id, segment_number, size_bytes, status
            ) VALUES (?, ?, ?, ?, 0)
            "#,
        )
        .bind(article.download_id)
        .bind(&article.message_id)
        .bind(article.segment_number)
        .bind(article.size_bytes)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to insert article: {}",
                e
            )))
        })?;

        Ok(result.last_insert_rowid())
    }

    /// Insert multiple articles in a batch (more efficient for large NZB files)
    pub async fn insert_articles_batch(&self, articles: &[NewArticle]) -> Result<()> {
        if articles.is_empty() {
            return Ok(());
        }

        // Build a multi-row insert query
        let mut query_builder = sqlx::QueryBuilder::new(
            "INSERT INTO download_articles (download_id, message_id, segment_number, size_bytes, status) "
        );

        query_builder.push_values(articles, |mut b, article| {
            b.push_bind(article.download_id)
                .push_bind(&article.message_id)
                .push_bind(article.segment_number)
                .push_bind(article.size_bytes)
                .push_bind(0); // status = PENDING
        });

        let query = query_builder.build();
        query.execute(&self.pool).await.map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to insert articles batch: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Update article status
    pub async fn update_article_status(&self, article_id: i64, status: i32) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            UPDATE download_articles
            SET status = ?, downloaded_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status)
        .bind(if status == article_status::DOWNLOADED {
            Some(now)
        } else {
            None
        })
        .bind(article_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update article status: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Update article status by message_id
    pub async fn update_article_status_by_message_id(
        &self,
        download_id: DownloadId,
        message_id: &str,
        status: i32,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            UPDATE download_articles
            SET status = ?, downloaded_at = ?
            WHERE download_id = ? AND message_id = ?
            "#,
        )
        .bind(status)
        .bind(if status == article_status::DOWNLOADED {
            Some(now)
        } else {
            None
        })
        .bind(download_id)
        .bind(message_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update article status: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Update multiple article statuses in a single transaction (more efficient for batch operations)
    ///
    /// # Arguments
    /// * `updates` - Vector of tuples containing (article_id, status)
    ///
    /// # Performance
    /// This method uses a CASE-WHEN statement to update multiple rows in a single query,
    /// which is significantly faster than individual UPDATE statements. With 100 updates,
    /// this can be 50-100x faster than calling `update_article_status` 100 times.
    ///
    /// # Example
    /// ```rust,ignore
    /// let updates = vec![
    ///     (123, article_status::DOWNLOADED),
    ///     (124, article_status::DOWNLOADED),
    ///     (125, article_status::FAILED),
    /// ];
    /// db.update_articles_status_batch(&updates).await?;
    /// ```
    pub async fn update_articles_status_batch(&self, updates: &[(i64, i32)]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }

        let now = chrono::Utc::now().timestamp();

        let mut query_builder =
            sqlx::QueryBuilder::new("UPDATE download_articles SET status = CASE ");

        // Build status CASE clause
        for (article_id, status) in updates {
            query_builder.push("WHEN id = ");
            query_builder.push_bind(*article_id);
            query_builder.push(" THEN ");
            query_builder.push_bind(*status);
            query_builder.push(" ");
        }
        query_builder.push("END, downloaded_at = CASE ");

        // Build downloaded_at CASE clause (only set timestamp for DOWNLOADED status)
        for (article_id, status) in updates {
            query_builder.push("WHEN id = ");
            query_builder.push_bind(*article_id);
            if *status == article_status::DOWNLOADED {
                query_builder.push(" THEN ");
                query_builder.push_bind(now);
            } else {
                query_builder.push(" THEN downloaded_at"); // Keep existing value
            }
            query_builder.push(" ");
        }
        query_builder.push("END WHERE id IN (");

        // Build WHERE IN clause
        let mut first = true;
        for (article_id, _) in updates {
            if !first {
                query_builder.push(", ");
            }
            query_builder.push_bind(*article_id);
            first = false;
        }
        query_builder.push(")");

        let query = query_builder.build();
        query.execute(&self.pool).await.map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update articles status batch: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Get all articles for a download
    pub async fn get_articles(&self, download_id: DownloadId) -> Result<Vec<Article>> {
        let rows = sqlx::query_as::<_, Article>(
            r#"
            SELECT id, download_id, message_id, segment_number, size_bytes, status, downloaded_at
            FROM download_articles
            WHERE download_id = ?
            ORDER BY segment_number ASC
            "#,
        )
        .bind(download_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get articles: {}",
                e
            )))
        })?;

        Ok(rows)
    }

    /// Get pending articles for a download (for resume)
    pub async fn get_pending_articles(&self, download_id: DownloadId) -> Result<Vec<Article>> {
        let rows = sqlx::query_as::<_, Article>(
            r#"
            SELECT id, download_id, message_id, segment_number, size_bytes, status, downloaded_at
            FROM download_articles
            WHERE download_id = ? AND status = 0
            ORDER BY segment_number ASC
            "#,
        )
        .bind(download_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get pending articles: {}",
                e
            )))
        })?;

        Ok(rows)
    }

    /// Get article by message_id
    pub async fn get_article_by_message_id(
        &self,
        download_id: DownloadId,
        message_id: &str,
    ) -> Result<Option<Article>> {
        let row = sqlx::query_as::<_, Article>(
            r#"
            SELECT id, download_id, message_id, segment_number, size_bytes, status, downloaded_at
            FROM download_articles
            WHERE download_id = ? AND message_id = ?
            "#,
        )
        .bind(download_id)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get article: {}",
                e
            )))
        })?;

        Ok(row)
    }

    /// Count articles by status for a download
    pub async fn count_articles_by_status(
        &self,
        download_id: DownloadId,
        status: i32,
    ) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM download_articles WHERE download_id = ? AND status = ?",
        )
        .bind(download_id)
        .bind(status)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to count articles: {}",
                e
            )))
        })?;

        Ok(count)
    }

    /// Get total article count for a download
    pub async fn count_articles(&self, download_id: DownloadId) -> Result<i64> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM download_articles WHERE download_id = ?")
                .bind(download_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    Error::Database(DatabaseError::QueryFailed(format!(
                        "Failed to count articles: {}",
                        e
                    )))
                })?;

        Ok(count)
    }

    /// Delete all articles for a download (automatic via CASCADE, but explicit method for clarity)
    pub async fn delete_articles(&self, download_id: DownloadId) -> Result<()> {
        sqlx::query("DELETE FROM download_articles WHERE download_id = ?")
            .bind(download_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to delete articles: {}",
                    e
                )))
            })?;

        Ok(())
    }
}
