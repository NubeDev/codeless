use std::str::FromStr;

use codeless_types::{
    AssistantAttachment, AssistantMessage, AssistantMessageRole, AssistantThread, AutoBypassPolicy,
    CostCents, GitAuth, Job, JobStatus, Persona, Repo, Review, ReviewStatus, StageStatus,
    StopReason, Task, TaskId, TaskStatus, Todo, TodoKind, TodoStatus, UnixMillis, WorkspaceMode,
};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

pub(super) fn assistant_thread_from_row(row: SqliteRow) -> sqlx::Result<AssistantThread> {
    let id: String = row.try_get("id")?;
    Ok(AssistantThread {
        id: parse_id(&id)?,
        title: row.try_get("title")?,
        persona_id: row.try_get("persona_id")?,
        created_at: UnixMillis(row.try_get("created_at")?),
        updated_at: UnixMillis(row.try_get("updated_at")?),
    })
}

pub(super) fn assistant_message_from_row(row: SqliteRow) -> sqlx::Result<AssistantMessage> {
    let id: String = row.try_get("id")?;
    let thread_id: String = row.try_get("thread_id")?;
    let role: String = row.try_get("role")?;
    Ok(AssistantMessage {
        id: parse_id(&id)?,
        thread_id: parse_id(&thread_id)?,
        role: parse_assistant_role(&role)?,
        content: row.try_get("content")?,
        meta_json: row.try_get("meta_json")?,
        created_at: UnixMillis(row.try_get("created_at")?),
    })
}

pub(super) fn assistant_attachment_from_row(row: SqliteRow) -> sqlx::Result<AssistantAttachment> {
    let id: String = row.try_get("id")?;
    let thread_id: String = row.try_get("thread_id")?;
    Ok(AssistantAttachment {
        id: parse_id(&id)?,
        thread_id: parse_id(&thread_id)?,
        original_name: row.try_get("original_name")?,
        stored_filename: row.try_get("stored_filename")?,
        mime_type: row.try_get("mime_type")?,
        size_bytes: row.try_get("size_bytes")?,
        created_at: UnixMillis(row.try_get("created_at")?),
    })
}

pub(super) fn assistant_role_label(role: AssistantMessageRole) -> &'static str {
    match role {
        AssistantMessageRole::User => "user",
        AssistantMessageRole::Assistant => "assistant",
        AssistantMessageRole::System => "system",
        AssistantMessageRole::Tool => "tool",
    }
}

fn parse_assistant_role(s: &str) -> sqlx::Result<AssistantMessageRole> {
    Ok(match s {
        "user" => AssistantMessageRole::User,
        "assistant" => AssistantMessageRole::Assistant,
        "system" => AssistantMessageRole::System,
        "tool" => AssistantMessageRole::Tool,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown assistant role: {other}").into(),
            ))
        }
    })
}

pub(super) fn persona_from_row(row: SqliteRow) -> sqlx::Result<Persona> {
    let allowed_raw: String = row.try_get("allowed_subagents")?;
    let snippets_raw: String = row.try_get("default_snippets")?;
    let allowed_tools_raw: String = row.try_get("allowed_tools")?;
    let allowed_subagents: Vec<String> = serde_json::from_str(&allowed_raw).map_err(serde_err)?;
    let default_snippets: Vec<String> = serde_json::from_str(&snippets_raw).map_err(serde_err)?;
    let allowed_tools: Vec<String> = serde_json::from_str(&allowed_tools_raw).map_err(serde_err)?;
    let use_for_jobs: i64 = row.try_get("use_for_jobs")?;
    let built_in: i64 = row.try_get("built_in")?;
    Ok(Persona {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        icon: row.try_get("icon")?,
        instructions: row.try_get("instructions")?,
        use_for_jobs: use_for_jobs != 0,
        default_model: row.try_get("default_model")?,
        allowed_subagents,
        default_snippets,
        allowed_tools,
        default_model_family: row.try_get("default_model_family")?,
        default_attachments_policy: row.try_get("default_attachments_policy")?,
        built_in: built_in != 0,
        created_at: UnixMillis(row.try_get("created_at")?),
        updated_at: UnixMillis(row.try_get("updated_at")?),
    })
}

