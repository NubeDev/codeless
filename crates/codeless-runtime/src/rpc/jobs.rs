use std::collections::HashMap;
use std::sync::Arc;

use codeless_adapters_host::WorktreeManager;
use codeless_rpc::{
    DraftJobFromConversationArgs, GcWorktreeEntry, GcWorktreesArgs, GcWorktreesResult, GetJobArgs,
    JobReportArgs, JobReportEventTally, JobReportResult, JobReportSpecChange, JobReportStage,
    JobReportToolCall, JobReportTurn, ListJobsArgs, ListJobsResult, ListStagesArgs,
    ListStagesResult, PauseJobArgs, RerunJobArgs, ResetJobArgs, ResumeJobArgs, RpcError, RpcResult,
    SetJobPolicyArgs, StartJobArgs, StopJobArgs, SubmitJobArgs, UpdateJobScopeArgs,
    UpdateJobScopeResult, WriteJobFileArgs,
};
use codeless_types::{
    AssistantAction, AssistantActionCard, AssistantActionStatus, AssistantMessageRole, CostCents,
    Event, Job, JobId, JobStatus, StageStatus, StopReason,
};
use sqlx::Row;

use super::InProcessRpc;
use crate::template::JobTemplate;
use crate::time::now_ms;

pub(super) async fn submit_job(rpc: &InProcessRpc, args: SubmitJobArgs) -> RpcResult<Job> {
    let repo = rpc
        .store
        .get_repo(args.repo_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("repo {}", args.repo_id)))?;
    let now = now_ms();

    // Enforce the one-in_repo-per-repo invariant. A second in_repo
    // job against the same repo would fight over the working copy.
    let mode = args.workspace_mode.unwrap_or_default();
    if mode == codeless_types::WorkspaceMode::InRepo {
        if let Some(existing) = rpc
            .store
            .active_in_repo_job(args.repo_id)
            .await
            .map_err(super::db_err)?
        {
            return Err(RpcError::Conflict(format!(
                "repo {} is already in use by job {} in in_repo mode; \
                 stop it or submit as worktree",
                args.repo_id, existing.id,
            )));
        }
    }

    // If the submit carries a template that parses into the canonical
    // `JobTemplate` shape, scaffold the on-disk job directory before
    // the Job row lands. CLI submits whose YAML is the wrapper format
    // and prompt-only submits fall through unscaffolded.
    if let Some(template_src) = args.template_yaml.as_deref() {
        if let Ok(template) = JobTemplate::parse_yaml(template_src) {
            // Per-stage persona override (D1): every `stage.persona`
            // id must resolve against the `personas` table before
            // the job row lands. Failing here keeps the failure
            // visible at the submit boundary — a missing id never
            // reaches the runner, where it would silently degrade
            // to the inherited persona. The lookup uses the same
            // `id` column the chat panel quotes (`builtin:<slug>`
            // for seeded rows, the user-minted id for user rows).
            for (idx, stage) in template.stages.iter().enumerate() {
                if let Some(persona_id) = stage.persona.as_deref() {
                    let row = rpc
                        .store
                        .get_persona(persona_id)
                        .await
                        .map_err(super::db_err)?;
                    if row.is_none() {
                        return Err(RpcError::InvalidArgument(format!(
                            "stage {} (`{}`) references persona `{}`, \
                             which does not exist",
                            idx + 1,
                            stage.title,
                            persona_id,
                        )));
                    }
                }
            }
            super::job_files::seed_job_directory(&repo.local_path, template_src)?;
        }
    }

    // Default landing state is `Draft` so the user can edit spec/docs
    // before the driver picks the job up. `start_immediately` is the
    // legacy/power-user path that skips the draft and queues immediately.
    let initial_status = if args.start_immediately {
        JobStatus::Queued
    } else {
        JobStatus::Draft
    };
    let job = Job {
        id: JobId::new(),
        repo_id: args.repo_id,
        status: initial_status,
        stop_reason: None,
        template_yaml: args.template_yaml,
        prompt: args.prompt,
        runner: args.runner,
        branch: args.branch,
        workspace_mode: args.workspace_mode.unwrap_or_default(),
        worktree_path: None,
        cost_cap_cents: CostCents(args.cost_cap_cents),
        wall_clock_cap_ms: args.wall_clock_cap_ms,
        cost_cents: CostCents::ZERO,
        model: args.model,
        permission_mode: args.permission_mode,
        effort: args.effort,
        // The UI composes this from the selected persona's
        // `instructions` and passes it through verbatim. Server-side
        // persona resolution lands in a later stage; until then the
        // composed text round-trips on the row so a resume reproduces
        // the same agent posture the user picked at submit time.
        system_prompt: args.system_prompt.filter(|s| !s.is_empty()),
        // Persist the persona id even when the caller did not also
        // send a composed `system_prompt`: a future stage will move
        // composition server-side and the id is the durable handle.
        // Empty strings collapse to `None` so the column stays a
        // clean optional regardless of how the UI shaped the payload.
        persona_id: args.persona_id.filter(|s| !s.is_empty()),
        auto_bypass_policy: args.auto_bypass_policy,
        pending_operator_comment: None,
        started_at: None,
        ended_at: None,
        created_at: now,
    };
    rpc.store.insert_job(&job).await.map_err(super::db_err)?;
    rpc.bus
        .publish(
            Some(job.id),
            None,
            None,
            Event::JobQueued {
                job_id: job.id,
                repo_id: job.repo_id,
            },
            now,
        )
        .await
        .map_err(super::db_err)?;
    Ok(job)
}

