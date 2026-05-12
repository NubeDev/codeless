use std::str::FromStr;

use codeless_types::{
    CostCents, GitAuth, Job, JobId, JobStatus, Repo, RepoId, StopReason, UnixMillis,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

/// SQLite-backed persistence for repos and jobs. Status enums are
/// mapped to their kebab-case wire labels (matching SCOPE.md Appendix
/// A) by explicit pattern match — the labels are wire-stable, so a
/// drift here is a wire-format break, not a refactor.
#[derive(Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

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

    pub async fn insert_job(&self, job: &Job) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO jobs \
             (id, repo_id, status, stop_reason, template_yaml, prompt, runner, branch, \
              worktree_path, cost_cap_cents, wall_clock_cap_ms, cost_cents, \
              started_at, ended_at, created_at) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(job.id.to_string())
        .bind(job.repo_id.to_string())
        .bind(job_status_label(job.status))
        .bind(job.stop_reason.map(stop_reason_label))
        .bind(&job.template_yaml)
        .bind(&job.prompt)
        .bind(&job.runner)
        .bind(&job.branch)
        .bind(&job.worktree_path)
        .bind(job.cost_cap_cents.0)
        .bind(job.wall_clock_cap_ms)
        .bind(job.cost_cents.0)
        .bind(job.started_at.map(|t| t.0))
        .bind(job.ended_at.map(|t| t.0))
        .bind(job.created_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_job(&self, id: JobId) -> sqlx::Result<Option<Job>> {
        let row = sqlx::query("SELECT * FROM jobs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(job_from_row).transpose()
    }

    /// Whole-row update by primary key. Returns `true` when a row
    /// was updated.
    pub async fn update_job(&self, job: &Job) -> sqlx::Result<bool> {
        let res = sqlx::query(
            "UPDATE jobs SET \
                repo_id=?, status=?, stop_reason=?, template_yaml=?, prompt=?, runner=?, \
                branch=?, worktree_path=?, cost_cap_cents=?, wall_clock_cap_ms=?, \
                cost_cents=?, started_at=?, ended_at=?, created_at=? \
             WHERE id=?",
        )
        .bind(job.repo_id.to_string())
        .bind(job_status_label(job.status))
        .bind(job.stop_reason.map(stop_reason_label))
        .bind(&job.template_yaml)
        .bind(&job.prompt)
        .bind(&job.runner)
        .bind(&job.branch)
        .bind(&job.worktree_path)
        .bind(job.cost_cap_cents.0)
        .bind(job.wall_clock_cap_ms)
        .bind(job.cost_cents.0)
        .bind(job.started_at.map(|t| t.0))
        .bind(job.ended_at.map(|t| t.0))
        .bind(job.created_at.0)
        .bind(job.id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn list_jobs(&self, repo: Option<RepoId>) -> sqlx::Result<Vec<Job>> {
        let rows = match repo {
            Some(r) => {
                sqlx::query("SELECT * FROM jobs WHERE repo_id = ? ORDER BY created_at")
                    .bind(r.to_string())
                    .fetch_all(&self.pool)
                    .await?
            }
            None => {
                sqlx::query("SELECT * FROM jobs ORDER BY created_at")
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        rows.into_iter().map(job_from_row).collect()
    }
}

fn repo_from_row(row: SqliteRow) -> sqlx::Result<Repo> {
    let id: String = row.try_get("id")?;
    let git_auth: String = row.try_get("git_auth")?;
    let git_auth: GitAuth = serde_json::from_str(&git_auth).map_err(serde_err)?;
    Ok(Repo {
        id: parse_id(&id)?,
        name: row.try_get("name")?,
        clone_url: row.try_get("clone_url")?,
        default_branch: row.try_get("default_branch")?,
        local_path: row.try_get("local_path")?,
        git_auth,
        concurrency_cap: row.try_get("concurrency_cap")?,
        default_runner: row.try_get("default_runner")?,
        created_at: UnixMillis(row.try_get("created_at")?),
        updated_at: UnixMillis(row.try_get("updated_at")?),
    })
}

fn job_from_row(row: SqliteRow) -> sqlx::Result<Job> {
    let id: String = row.try_get("id")?;
    let repo_id: String = row.try_get("repo_id")?;
    let status: String = row.try_get("status")?;
    let stop_reason: Option<String> = row.try_get("stop_reason")?;
    let started_at: Option<i64> = row.try_get("started_at")?;
    let ended_at: Option<i64> = row.try_get("ended_at")?;
    Ok(Job {
        id: parse_id(&id)?,
        repo_id: parse_id(&repo_id)?,
        status: parse_job_status(&status)?,
        stop_reason: stop_reason.as_deref().map(parse_stop_reason).transpose()?,
        template_yaml: row.try_get("template_yaml")?,
        prompt: row.try_get("prompt")?,
        runner: row.try_get("runner")?,
        branch: row.try_get("branch")?,
        worktree_path: row.try_get("worktree_path")?,
        cost_cap_cents: CostCents(row.try_get("cost_cap_cents")?),
        wall_clock_cap_ms: row.try_get("wall_clock_cap_ms")?,
        cost_cents: CostCents(row.try_get("cost_cents")?),
        started_at: started_at.map(UnixMillis),
        ended_at: ended_at.map(UnixMillis),
        created_at: UnixMillis(row.try_get("created_at")?),
    })
}

fn parse_id<T: FromStr>(s: &str) -> sqlx::Result<T>
where
    T::Err: std::fmt::Display,
{
    T::from_str(s).map_err(|e| sqlx::Error::Decode(format!("ulid decode: {e}").into()))
}

fn job_status_label(s: JobStatus) -> &'static str {
    match s {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::AwaitingReview => "awaiting-review",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::Stopped => "stopped",
    }
}

fn parse_job_status(s: &str) -> sqlx::Result<JobStatus> {
    Ok(match s {
        "queued" => JobStatus::Queued,
        "running" => JobStatus::Running,
        "awaiting-review" => JobStatus::AwaitingReview,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        "stopped" => JobStatus::Stopped,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown job status: {other}").into(),
            ))
        }
    })
}

fn stop_reason_label(s: StopReason) -> &'static str {
    match s {
        StopReason::User => "user",
        StopReason::CostCap => "cost-cap",
        StopReason::WallClock => "wall-clock",
        StopReason::RunnerCrash => "runner-crash",
    }
}

fn parse_stop_reason(s: &str) -> sqlx::Result<StopReason> {
    Ok(match s {
        "user" => StopReason::User,
        "cost-cap" => StopReason::CostCap,
        "wall-clock" => StopReason::WallClock,
        "runner-crash" => StopReason::RunnerCrash,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown stop reason: {other}").into(),
            ))
        }
    })
}

fn serde_err(e: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Decode(format!("json: {e}").into())
}