pub(super) fn repo_from_row(row: SqliteRow) -> sqlx::Result<Repo> {
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

pub(super) fn job_from_row(row: SqliteRow) -> sqlx::Result<Job> {
    let id: String = row.try_get("id")?;
    let repo_id: String = row.try_get("repo_id")?;
    let status: String = row.try_get("status")?;
    let stop_reason: Option<String> = row.try_get("stop_reason")?;
    let workspace_mode: String = row.try_get("workspace_mode")?;
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
        workspace_mode: parse_workspace_mode(&workspace_mode)?,
        worktree_path: row.try_get("worktree_path")?,
        cost_cap_cents: CostCents(row.try_get("cost_cap_cents")?),
        wall_clock_cap_ms: row.try_get("wall_clock_cap_ms")?,
        cost_cents: CostCents(row.try_get("cost_cents")?),
        model: row.try_get("model")?,
        permission_mode: row.try_get("permission_mode")?,
        effort: row.try_get("effort")?,
        system_prompt: row.try_get("system_prompt")?,
        persona_id: row.try_get("persona_id")?,
        auto_bypass_policy: decode_auto_bypass_policy(row.try_get("auto_bypass_policy")?)?,
        pending_operator_comment: row.try_get("pending_operator_comment")?,
        precheck_override_once: row.try_get::<i64, _>("precheck_override_once").unwrap_or(0) != 0,
        started_at: started_at.map(UnixMillis),
        ended_at: ended_at.map(UnixMillis),
        created_at: UnixMillis(row.try_get("created_at")?),
    })
}

/// JSON-encode the policy for storage. The serde-tagged wire form is
/// the same shape the column carries, so the round-trip is the
/// `AutoBypassPolicy`'s own `Serialize` impl. `None` becomes SQL
/// NULL (the column default) so existing rows decode unchanged.
pub(super) fn encode_auto_bypass_policy(
    policy: Option<&AutoBypassPolicy>,
) -> sqlx::Result<Option<String>> {
    policy
        .map(|p| serde_json::to_string(p).map_err(serde_err))
        .transpose()
}

fn decode_auto_bypass_policy(raw: Option<String>) -> sqlx::Result<Option<AutoBypassPolicy>> {
    raw.map(|s| serde_json::from_str::<AutoBypassPolicy>(&s).map_err(serde_err))
        .transpose()
}

pub(super) fn parse_id<T: FromStr>(s: &str) -> sqlx::Result<T>
where
    T::Err: std::fmt::Display,
{
    T::from_str(s).map_err(|e| sqlx::Error::Decode(format!("ulid decode: {e}").into()))
}

pub(super) fn task_from_row(row: SqliteRow) -> sqlx::Result<Task> {
    let id: String = row.try_get("id")?;
    let stage_id: String = row.try_get("stage_id")?;
    let ordinal: i64 = row.try_get("ordinal")?;
    let status: String = row.try_get("status")?;
    let depends_on: String = row.try_get("depends_on")?;
    let depends_on: Vec<TaskId> = serde_json::from_str(&depends_on).map_err(serde_err)?;
    let lease_expires_at: Option<i64> = row.try_get("lease_expires_at")?;
    let started_at: Option<i64> = row.try_get("started_at")?;
    let ended_at: Option<i64> = row.try_get("ended_at")?;
    Ok(Task {
        id: parse_id(&id)?,
        stage_id: parse_id(&stage_id)?,
        ordinal: ordinal as u32,
        status: parse_task_status(&status)?,
        depends_on,
        lease_holder: row.try_get("lease_holder")?,
        lease_expires_at: lease_expires_at.map(UnixMillis),
        cost_cents: CostCents(row.try_get("cost_cents")?),
        input_tokens: row.try_get("input_tokens")?,
        output_tokens: row.try_get("output_tokens")?,
        started_at: started_at.map(UnixMillis),
        ended_at: ended_at.map(UnixMillis),
    })
}

pub(super) fn stage_status_label(s: StageStatus) -> &'static str {
    match s {
        StageStatus::Pending => "pending",
        StageStatus::Running => "running",
        StageStatus::AwaitingReview => "awaiting-review",
        StageStatus::Passed => "passed",
        StageStatus::Failed => "failed",
    }
}

/// Decode the JSON-encoded `stages.acceptance` column. `None` (SQL
/// NULL) and `Some` (a JSON array literal) are kept distinct so the
/// wire round-trip preserves "field omitted" vs. "field set to empty
/// list" — the UI overview reads the empty-list case as "stage has no
/// acceptance criteria yet", which is different from "this stage
/// predates the field".
pub(super) fn parse_acceptance(raw: Option<String>) -> sqlx::Result<Option<Vec<String>>> {
    raw.map(|s| serde_json::from_str::<Vec<String>>(&s).map_err(serde_err))
        .transpose()
}