pub(super) async fn start_job(rpc: &InProcessRpc, args: StartJobArgs) -> RpcResult<Job> {
    let mut job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    if job.status != JobStatus::Draft {
        return Err(RpcError::Conflict(format!(
            "job {} is {:?}, not Draft — only Draft jobs can be started",
            job.id, job.status
        )));
    }
    resync_template_from_disk(rpc, &mut job).await?;
    crate::state_machine::transition_job(job.status, JobStatus::Queued).map_err(|e| {
        RpcError::Conflict(format!(
            "illegal job transition from {:?} to Queued: {e}",
            job.status
        ))
    })?;
    job.status = JobStatus::Queued;
    if !rpc.store.update_job(&job).await.map_err(super::db_err)? {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    }
    // Reuse the long-defined-but-never-emitted `JobPromoted` variant for
    // Draft → Queued. The dashboard maps it to "running" optimistically.
    rpc.bus
        .publish(
            Some(job.id),
            None,
            None,
            Event::JobPromoted { job_id: job.id },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(job)
}

pub(super) async fn resume_job(rpc: &InProcessRpc, args: ResumeJobArgs) -> RpcResult<Job> {
    let mut job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    if !matches!(
        job.status,
        JobStatus::Stopped | JobStatus::Failed | JobStatus::Paused
    ) {
        return Err(RpcError::Conflict(format!(
            "job {} is {:?}; only Stopped, Failed, or Paused jobs are \
             resumable. Use stop_job or pause_job to interrupt a running job.",
            job.id, job.status
        )));
    }
    resync_template_from_disk(rpc, &mut job).await?;
    // Bypass-on-resume: when the caller set `bypass`, find the most
    // recently failed stage on this job and stamp its `bypassed_at`
    // so the next run of `TemplateRunner` advances past it. The
    // status column stays `Failed` so history is honest.
    if args.bypass {
        let stages = rpc
            .store
            .list_stages_for_job(job.id)
            .await
            .map_err(super::db_err)?;
        let target = stages
            .iter()
            .filter(|s| matches!(s.stage.status, StageStatus::Failed))
            .filter(|s| s.stage.bypassed_at.is_none())
            .max_by_key(|s| s.stage.ordinal);
        if let Some(s) = target {
            let now = crate::time::now_ms();
            rpc.store
                .mark_stage_bypassed(s.stage.id, now, "operator bypass via resume_job")
                .await
                .map_err(super::db_err)?;
            tracing::info!(
                job_id = %job.id,
                stage_id = %s.stage.id,
                ordinal = s.stage.ordinal,
                "resume_job: bypassed failed stage at operator request",
            );
        } else {
            tracing::warn!(
                job_id = %job.id,
                "resume_job: bypass set but no Failed stage without an existing bypass found; resume proceeds without bypass",
            );
        }
    }
    // `next_stage_comment` lands on the job row's
    // `pending_operator_comment` slot. The runner factory consumes
    // and clears it atomically when building the next runner, so
    // the comment threads into exactly the stage the operator wrote
    // it for and a later resume without a fresh comment does not
    // re-apply stale text. Empty string is normalised to clear.
    {
        let normalised = args
            .next_stage_comment
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        rpc.store
            .set_pending_operator_comment(job.id, normalised)
            .await
            .map_err(super::db_err)?;
        if let Some(c) = normalised {
            tracing::info!(
                job_id = %job.id,
                comment_len = c.len(),
                "resume_job: stashed pending_operator_comment for next runner build",
            );
        }
    }
    crate::state_machine::transition_job(job.status, JobStatus::Queued).map_err(|e| {
        RpcError::Conflict(format!(
            "illegal job transition from {:?} to Queued: {e}",
            job.status
        ))
    })?;
    // Cap bumps are additive. Saturating add so a huge number doesn't
    // overflow the SQLite-side i64 into a negative cap the watcher trips.
    let previous_reason = job.stop_reason;
    if let Some(bump) = args.additional_cost_cap_cents {
        if bump > 0 {
            job.cost_cap_cents = CostCents(job.cost_cap_cents.0.saturating_add(bump));
        }
    }
    if let Some(bump) = args.additional_wall_clock_cap_ms {
        if bump > 0 {
            job.wall_clock_cap_ms = job.wall_clock_cap_ms.saturating_add(bump);
        }
    }
    job.status = JobStatus::Queued;
    // Clearing `stop_reason` would erase the original outcome from the
    // row; `previous_reason` rides on `JobResumed` for history instead.
    job.stop_reason = None;
    // `ended_at` clears — the job is live again. The captured worktree
    // path, branch, and per-stage `session_id` values are untouched.
    job.ended_at = None;
    if !rpc.store.update_job(&job).await.map_err(super::db_err)? {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    }
    rpc.bus
        .publish(
            Some(job.id),
            None,
            None,
            Event::JobResumed {
                job_id: job.id,
                previous_reason,
                // The actor that initiated the resume is set by the
                // calling surface (e.g. the Slack adapter sets
                // `"slack"`). The base RPC has no surface context so
                // it publishes `None` here; richer call sites layer
                // their own envelope through a future
                // `ResumeJobArgs.actor` field or a server-side
                // middleware.
                actor: None,
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(job)
}

pub(super) async fn get_job(rpc: &InProcessRpc, args: GetJobArgs) -> RpcResult<Job> {
    let mut job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    let before = job.template_yaml.clone();
    if let Err(e) = resync_template_from_disk(rpc, &mut job).await {
        tracing::warn!(job_id = %job.id, error = %e, "get_job: template resync skipped");
    } else if job.template_yaml != before {
        rpc.store.update_job(&job).await.map_err(super::db_err)?;
    }
    Ok(job)
}

pub(super) async fn list_jobs(rpc: &InProcessRpc, args: ListJobsArgs) -> RpcResult<ListJobsResult> {
    Ok(ListJobsResult {
        jobs: rpc
            .store
            .list_jobs(args.repo_id)
            .await
            .map_err(super::db_err)?,
    })
}

pub(super) async fn list_stages(
    rpc: &InProcessRpc,
    args: ListStagesArgs,
) -> RpcResult<ListStagesResult> {
    let rows = rpc
        .store
        .list_stages_for_job(args.job_id)
        .await
        .map_err(super::db_err)?;
    let stages = rows
        .into_iter()
        .map(|row| codeless_rpc::StageRollup {
            stage: row.stage,
            cost_cents: row.cost_cents,
            task_count: row.task_count,
        })
        .collect();
    Ok(ListStagesResult { stages })
}

pub(super) async fn job_report(
    rpc: &InProcessRpc,
    args: JobReportArgs,
) -> RpcResult<JobReportResult> {
    let job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    let job_id_s = args.job_id.to_string();
    let pool = rpc.store.pool();

    // Stage rows in chronological order so a stage that was retried
    // (cost-cap → resume) shows two entries for the same ordinal.
    let stage_rows = sqlx::query(
        "SELECT ordinal, name, status, session_id, started_at, ended_at \
         FROM stages WHERE job_id = ? ORDER BY COALESCE(started_at, 0)",
    )
    .bind(&job_id_s)
    .fetch_all(pool)
    .await
    .map_err(super::db_err)?;

    let mut attempt_seen: HashMap<u32, u32> = HashMap::new();
    let mut stages: Vec<JobReportStage> = Vec::with_capacity(stage_rows.len());
    for r in stage_rows {
        let ordinal = r.try_get::<i64, _>("ordinal").map_err(super::db_err)? as u32;
        let attempt = attempt_seen.entry(ordinal).or_insert(0);
        let started_at: Option<i64> = r.try_get("started_at").map_err(super::db_err)?;
        let ended_at: Option<i64> = r.try_get("ended_at").map_err(super::db_err)?;
        stages.push(JobReportStage {
            ordinal,
            attempt: *attempt,
            title: r.try_get("name").map_err(super::db_err)?,
            status: r.try_get("status").map_err(super::db_err)?,
            session_id: r.try_get("session_id").map_err(super::db_err)?,
            // Filled below from turn buckets so a stage without a task row
            // still gets the right number.
            cost_cents: 0,
            duration_ms: match (started_at, ended_at) {
                (Some(s), Some(e)) => Some(e - s),
                _ => None,
            },
            started_at,
            ended_at,
        });
        *attempt += 1;
    }

    // ai-message-complete = one Claude reply. Cost lives in the payload;
    // bucket turns into stages by timestamp window.
    let turn_rows = sqlx::query(
        "SELECT task_id, payload, created_at FROM events \
         WHERE job_id = ? AND type = 'ai-message-complete' \
         ORDER BY created_at",
    )
    .bind(&job_id_s)
    .fetch_all(pool)
    .await
    .map_err(super::db_err)?;

    let mut turns: Vec<JobReportTurn> = Vec::with_capacity(turn_rows.len());
    for r in turn_rows {
        let task_id: Option<String> = r.try_get("task_id").map_err(super::db_err)?;
        let payload: String = r.try_get("payload").map_err(super::db_err)?;
        let at: i64 = r.try_get("created_at").map_err(super::db_err)?;
        let v: serde_json::Value =
            serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null);
        let cost_cents = v.get("cost_cents").and_then(|x| x.as_i64()).unwrap_or(0);
        let input_tokens = v.get("input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
        let output_tokens = v.get("output_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
        // The *latest* matching stage window wins for retried attempts.
        let stage_ordinal = stages
            .iter()
            .rev()
            .find(|s| match (s.started_at, s.ended_at) {
                (Some(start), Some(end)) => at >= start && at <= end,
                (Some(start), None) => at >= start,
                _ => false,
            })
            .map(|s| s.ordinal);
        turns.push(JobReportTurn {
            task_id: task_id.unwrap_or_default(),
            stage_ordinal,
            cost_cents,
            input_tokens,
            output_tokens,
            at,
        });
    }

    // Fold per-turn cost back into the matching stage attempt.
    for turn in &turns {
        if let Some(ord) = turn.stage_ordinal {
            if let Some(target) = stages.iter_mut().rev().find(|s| {
                s.ordinal == ord
                    && match (s.started_at, s.ended_at) {
                        (Some(start), Some(end)) => turn.at >= start && turn.at <= end,
                        (Some(start), None) => turn.at >= start,
                        _ => false,
                    }
            }) {
                target.cost_cents += turn.cost_cents;
            }
        }
    }

    let tool_rows = sqlx::query(
        "SELECT COALESCE(json_extract(payload, '$.tool'), '<unknown>') AS tool, \
                COUNT(*) AS n \
         FROM events WHERE job_id = ? AND type = 'tool-call' \
         GROUP BY tool ORDER BY n DESC",
    )
    .bind(&job_id_s)
    .fetch_all(pool)
    .await
    .map_err(super::db_err)?;
    let tool_calls: Vec<JobReportToolCall> = tool_rows
        .into_iter()
        .map(|r| {
            Ok::<_, sqlx::Error>(JobReportToolCall {
                tool: r.try_get("tool")?,
                count: r.try_get::<i64, _>("n")? as u32,
            })
        })
        .collect::<Result<_, _>>()
        .map_err(super::db_err)?;

    let tally_rows = sqlx::query(
        "SELECT type AS kind, COUNT(*) AS n FROM events WHERE job_id = ? \
         GROUP BY type ORDER BY n DESC",
    )
    .bind(&job_id_s)
    .fetch_all(pool)
    .await
    .map_err(super::db_err)?;
    let event_tally: Vec<JobReportEventTally> = tally_rows
        .into_iter()
        .map(|r| {
            Ok::<_, sqlx::Error>(JobReportEventTally {
                kind: r.try_get("kind")?,
                count: r.try_get::<i64, _>("n")? as u32,
            })
        })
        .collect::<Result<_, _>>()
        .map_err(super::db_err)?;

    // Bucket spec-edit events by file. `JobTemplateUpdated` lands under
    // `kind: "template"`; `JobFileUpdated` lands under `kind: "file"`.
    let spec_rows = sqlx::query(
        "SELECT type AS kind, \
                json_extract(payload, '$.filename') AS filename, \
                COUNT(*) AS n, \
                MAX(created_at) AS last_at \
         FROM events \
         WHERE job_id = ? AND type IN ('job-template-updated', 'job-file-updated') \
         GROUP BY type, filename \
         ORDER BY last_at DESC",
    )
    .bind(&job_id_s)
    .fetch_all(pool)
    .await
    .map_err(super::db_err)?;
    let spec_changes: Vec<JobReportSpecChange> = spec_rows
        .into_iter()
        .map(|r| {
            let raw_kind: String = r.try_get("kind")?;
            let kind = match raw_kind.as_str() {
                "job-template-updated" => "template".to_owned(),
                "job-file-updated" => "file".to_owned(),
                other => other.to_owned(),
            };
            Ok::<_, sqlx::Error>(JobReportSpecChange {
                kind,
                filename: r.try_get("filename")?,
                count: r.try_get::<i64, _>("n")? as u32,
                last_at: r.try_get("last_at")?,
            })
        })
        .collect::<Result<_, _>>()
        .map_err(super::db_err)?;

    let started_at = job.started_at.map(|t| t.0);
    let ended_at = job.ended_at.map(|t| t.0);
    let wall_clock_ms = match (started_at, ended_at) {
        (Some(s), Some(e)) => Some(e - s),
        _ => None,
    };

    Ok(JobReportResult {
        job_id: args.job_id,
        status: format!("{:?}", job.status).to_lowercase(),
        stop_reason: job.stop_reason.map(|r| format!("{:?}", r).to_lowercase()),
        cost_cents: job.cost_cents.0,
        cost_cap_cents: job.cost_cap_cents.0,
        started_at,
        ended_at,
        wall_clock_ms,
        stages,
        turns,
        tool_calls,
        event_tally,
        spec_changes,
    })
}

pub(super) async fn stop_job(rpc: &InProcessRpc, args: StopJobArgs) -> RpcResult<()> {
    let Some(mut job) = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
    else {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    };
    match job.status {
        JobStatus::Completed | JobStatus::Failed | JobStatus::Stopped => {
            return Err(RpcError::Conflict(format!(
                "job {} is already terminal ({:?})",
                job.id, job.status
            )));
        }
        _ => {}
    }
    let now = now_ms();
    job.status = JobStatus::Stopped;
    job.stop_reason = Some(StopReason::User);
    job.ended_at = Some(now);
    rpc.store.update_job(&job).await.map_err(super::db_err)?;
    rpc.bus
        .publish(
            Some(job.id),
            None,
            None,
            Event::JobStopped {
                job_id: job.id,
                reason: StopReason::User,
            },
            now,
        )
        .await
        .map_err(super::db_err)?;
    Ok(())
}

pub(super) async fn update_job_fields(
    rpc: &InProcessRpc,
    args: codeless_rpc::UpdateJobArgs,
) -> RpcResult<Job> {
    let mut job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    match job.status {
        JobStatus::Draft | JobStatus::Stopped | JobStatus::Failed | JobStatus::Completed => {}
        _ => {
            return Err(RpcError::Conflict(format!(
                "job {} is {:?}; only Draft or terminal jobs can be edited",
                job.id, job.status
            )));
        }
    }
    if let Some(v) = args.runner {
        job.runner = v;
    }
    if let Some(v) = args.model {
        job.model = if v.is_empty() { None } else { Some(v) };
    }
    if let Some(v) = args.permission_mode {
        job.permission_mode = if v.is_empty() { None } else { Some(v) };
    }
    if let Some(v) = args.effort {
        job.effort = if v.is_empty() { None } else { Some(v) };
    }
    if let Some(v) = args.cost_cap_cents {
        job.cost_cap_cents = codeless_types::CostCents(v);
    }
    if let Some(v) = args.wall_clock_cap_ms {
        job.wall_clock_cap_ms = v;
    }
    if let Some(v) = args.branch {
        job.branch = v;
    }
    if !rpc.store.update_job(&job).await.map_err(super::db_err)? {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    }
    Ok(job)
}

pub(super) async fn delete_job(
    rpc: &InProcessRpc,
    args: codeless_rpc::DeleteJobArgs,
) -> RpcResult<()> {
    let job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    if matches!(job.status, JobStatus::Running | JobStatus::Queued) {
        return Err(RpcError::Conflict(format!(
            "job {} is {:?}; stop it before deleting",
            job.id, job.status
        )));
    }
    if !rpc
        .store
        .delete_job(args.job_id)
        .await
        .map_err(super::db_err)?
    {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    }
    Ok(())
}

pub(super) async fn pause_job(rpc: &InProcessRpc, args: PauseJobArgs) -> RpcResult<()> {
    let Some(mut job) = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
    else {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    };
    if !matches!(job.status, JobStatus::Running | JobStatus::AwaitingReview) {
        return Err(RpcError::Conflict(format!(
            "job {} is {:?}; only Running or AwaitingReview jobs can be paused. \
             Use start_job to promote a Draft, or resume_job to restart a paused/stopped row.",
            job.id, job.status
        )));
    }
    crate::state_machine::transition_job(job.status, JobStatus::Paused).map_err(|e| {
        RpcError::Conflict(format!(
            "illegal job transition from {:?} to Paused: {e}",
            job.status
        ))
    })?;
    let now = now_ms();
    job.status = JobStatus::Paused;
    job.stop_reason = Some(StopReason::User);
    job.ended_at = Some(now);
    rpc.store.update_job(&job).await.map_err(super::db_err)?;
    // The cap-watcher subscribes to the bus and fires the runner's
    // cancellation token when it sees `JobPaused` it didn't author.
    rpc.bus
        .publish(
            Some(job.id),
            None,
            None,
            Event::JobPaused {
                job_id: job.id,
                reason: StopReason::User,
            },
            now,
        )
        .await
        .map_err(super::db_err)?;
    Ok(())
}

pub(super) async fn set_job_policy(rpc: &InProcessRpc, args: SetJobPolicyArgs) -> RpcResult<()> {
    // Q5 in DOCS/AUTO-BYPASS-DECISIONS.md: only let the operator move
    // the policy while the row is not racing the stage-failed handler.
    // Queued is rejected for the same reason Running is — the scheduler
    // may flip Queued -> Running between the policy read and the policy
    // write. Completed is intentionally rejected here too: the stage
    // description pins the permitted set to Draft / Stopped / Paused so
    // the audit trail can never grow a post-hoc policy change after the
    // job is closed out.
    let Some(mut job) = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
    else {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    };
    match job.status {
        JobStatus::Running => {
            return Err(RpcError::Conflict(
                "job is Running; pause before changing the auto-bypass policy".into(),
            ));
        }
        JobStatus::Queued => {
            return Err(RpcError::Conflict(
                "job is Queued; pause before changing the auto-bypass policy".into(),
            ));
        }
        JobStatus::Draft | JobStatus::Stopped | JobStatus::Paused => {}
        other => {
            return Err(RpcError::Conflict(format!(
                "job is {other:?}; auto-bypass policy can only be set on Draft, Stopped, or Paused jobs",
            )));
        }
    }

    // Idempotency clause from Q5: same-policy-set is a no-op success
    // that emits no event. Cross-window subscribers therefore only see
    // traffic on a real change, and the UI can call defensively.
    if job.auto_bypass_policy == args.policy {
        return Ok(());
    }

    let policy_name = args.policy.as_ref().map(|p| p.policy_name().to_string());
    job.auto_bypass_policy = args.policy;
    if !rpc.store.update_job(&job).await.map_err(super::db_err)? {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    }
    rpc.bus
        .publish(
            Some(job.id),
            None,
            None,
            Event::JobPolicyChanged {
                job_id: job.id,
                policy_name,
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(())
}

pub(super) async fn reset_job(rpc: &InProcessRpc, args: ResetJobArgs) -> RpcResult<Job> {
    let mut job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    let previous_status = job.status;
    if !matches!(
        job.status,
        JobStatus::Queued | JobStatus::Failed | JobStatus::Stopped
    ) {
        return Err(RpcError::Conflict(format!(
            "job {} is {:?}; only Queued, Failed, or Stopped jobs can be reset. \
             Use stop_job / pause_job / resume_job for live or paused rows.",
            job.id, job.status
        )));
    }
    crate::state_machine::transition_job(job.status, JobStatus::Draft).map_err(|e| {
        RpcError::Conflict(format!(
            "illegal job transition from {:?} to Draft: {e}",
            job.status
        ))
    })?;

    // Best-effort worktree reap. A `Queued` row that never made it to
    // Running may have no worktree at all (the whole reason reset is
    // needed); a row with `worktree_path` set may still have on-disk
    // state from a partial provision. Either way, failure to reap is
    // logged but not surfaced — the user is already in recovery mode
    // and a hard error would re-wedge the row they are trying to free.
    if let (Some(path), Some(worktrees)) = (job.worktree_path.clone(), rpc.worktrees.clone()) {
        if let Ok(Some(repo)) = rpc.store.get_repo(job.repo_id).await {
            let repo_path = std::path::PathBuf::from(repo.local_path);
            let wt_path = std::path::PathBuf::from(&path);
            let res =
                tokio::task::spawn_blocking(move || worktrees.remove(&repo_path, &wt_path)).await;
            match res {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::warn!(
                    job_id = %job.id,
                    worktree = %path,
                    error = %e,
                    "reset_job: worktree remove failed, continuing"
                ),
                Err(e) => tracing::warn!(
                    job_id = %job.id,
                    worktree = %path,
                    error = %e,
                    "reset_job: worktree remove join failed, continuing"
                ),
            }
        }
    }

    job.status = JobStatus::Draft;
    job.worktree_path = None;
    job.stop_reason = None;
    job.ended_at = None;
    if !rpc.store.update_job(&job).await.map_err(super::db_err)? {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    }
    rpc.bus
        .publish(
            Some(job.id),
            None,
            None,
            Event::JobReset {
                job_id: job.id,
                previous_status,
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(job)
}

pub(super) async fn rerun_job(rpc: &InProcessRpc, args: RerunJobArgs) -> RpcResult<Job> {
    let Some(source) = rpc
        .store
        .get_job(args.source_job_id)
        .await
        .map_err(super::db_err)?
    else {
        return Err(RpcError::NotFound(format!("job {}", args.source_job_id)));
    };
    let now = now_ms();
    // Empty branch makes `WorktreeManager` fall back to `codeless/job-<new_id>`
    // so a rerun never collides with the source job's branch.
    let job = Job {
        id: JobId::new(),
        repo_id: source.repo_id,
        status: JobStatus::Draft,
        stop_reason: None,
        template_yaml: source.template_yaml,
        prompt: source.prompt,
        runner: source.runner,
        branch: String::new(),
        workspace_mode: source.workspace_mode,
        worktree_path: None,
        cost_cap_cents: source.cost_cap_cents,
        wall_clock_cap_ms: source.wall_clock_cap_ms,
        cost_cents: CostCents::ZERO,
        model: source.model,
        permission_mode: source.permission_mode,
        effort: source.effort,
        system_prompt: source.system_prompt,
        persona_id: source.persona_id,
        auto_bypass_policy: source.auto_bypass_policy,
        pending_operator_comment: None,
        started_at: None,
        ended_at: None,
        created_at: now,
    };
    rpc.store.insert_job(&job).await.map_err(super::db_err)?;
    rpc.bus
        .publish(
            Some(job.id),
            None,
            None,
            Event::JobQueued {
                job_id: job.id,
                repo_id: job.repo_id,
            },
            now,
        )
        .await
        .map_err(super::db_err)?;
    Ok(job)
}

pub(super) async fn gc_worktrees(
    rpc: &InProcessRpc,
    args: GcWorktreesArgs,
) -> RpcResult<GcWorktreesResult> {
    let Some(worktrees) = rpc.worktrees.clone() else {
        return Err(RpcError::Internal(
            "gc_worktrees: no worktree root configured on the server".into(),
        ));
    };
    let manager = worktrees.clone();
    let on_disk = tokio::task::spawn_blocking(move || manager.list_on_disk())
        .await
        .map_err(|e| RpcError::Internal(format!("gc list join: {e}")))?
        .map_err(|e| RpcError::Internal(format!("gc list: {e}")))?;
    let root = worktrees.base().to_string_lossy().into_owned();

    let now_i64: i64 = now_ms().as_i64();
    let cutoff = args.older_than_ms.map(|d| now_i64.saturating_sub(d.max(0)));
    let id_filter: Option<std::collections::HashSet<String>> = args
        .job_ids
        .as_ref()
        .map(|ids| ids.iter().map(|id| id.to_string()).collect());

    let mut entries: Vec<GcWorktreeEntry> = Vec::with_capacity(on_disk.len());
    let mut total: i64 = 0;
    let mut removed: i64 = 0;

    for entry in on_disk {
        if let Some(set) = &id_filter {
            if !set.contains(&entry.job_id) {
                continue;
            }
        }
        if let Some(c) = cutoff {
            let mtime = entry.mtime_ms.unwrap_or(now_i64);
            if mtime > c {
                continue;
            }
        }
        total = total.saturating_add(entry.size_bytes);

        // Parse the directory's job_id back to a `JobId`. If parsing
        // fails (stray `job-foo` dir) the entry still surfaces — just
        // without a typed id and without an automatic remove.
        let parsed_id: Option<codeless_types::JobId> = entry.job_id.parse().ok();

        let mut gc_entry = GcWorktreeEntry {
            job_id: parsed_id,
            path: entry.path.to_string_lossy().into_owned(),
            size_bytes: entry.size_bytes,
            mtime_ms: entry.mtime_ms,
            removed: false,
            error: None,
        };

        if !args.dry_run {
            match remove_one_worktree(rpc, &worktrees, &gc_entry, &entry.path).await {
                Ok(()) => {
                    gc_entry.removed = true;
                    removed += 1;
                }
                Err(e) => {
                    gc_entry.error = Some(e);
                }
            }
        }

        entries.push(gc_entry);
    }

    Ok(GcWorktreesResult {
        entries,
        total_size_bytes: total,
        removed_count: removed,
        root: Some(root),
    })
}

/// Remove one worktree referenced by a GC entry. Resolves the source
/// repo path via the job row when the entry's directory name parses as
/// a `JobId`; falls back to a plain directory removal for stray entries.
async fn remove_one_worktree(
    rpc: &InProcessRpc,
    manager: &Arc<WorktreeManager>,
    entry: &GcWorktreeEntry,
    path: &std::path::Path,
) -> Result<(), String> {
    let repo_path: Option<std::path::PathBuf> = if let Some(jid) = entry.job_id {
        let job = rpc
            .store
            .get_job(jid)
            .await
            .map_err(|e| format!("db: {e}"))?;
        let job = job.ok_or_else(|| format!("job {jid} not in store"))?;
        let repo = rpc
            .store
            .get_repo(job.repo_id)
            .await
            .map_err(|e| format!("db: {e}"))?;
        repo.map(|r| std::path::PathBuf::from(r.local_path))
    } else {
        None
    };
    let manager = Arc::clone(manager);
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || match repo_path {
        Some(rp) => manager.remove(&rp, &path).map_err(|e| e.to_string()),
        None => std::fs::remove_dir_all(&path).map_err(|e| e.to_string()),
    })
    .await
    .map_err(|e| format!("join: {e}"))?
}

/// Re-read `template.yaml` from disk and refresh the job's DB column
/// when it differs. Called from `start_job` and `resume_job` so
/// chat-driven filesystem edits land in SQLite before the driver reads
/// the template.
///
/// No-op when the job has no on-disk `template.yaml`, when DB and disk
/// match, or when the job has no `template_yaml` mirror yet. A `name:`
/// field that no longer matches the recorded name is `Conflict` —
/// renames are rejected by `update_job_template` too, so chat edits
/// must not bypass that rule by writing to disk.
pub(super) async fn resync_template_from_disk(
    rpc: &InProcessRpc,
    job: &mut codeless_types::Job,
) -> RpcResult<()> {
    let Some(db_yaml) = job.template_yaml.clone() else {
        return Ok(());
    };
    let prev = JobTemplate::parse_yaml(&db_yaml)
        .map_err(|e| RpcError::Internal(format!("job {} stored template parse: {e}", job.id)))?;

    let repo = rpc
        .store
        .get_repo(job.repo_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("repo {}", job.repo_id)))?;
    let repo_path = std::path::PathBuf::from(&repo.local_path);
    let tpl_path = crate::job_dir::template_yaml_path(&repo_path, &prev.name);

    let disk_yaml = match std::fs::read_to_string(&tpl_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(RpcError::Internal(format!(
                "read {}: {e}",
                tpl_path.display()
            )));
        }
    };
    if disk_yaml == db_yaml {
        return Ok(());
    }

    let parsed = JobTemplate::parse_yaml(&disk_yaml).map_err(|e| {
        RpcError::InvalidArgument(format!(
            "{} on disk does not parse: {e}",
            tpl_path.display()
        ))
    })?;
    if parsed.name != prev.name {
        return Err(RpcError::Conflict(format!(
            "rename refused: spec name is `{}`, cannot become `{}`. \
             Restore `name:` in template.yaml or submit a fresh job to rename.",
            prev.name, parsed.name,
        )));
    }

    codeless_adapters_host::commit_paths(
        &repo_path,
        &format!("update template: {} (chat)", parsed.name),
        std::slice::from_ref(&tpl_path),
    )
    .map_err(|e| RpcError::Internal(format!("git: {e}")))?;

    job.template_yaml = Some(disk_yaml);
    rpc.bus
        .publish(
            Some(job.id),
            None,
            None,
            codeless_types::Event::JobTemplateUpdated { job_id: job.id },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(())
}

/// Filename `update_job_scope` writes. Held as a constant rather than
/// inlined so the wire spelling is in one place and the dispatcher map
/// in F3 stays trivial — the assistant action card carries no filename
/// because the chat surface only ever rewrites `SCOPE.md`. Spec edits
/// targeting other files go through `write_job_file` directly.
pub(super) const SCOPE_FILENAME: &str = "SCOPE.md";

pub(super) async fn update_job_scope(
    rpc: &super::InProcessRpc,
    args: UpdateJobScopeArgs,
) -> RpcResult<UpdateJobScopeResult> {
    if args.content.trim().is_empty() {
        return Err(RpcError::InvalidArgument(
            "scope content is empty; refusing to overwrite SCOPE.md with whitespace".into(),
        ));
    }
    let job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    // The paused-job guard: the chat surface refuses to race the runner.
    // `write_job_file` itself accepts the risk because the CLI / Spec
    // pane explicitly opts in — the gate lives here so a future tool
    // dispatcher routing `edit_scope` through this RPC inherits the
    // same rule without a duplicate match arm.
    if matches!(
        job.status,
        JobStatus::Running | JobStatus::Queued | JobStatus::AwaitingReview
    ) {
        return Err(RpcError::Conflict(format!(
            "job {} is {:?}; pause it first before rewriting SCOPE.md",
            job.id, job.status
        )));
    }
    let res = super::job_files::write_job_file(
        rpc,
        WriteJobFileArgs {
            job_id: args.job_id,
            filename: SCOPE_FILENAME.to_owned(),
            content: args.content,
        },
    )
    .await?;
    Ok(UpdateJobScopeResult { filename: res.name })
}

pub(super) async fn draft_job_from_conversation(
    rpc: &super::InProcessRpc,
    args: DraftJobFromConversationArgs,
) -> RpcResult<Job> {
    // Existence check up front so a missing thread surfaces as a typed
    // 404 rather than "no pending card" — the two failure modes look
    // similar from the wire and we want them distinguishable.
    rpc.store
        .get_assistant_thread(args.thread_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("assistant thread {}", args.thread_id)))?;

    let messages = rpc
        .store
        .list_assistant_messages(args.thread_id)
        .await
        .map_err(super::db_err)?;

    // Newest-pending wins. A thread can accumulate multiple `/draft`
    // proposals over its lifetime; the user re-issues the command when
    // they want to change the spec, and the dispatcher picks up the
    // last one so the conversation reads naturally. Cancelled or
    // already-confirmed cards are skipped — re-using one would
    // double-submit the same proposal.
    let draft = messages
        .iter()
        .rev()
        .filter(|m| matches!(m.role, AssistantMessageRole::Assistant))
        .filter_map(|m| m.meta_json.as_deref())
        .filter_map(|meta| serde_json::from_str::<AssistantActionCard>(meta).ok())
        .find(|card| {
            matches!(card.status, AssistantActionStatus::Pending)
                && matches!(card.action, AssistantAction::DraftJob { .. })
        });

    let Some(card) = draft else {
        return Err(RpcError::InvalidArgument(format!(
            "no pending DraftJob card on thread {}; issue `/draft <repo_id> -- <prompt>` first",
            args.thread_id
        )));
    };

    let AssistantAction::DraftJob {
        repo_id,
        prompt,
        runner,
        branch,
        cost_cap_cents,
        wall_clock_cap_ms,
        workspace_mode,
        model,
        permission_mode,
        effort,
    } = card.action
    else {
        // Unreachable: the `find` above already matched the variant.
        // Keeping the panic-free fallback so an accidental refactor
        // returns a typed error rather than reaching `unreachable!`.
        return Err(RpcError::Internal(
            "draft card variant mismatched after lookup".into(),
        ));
    };

    // `start_immediately = false` — the row lands in `Draft` so the
    // user can edit the spec / docs / handover before queueing,
    // matching SCOPE.md Decisions §3 (no "just do it" path from chat).
    submit_job(
        rpc,
        SubmitJobArgs {
            repo_id,
            prompt: Some(prompt),
            template_yaml: None,
            runner,
            branch,
            workspace_mode,
            cost_cap_cents,
            wall_clock_cap_ms,
            model,
            permission_mode,
            effort,
            // Chat-drafted jobs have no persona binding yet; the user
            // can pick one from the dropdown on the submit form if they
            // promote the draft from the job page.
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            start_immediately: false,
        },
    )
    .await
}
