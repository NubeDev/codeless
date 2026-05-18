//! Failure-time `set_policy` recommendation card (SCOPE-ASSISTANT-PARITY
//! W3d). When a job halts on a non-cap stage failure and has no
//! `auto_bypass_policy` set, we drop a one-shot `set_policy` action
//! card into the most-recently-touched assistant thread so the operator
//! can switch the job's posture from chat without leaving the surface
//! they were already on.
//!
//! The card is suppressed in three cases the scope doc calls out:
//!
//! - Cap-breach halts (`CostCap`, `WallClock`) — those publish
//!   `JobStopped`, not `JobFailed`, but the explicit `stop_reason`
//!   check defends against a future runner that stamps a cap reason
//!   on the row before flipping to `Failed`. Caps always halt, by
//!   design (AUTO-BYPASS-DECISIONS.md Q2).
//! - The job already has a policy set — under `Quick` / `LongTerm` /
//!   `Cheap` / `BestJudgement` / `JustCode` / `Custom` the operator
//!   already opted in, and under `Relentless` a stage failure should
//!   not have produced `JobFailed` at all (the policy auto-bypasses).
//! - No assistant thread exists yet — there is nowhere to render the
//!   card. The user's next visit will see the halted job through the
//!   regular jobs surface; we do not synthesise a thread out of
//!   nowhere just to hold a recommendation.

use codeless_rpc::RpcError;
use codeless_types::{
    AssistantAction, AssistantActionCard, AssistantMessage, AssistantMessageId,
    AssistantMessageRole, AutoBypassPolicy, Event, JobId, StopReason,
};

use crate::rpc::InProcessRpc;
use crate::time::now_ms;

/// Policy the failure-time card proposes. `LongTerm` is the
/// "auto-recover with a durable fix" preset — the most common posture
/// an operator picks after the first unplanned halt. The picker on the
/// confirmation card still lets the user swap it before confirming, so
/// the choice here is the default the planner stands behind, not a
/// hard-coded prescription.
const RECOMMENDED_POLICY: AutoBypassPolicy = AutoBypassPolicy::LongTerm;

/// Insert a `set_policy` action card on the most-recently-touched
/// assistant thread when a job halts on a non-cap stage failure with
/// no `auto_bypass_policy` set. Best-effort — a failure here logs and
/// returns rather than poisoning the caller's terminal-state path.
pub async fn maybe_emit_failure_set_policy_card(rpc: &InProcessRpc, job_id: JobId) {
    if let Err(err) = try_emit(rpc, job_id).await {
        tracing::warn!(
            %job_id,
            error = %err,
            "failure-time set_policy card: skipped",
        );
    }
}