pub(super) fn parse_stage_status(s: &str) -> StageStatus {
    match s {
        "running" => StageStatus::Running,
        "awaiting-review" => StageStatus::AwaitingReview,
        "passed" => StageStatus::Passed,
        "failed" => StageStatus::Failed,
        _ => StageStatus::Pending,
    }
}

pub(super) fn failure_class_label(c: codeless_types::FailureClass) -> &'static str {
    use codeless_types::FailureClass::*;
    match c {
        PreCheckFailed => "pre-check-failed",
        RunnerError => "runner-error",
        ReviewPatchInvalid => "review-patch-invalid",
        ReviewFail => "review-fail",
        ReviewUnparseable => "review-unparseable",
        OrphanReap => "orphan-reap",
    }
}

pub(super) fn parse_failure_class(s: &str) -> Option<codeless_types::FailureClass> {
    use codeless_types::FailureClass::*;
    match s {
        "pre-check-failed" => Some(PreCheckFailed),
        "runner-error" => Some(RunnerError),
        "review-patch-invalid" => Some(ReviewPatchInvalid),
        "review-fail" => Some(ReviewFail),
        "review-unparseable" => Some(ReviewUnparseable),
        "orphan-reap" => Some(OrphanReap),
        _ => None,
    }
}

pub(super) fn task_status_label(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Enqueued => "enqueued",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

pub(super) fn todo_status_label(s: TodoStatus) -> &'static str {
    match s {
        TodoStatus::Pending => "pending",
        TodoStatus::InProgress => "in-progress",
        TodoStatus::Done => "done",
        TodoStatus::Skipped => "skipped",
        TodoStatus::Failed => "failed",
    }
}

pub(super) fn parse_todo_status(s: &str) -> sqlx::Result<TodoStatus> {
    Ok(match s {
        "pending" => TodoStatus::Pending,
        "in-progress" => TodoStatus::InProgress,
        "done" => TodoStatus::Done,
        "skipped" => TodoStatus::Skipped,
        "failed" => TodoStatus::Failed,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown todo status: {other}").into(),
            ))
        }
    })
}

pub(super) fn todo_kind_label(k: TodoKind) -> &'static str {
    match k {
        TodoKind::Runner => "runner",
        TodoKind::Planner => "planner",
        TodoKind::Checks => "checks",
        TodoKind::Docs => "docs",
        TodoKind::Git => "git",
    }
}

pub(super) fn parse_todo_kind(s: &str) -> sqlx::Result<TodoKind> {
    Ok(match s {
        "runner" => TodoKind::Runner,
        "planner" => TodoKind::Planner,
        "checks" => TodoKind::Checks,
        "docs" => TodoKind::Docs,
        "git" => TodoKind::Git,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown todo kind: {other}").into(),
            ))
        }
    })
}

pub(super) fn todo_from_row(row: SqliteRow) -> sqlx::Result<Todo> {
    let id: String = row.try_get("id")?;
    let task_id: String = row.try_get("task_id")?;
    let status: String = row.try_get("status")?;
    let kind: String = row.try_get("kind")?;
    let ordinal: i64 = row.try_get("ordinal")?;
    let started_at: Option<i64> = row.try_get("started_at")?;
    let ended_at: Option<i64> = row.try_get("ended_at")?;
    let failure_detail: Option<String> = row.try_get("failure_detail")?;
    Ok(Todo {
        id: parse_id(&id)?,
        task_id: parse_id(&task_id)?,
        ordinal: ordinal as u32,
        title: row.try_get("title")?,
        status: parse_todo_status(&status)?,
        kind: parse_todo_kind(&kind)?,
        created_at: UnixMillis(row.try_get("created_at")?),
        started_at: started_at.map(UnixMillis),
        ended_at: ended_at.map(UnixMillis),
        failure_detail,
    })
}

fn parse_task_status(s: &str) -> sqlx::Result<TaskStatus> {
    Ok(match s {
        "enqueued" => TaskStatus::Enqueued,
        "running" => TaskStatus::Running,
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "cancelled" => TaskStatus::Cancelled,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown task status: {other}").into(),
            ))
        }
    })
}

