use codeless_types::{Repo, RepoId};

use super::codec::{repo_from_row, serde_err};
use super::SqliteStore;

impl SqliteStore {
    pub async fn insert_repo(&self, repo: &Repo) -> sqlx::Result<()> {
        let git_auth = serde_json::to_string(&repo.git_auth).map_err(serde_err)?;
        sqlx::query(
            "INSERT INTO repos \
             (id, name, clone_url, default_branch, local_path, git_auth, \
              concurrency_cap, default_runner, created_at, updated_at) \
             VALUES (?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(repo.id.to_string())
        .bind(&repo.name)
        .bind(&repo.clone_url)
        .bind(&repo.default_branch)
        .bind(&repo.local_path)
        .bind(&git_auth)
        .bind(repo.concurrency_cap)
        .bind(&repo.default_runner)
        .bind(repo.created_at.0)
        .bind(repo.updated_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_repo(&self, id: RepoId) -> sqlx::Result<Option<Repo>> {
        let row = sqlx::query("SELECT * FROM repos WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(repo_from_row).transpose()
    }

    /// Returns `true` when a row was removed, `false` when no row
    /// existed. FK violations (a repo with live jobs) surface as
    /// `sqlx::Error` — the wire contract is `ON DELETE RESTRICT`
    /// (Appendix A) and we do not auto-cascade.
    pub async fn remove_repo(&self, id: RepoId) -> sqlx::Result<bool> {
        let res = sqlx::query("DELETE FROM repos WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn list_repos(&self) -> sqlx::Result<Vec<Repo>> {
        let rows = sqlx::query("SELECT * FROM repos ORDER BY created_at")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(repo_from_row).collect()
    }
}