async fn try_emit(rpc: &InProcessRpc, job_id: JobId) -> Result<(), RpcError> {
    let store = rpc.store();
    let bus = rpc.bus();

    let job = store
        .get_job(job_id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {job_id}")))?;
    if job.auto_bypass_policy.is_some() {
        return Ok(());
    }
    if matches!(
        job.stop_reason,
        Some(StopReason::CostCap) | Some(StopReason::WallClock)
    ) {
        return Ok(());
    }

    // Newest-touched first: `list_assistant_threads` orders by
    // `updated_at DESC, id DESC`, so the head row is the conversation
    // the operator was last in. No thread → nothing to do.
    let Some(thread) = store
        .list_assistant_threads()
        .await
        .map_err(db_err)?
        .into_iter()
        .next()
    else {
        return Ok(());
    };

    let card = AssistantActionCard::new(AssistantAction::SetPolicy {
        job_id,
        policy: Some(RECOMMENDED_POLICY),
    });
    let meta = serde_json::to_string(&card)
        .map_err(|e| RpcError::Internal(format!("serialise action card: {e}")))?;
    let now = now_ms();
    let content = format!(
        "Job `{job_id}` halted on a stage failure with no auto-bypass \
         policy set. Switch the job to `long-term` so a similar failure \
         auto-recovers next time?"
    );
    let row = AssistantMessage {
        id: AssistantMessageId::new(),
        thread_id: thread.id,
        role: AssistantMessageRole::Assistant,
        content,
        meta_json: Some(meta),
        created_at: now,
    };
    store.insert_assistant_message(&row).await.map_err(db_err)?;
    store
        .touch_assistant_thread(thread.id, now)
        .await
        .map_err(db_err)?;
    // Same synthetic `bus_job_id` the planner uses for `AiToken` /
    // `AssistantThreadTouched` envelopes: the per-thread subscriber
    // filters on `JobId(thread_id.0)` regardless of which job the
    // card references.
    let bus_job_id = JobId(thread.id.0);
    bus.publish(
        Some(bus_job_id),
        None,
        None,
        Event::AssistantThreadTouched {
            thread_id: thread.id,
        },
        now,
    )
    .await
    .map_err(db_err)?;
    Ok(())
}

fn db_err(e: sqlx::Error) -> RpcError {
    RpcError::Internal(format!("db: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::InProcessRpc;
    use codeless_rpc::{CreateAssistantThreadArgs, RpcServer};
    use codeless_types::{
        AssistantActionStatus, CostCents, GitAuth, Job, JobStatus, Repo, RepoId, UnixMillis,
        WorkspaceMode,
    };

    async fn insert_repo(rpc: &InProcessRpc) -> RepoId {
        let repo_id = RepoId::new();
        let now = UnixMillis(1_778_000_000_000);
        rpc.store()
            .insert_repo(&Repo {
                id: repo_id,
                name: "test".into(),
                clone_url: "ssh://x/y".into(),
                default_branch: "main".into(),
                local_path: "/nonexistent/codeless-w3d-test".into(),
                git_auth: GitAuth::Token {
                    env_var: "TOKEN".into(),
                },
                concurrency_cap: None,
                default_runner: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repo_id
    }

    fn failed_job(
        repo_id: RepoId,
        policy: Option<AutoBypassPolicy>,
        stop_reason: Option<StopReason>,
    ) -> Job {
        Job {
            id: JobId::new(),
            repo_id,
            status: JobStatus::Failed,
            stop_reason,
            template_yaml: None,
            prompt: Some("anything".into()),
            runner: "mock".into(),
            branch: "codeless/test".into(),
            workspace_mode: WorkspaceMode::default(),
            worktree_path: None,
            cost_cap_cents: CostCents(0),
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(0),
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: policy,
            pending_operator_comment: None,
            precheck_override_once: false,
            started_at: Some(UnixMillis(1_778_000_000_000)),
            ended_at: Some(UnixMillis(1_778_000_060_000)),
            created_at: UnixMillis(1_778_000_000_000),
        }
    }

    async fn rpc_with_thread() -> (
        InProcessRpc,
        tempfile::TempDir,
        codeless_types::AssistantThread,
    ) {
        let dir = tempfile::TempDir::new().unwrap();
        let rpc = InProcessRpc::new()
            .await
            .unwrap()
            .with_assistant_data_dir(dir.path().to_path_buf());
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs {
                title: None,
                persona_id: "builtin:general".into(),
            })
            .await
            .unwrap();
        (rpc, dir, thread)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn emits_card_on_none_policy_failure() {
        let (rpc, _data, thread) = rpc_with_thread().await;
        let repo_id = insert_repo(&rpc).await;
        let job = failed_job(repo_id, None, Some(StopReason::RunnerCrash));
        rpc.store().insert_job(&job).await.unwrap();

        maybe_emit_failure_set_policy_card(&rpc, job.id).await;

        let messages = rpc
            .store()
            .list_assistant_messages(thread.id)
            .await
            .unwrap();
        // Thread had no prior messages, so the recommendation row is
        // the only one in the transcript.
        assert_eq!(messages.len(), 1);
        let row = &messages[0];
        assert!(matches!(row.role, AssistantMessageRole::Assistant));
        let card: AssistantActionCard =
            serde_json::from_str(row.meta_json.as_deref().expect("meta_json")).unwrap();
        assert!(matches!(card.status, AssistantActionStatus::Pending));
        match card.action {
            AssistantAction::SetPolicy {
                job_id,
                policy: Some(AutoBypassPolicy::LongTerm),
            } => assert_eq!(job_id, job.id),
            other => panic!("expected SetPolicy(LongTerm), got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn skips_when_policy_already_set() {
        let (rpc, _data, thread) = rpc_with_thread().await;
        let repo_id = insert_repo(&rpc).await;
        let job = failed_job(
            repo_id,
            Some(AutoBypassPolicy::Quick),
            Some(StopReason::RunnerCrash),
        );
        rpc.store().insert_job(&job).await.unwrap();

        maybe_emit_failure_set_policy_card(&rpc, job.id).await;

        let messages = rpc
            .store()
            .list_assistant_messages(thread.id)
            .await
            .unwrap();
        assert!(
            messages.is_empty(),
            "card must not be emitted when policy is already set; got {messages:?}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn skips_on_cost_cap_breach() {
        let (rpc, _data, thread) = rpc_with_thread().await;
        let repo_id = insert_repo(&rpc).await;
        let job = failed_job(repo_id, None, Some(StopReason::CostCap));
        rpc.store().insert_job(&job).await.unwrap();

        maybe_emit_failure_set_policy_card(&rpc, job.id).await;

        let messages = rpc
            .store()
            .list_assistant_messages(thread.id)
            .await
            .unwrap();
        assert!(messages.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn skips_on_wall_clock_breach() {
        let (rpc, _data, thread) = rpc_with_thread().await;
        let repo_id = insert_repo(&rpc).await;
        let job = failed_job(repo_id, None, Some(StopReason::WallClock));
        rpc.store().insert_job(&job).await.unwrap();

        maybe_emit_failure_set_policy_card(&rpc, job.id).await;

        let messages = rpc
            .store()
            .list_assistant_messages(thread.id)
            .await
            .unwrap();
        assert!(messages.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_thread_is_a_quiet_noop() {
        let dir = tempfile::TempDir::new().unwrap();
        let rpc = InProcessRpc::new()
            .await
            .unwrap()
            .with_assistant_data_dir(dir.path().to_path_buf());
        let repo_id = insert_repo(&rpc).await;
        let job = failed_job(repo_id, None, Some(StopReason::RunnerCrash));
        rpc.store().insert_job(&job).await.unwrap();

        // No thread exists; the function must not panic or error.
        maybe_emit_failure_set_policy_card(&rpc, job.id).await;
    }
}
