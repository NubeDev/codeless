//! Stage 7 exit test for the adapter-registry job.
//!
//! `restart_server` partitions every `Running` job into *resumable* vs
//! *killed* before it agrees to exit. The decision rule is fixed in
//! `DOCS/SCOPE.md` §"Adapter registry, stage 1": a job is resumable
//! iff its runner is template-driven (today: `mock`, `anthropic`) and
//! its most recent persisted stage transition is within the last 30s;
//! everything else is killed. The pre-condition is then gated by the
//! `force` flag — `false` returns `RestartHasRunningJobs { resumable,
//! killed }` and refuses to exit; `true` proceeds.
//!
//! The test drives the partition directly against `InProcessRpc` by
//! seeding rows in SQLite — the unit of behaviour under test is the
//! decision, not the runner. The same surface lights up under a real
//! driver because both paths read through `SqliteStore::list_jobs` +
//! `list_stages_for_job`.

use std::time::Duration;

use codeless_rpc::{AdapterError, RestartServerArgs, RpcError, RpcServer};
use codeless_runtime::{InProcessRpc, RestartContext, RESUMABLE_WINDOW};
use codeless_types::{
    AutoBypassPolicy, CostCents, GitAuth, Job, JobId, JobStatus, Repo, RepoId, Stage, StageId,
    StageStatus, UnixMillis, WorkspaceMode,
};

fn now_ms() -> i64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .expect("system clock")
}

fn make_repo(now: i64) -> Repo {
    Repo {
        id: RepoId::new(),
        name: "demo".into(),
        clone_url: "https://example.test/demo.git".into(),
        default_branch: "main".into(),
        local_path: "/tmp/demo".into(),
        git_auth: GitAuth::Token {
            env_var: "GITHUB_TOKEN".into(),
        },
        concurrency_cap: None,
        default_runner: None,
        created_at: UnixMillis(now),
        updated_at: UnixMillis(now),
    }
}

fn make_job(repo_id: RepoId, runner: &str, status: JobStatus, now: i64) -> Job {
    Job {
        id: JobId::new(),
        repo_id,
        status,
        stop_reason: None,
        template_yaml: None,
        prompt: Some("anything".into()),
        runner: runner.into(),
        branch: format!("codeless/job-{}", runner),
        workspace_mode: WorkspaceMode::Worktree,
        worktree_path: None,
        cost_cap_cents: CostCents(500),
        wall_clock_cap_ms: 60_000,
        cost_cents: CostCents(0),
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        auto_bypass_policy: None as Option<AutoBypassPolicy>,
        pending_operator_comment: None,
        precheck_override_once: false,
        started_at: Some(UnixMillis(now)),
        ended_at: None,
        created_at: UnixMillis(now),
    }
}

fn make_stage(job_id: JobId, started_at_ms: i64) -> Stage {
    Stage {
        id: StageId::new(),
        job_id,
        ordinal: 0,
        name: "stage-0".into(),
        status: StageStatus::Running,
        verify_cmd: None,
        started_at: Some(UnixMillis(started_at_ms)),
        ended_at: None,
        session_id: None,
        goal: None,
        acceptance: None,
        last_activity_at: Some(UnixMillis(started_at_ms)),
        archived: false,
        persona_id: None,
        bypassed_at: None,
        bypassed_reason: None,
        failure_class: None,
        failure_detail: None,
    }
}

async fn seed_job(rpc: &InProcessRpc, repo_id: RepoId, runner: &str, stage_age: Duration) -> JobId {
    let now = now_ms();
    let job = make_job(repo_id, runner, JobStatus::Running, now);
    let job_id = job.id;
    rpc.store().insert_job(&job).await.expect("insert_job");
    let stage = make_stage(job_id, now - stage_age.as_millis() as i64);
    rpc.store()
        .insert_stage(&stage)
        .await
        .expect("insert_stage");
    job_id
}