pub(super) fn job_status_label(s: JobStatus) -> &'static str {
    match s {
        JobStatus::Draft => "draft",
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::AwaitingReview => "awaiting-review",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::Stopped => "stopped",
        JobStatus::Paused => "paused",
    }
}

fn parse_job_status(s: &str) -> sqlx::Result<JobStatus> {
    Ok(match s {
        "draft" => JobStatus::Draft,
        "queued" => JobStatus::Queued,
        "running" => JobStatus::Running,
        "awaiting-review" => JobStatus::AwaitingReview,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        "stopped" => JobStatus::Stopped,
        "paused" => JobStatus::Paused,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown job status: {other}").into(),
            ))
        }
    })
}

pub(super) fn stop_reason_label(s: StopReason) -> String {
    match s {
        StopReason::User => "user".into(),
        StopReason::CostCap => "cost-cap".into(),
        StopReason::WallClock => "wall-clock".into(),
        StopReason::RunnerCrash => "runner-crash".into(),
        StopReason::AutoBypassThrashing => "auto-bypass-thrashing".into(),
        StopReason::ReviewPreCheck => "review-pre-check".into(),
        // The scoped variant carries a `PausePointId`, so the
        // SQLite column gets a colon-prefixed form that the parser
        // splits on. The unit-variant shape is unchanged for the
        // existing six values, so old rows still decode.
        StopReason::ScopedPausePoint { point_id } => format!("scoped-pause-point:{point_id}"),
    }
}

pub(super) fn workspace_mode_label(m: WorkspaceMode) -> &'static str {
    match m {
        WorkspaceMode::InRepo => "in-repo",
        WorkspaceMode::Worktree => "worktree",
    }
}

fn parse_workspace_mode(s: &str) -> sqlx::Result<WorkspaceMode> {
    Ok(match s {
        "in-repo" => WorkspaceMode::InRepo,
        "worktree" => WorkspaceMode::Worktree,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown workspace_mode: {other}").into(),
            ))
        }
    })
}

pub(super) fn review_status_label(s: ReviewStatus) -> &'static str {
    match s {
        ReviewStatus::Pending => "pending",
        ReviewStatus::Approved => "approved",
        ReviewStatus::Rejected => "rejected",
        ReviewStatus::Stopped => "stopped",
        ReviewStatus::RerunRequested => "rerun-requested",
    }
}

fn parse_review_status(s: &str) -> sqlx::Result<ReviewStatus> {
    Ok(match s {
        "pending" => ReviewStatus::Pending,
        "approved" => ReviewStatus::Approved,
        "rejected" => ReviewStatus::Rejected,
        "stopped" => ReviewStatus::Stopped,
        "rerun-requested" => ReviewStatus::RerunRequested,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown review status: {other}").into(),
            ))
        }
    })
}

pub(super) fn review_from_row(row: SqliteRow) -> sqlx::Result<Review> {
    let id: String = row.try_get("id")?;
    let stage_id: String = row.try_get("stage_id")?;
    let status: String = row.try_get("status")?;
    let resolved_at: Option<i64> = row.try_get("resolved_at")?;
    Ok(Review {
        id: parse_id(&id)?,
        stage_id: parse_id(&stage_id)?,
        status: parse_review_status(&status)?,
        comment: row.try_get("comment")?,
        requested_at: UnixMillis(row.try_get("requested_at")?),
        resolved_at: resolved_at.map(UnixMillis),
    })
}

fn parse_stop_reason(s: &str) -> sqlx::Result<StopReason> {
    if let Some(rest) = s.strip_prefix("scoped-pause-point:") {
        let point_id = rest.parse::<codeless_types::PausePointId>().map_err(|e| {
            sqlx::Error::Decode(format!("invalid scoped-pause-point ulid {rest:?}: {e}").into())
        })?;
        return Ok(StopReason::ScopedPausePoint { point_id });
    }
    Ok(match s {
        "user" => StopReason::User,
        "cost-cap" => StopReason::CostCap,
        "wall-clock" => StopReason::WallClock,
        "runner-crash" => StopReason::RunnerCrash,
        "auto-bypass-thrashing" => StopReason::AutoBypassThrashing,
        "review-pre-check" => StopReason::ReviewPreCheck,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown stop reason: {other}").into(),
            ))
        }
    })
}

pub(super) fn serde_err(e: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Decode(format!("json: {e}").into())
}
