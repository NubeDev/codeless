use codeless_types::{JobId, Review, ReviewId, ReviewStatus, StageId};

use super::codec::{review_from_row, review_status_label};
use super::SqliteStore;

impl SqliteStore {
    pub async fn insert_review(&self, review: &Review) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO reviews (id, stage_id, status, comment, requested_at, resolved_at) \
             VALUES (?,?,?,?,?,?)",
        )
        .bind(review.id.to_string())
        .bind(review.stage_id.to_string())
        .bind(review_status_label(review.status))
        .bind(&review.comment)
        .bind(review.requested_at.0)
        .bind(review.resolved_at.map(|t| t.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_review(&self, id: ReviewId) -> sqlx::Result<Option<Review>> {
        let row = sqlx::query("SELECT * FROM reviews WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(review_from_row).transpose()
    }

    pub async fn update_review(&self, review: &Review) -> sqlx::Result<()> {
        sqlx::query("UPDATE reviews SET status = ?, comment = ?, resolved_at = ? WHERE id = ?")
            .bind(review_status_label(review.status))
            .bind(&review.comment)
            .bind(review.resolved_at.map(|t| t.0))
            .bind(review.id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List reviews, optionally narrowed by job, stage, or status. The
    /// filters compose with AND. Ordered by `requested_at` so the UI
    /// gets a stable oldest-first list. The job filter joins through
    /// `stages` so the per-job review panel does not need to map stages
    /// to jobs client-side.
    pub async fn list_reviews(
        &self,
        job_id: Option<JobId>,
        stage_id: Option<StageId>,
        status: Option<ReviewStatus>,
    ) -> sqlx::Result<Vec<Review>> {
        let status_label = status.map(review_status_label);
        let job_str = job_id.map(|j| j.to_string());
        let stage_str = stage_id.map(|s| s.to_string());
        let rows = sqlx::query(
            "SELECT reviews.* FROM reviews \
             LEFT JOIN stages ON stages.id = reviews.stage_id \
             WHERE (? IS NULL OR stages.job_id = ?) \
               AND (? IS NULL OR reviews.stage_id = ?) \
               AND (? IS NULL OR reviews.status = ?) \
             ORDER BY reviews.requested_at",
        )
        .bind(&job_str)
        .bind(&job_str)
        .bind(&stage_str)
        .bind(&stage_str)
        .bind(status_label)
        .bind(status_label)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(review_from_row).collect()
    }
}