/// Build a runtime under `SupervisedCli` so the post-partition path
/// has somewhere to land — `Bare` would short-circuit to
/// `RestartUnsupervised` and obscure the partition we are testing.
async fn supervised_rpc() -> InProcessRpc {
    let rpc = InProcessRpc::new()
        .await
        .expect("fresh in-memory runtime")
        .with_restart_context(RestartContext::SupervisedCli);
    // Seed a `repos` row first; jobs FK-reference it via `repo_id`.
    let now = now_ms();
    rpc.store()
        .insert_repo(&make_repo(now))
        .await
        .expect("insert_repo");
    rpc
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partition_splits_running_jobs_by_runner_and_recency() {
    let rpc = supervised_rpc().await;
    let repo_id = rpc.store().list_repos().await.unwrap()[0].id;

    // Template-driven, recent stage: resumable.
    let resumable = seed_job(&rpc, repo_id, "mock", Duration::from_secs(5)).await;
    // PTY-bound runner: killed regardless of recency.
    let pty_killed = seed_job(&rpc, repo_id, "claude", Duration::from_secs(2)).await;
    // Template-driven but stale checkpoint: killed.
    let stale = seed_job(
        &rpc,
        repo_id,
        "anthropic",
        RESUMABLE_WINDOW + Duration::from_secs(5),
    )
    .await;

    let err = rpc
        .restart_server(RestartServerArgs { force: false })
        .await
        .expect_err("force=false with running jobs must refuse");
    let (got_resumable, got_killed) = match err {
        RpcError::Adapter(AdapterError::RestartHasRunningJobs { resumable, killed }) => {
            (resumable, killed)
        }
        other => panic!("expected RestartHasRunningJobs, got {other:?}"),
    };
    assert!(
        got_resumable.contains(&resumable),
        "expected {resumable:?} in resumable list {got_resumable:?}"
    );
    assert!(
        got_killed.contains(&pty_killed),
        "PTY-bound runner must be killed, not resumable; got resumable={got_resumable:?} killed={got_killed:?}"
    );
    assert!(
        got_killed.contains(&stale),
        "stale checkpoint must be killed; got resumable={got_resumable:?} killed={got_killed:?}"
    );
    assert!(
        !got_resumable.contains(&pty_killed),
        "PTY-bound runner must never appear in resumable"
    );
    assert!(
        !got_resumable.contains(&stale),
        "stale checkpoint must not appear in resumable"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_true_proceeds_past_running_jobs() {
    let rpc = supervised_rpc().await;
    let repo_id = rpc.store().list_repos().await.unwrap()[0].id;
    let _ = seed_job(&rpc, repo_id, "claude", Duration::from_secs(2)).await;

    let trigger = rpc.restart_trigger();
    let waiter = trigger.clone();
    let join = tokio::spawn(async move { waiter.wait().await });

    rpc.restart_server(RestartServerArgs { force: true })
        .await
        .expect("force=true must proceed even with running jobs");

    // Trigger must have fired — supervised context exits with 75.
    tokio::time::timeout(Duration::from_secs(2), join)
        .await
        .expect("restart trigger must fire after force=true call")
        .expect("waiter task");
    assert_eq!(
        trigger.desired_exit_code(),
        Some(codeless_runtime::EX_TEMPFAIL)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bare_context_returns_unsupervised_hint() {
    let rpc = InProcessRpc::new()
        .await
        .expect("fresh runtime")
        .with_restart_context(RestartContext::Bare);
    // No running jobs — the partition path is empty, so the next
    // gate is the context check.
    let err = rpc
        .restart_server(RestartServerArgs { force: false })
        .await
        .expect_err("bare context must refuse to exit");
    match err {
        RpcError::Adapter(AdapterError::RestartUnsupervised { hint }) => {
            assert!(
                hint.contains("--respawn-on-exit"),
                "the hint must mention the watcher flag so the operator can opt in; got {hint:?}"
            );
        }
        other => panic!("expected RestartUnsupervised, got {other:?}"),
    }
    assert!(
        rpc.restart_trigger().desired_exit_code().is_none(),
        "bare context must not arm the trigger"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tauri_desktop_context_fires_with_exit_code_zero() {
    let rpc = InProcessRpc::new()
        .await
        .expect("fresh runtime")
        .with_restart_context(RestartContext::TauriDesktop);
    rpc.restart_server(RestartServerArgs { force: false })
        .await
        .expect("no running jobs, Tauri context proceeds");
    assert_eq!(
        rpc.restart_trigger().desired_exit_code(),
        Some(0),
        "Tauri sidecar exit code must be 0 — the shell interprets the drop, not EX_TEMPFAIL"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_running_set_proceeds_without_force() {
    let rpc = supervised_rpc().await;
    rpc.restart_server(RestartServerArgs { force: false })
        .await
        .expect("with no running jobs, force=false still proceeds");
    assert_eq!(
        rpc.restart_trigger().desired_exit_code(),
        Some(codeless_runtime::EX_TEMPFAIL)
    );
}
