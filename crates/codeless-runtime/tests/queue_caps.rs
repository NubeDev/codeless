//! Concurrency caps are enforced atomically inside `lease_next` —
//! a running-count over the three scopes (global, per-runner,
//! per-repo) gates the inner SELECT, so a saturated scope yields
//! `None` and the task stays enqueued until something completes.
//! Unlimited (`None`) caps short-circuit the count.

use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::{InProcessRpc, QueueConfig, SqliteStore};
use codeless_types::{
    CostCents, GitAuth, JobId, RepoId, Stage, StageId, StageStatus, Task, TaskId, TaskStatus,
    UnixMillis,
};
use sqlx::SqlitePool;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn rpc_with_caps(caps: QueueConfig) -> (InProcessRpc, SqliteStore) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let rpc = InProcessRpc::with_db(pool.clone()).await.unwrap();
    let store = SqliteStore::with_config(pool, caps);
    (rpc, store)
}

async fn seed_repo_job(rpc: &InProcessRpc, name: &str, runner: &str) -> (RepoId, JobId) {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: name.into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: Some(runner.into()),
        })
        .await
        .unwrap();
    let job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("x".into()),
            template_yaml: None,
            runner: runner.into(),
            branch: "codeless/job-1".into(),
            workspace_mode: None,
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            start_immediately: true,
        })
        .await
        .unwrap();
    (repo.id, job.id)
}

async fn seed_stage_with_tasks(store: &SqliteStore, job_id: JobId, n: usize) -> Vec<TaskId> {
    let stage = Stage {
        id: StageId::new(),
        job_id,
        ordinal: 0,
        name: "s".into(),
        status: StageStatus::Pending,
        verify_cmd: None,
        started_at: None,
        ended_at: None,
        session_id: None,
        goal: None,
        acceptance: None,
        last_activity_at: None,
        archived: false,
        persona_id: None,
        bypassed_at: None,
        bypassed_reason: None,
    };
    store.insert_stage(&stage).await.unwrap();
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let t = Task {
            id: TaskId::new(),
            stage_id: stage.id,
            ordinal: i as u32,
            status: TaskStatus::Enqueued,
            depends_on: vec![],
            lease_holder: None,
            lease_expires_at: None,
            cost_cents: CostCents::ZERO,
            input_tokens: 0,
            output_tokens: 0,
            started_at: None,
            ended_at: None,
        };
        store.enqueue_task(&t).await.unwrap();
        ids.push(t.id);
    }
    ids
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_cap_blocks_additional_leases() {
    let (rpc, store) = rpc_with_caps(QueueConfig {
        max_global: Some(1),
        ..QueueConfig::unlimited()
    })
    .await;
    let (_repo, job) = seed_repo_job(&rpc, "demo", "mock").await;
    let _ids = seed_stage_with_tasks(&store, job, 2).await;

    let first = store
        .lease_next("mock", "h1", 60_000, UnixMillis(1))
        .await
        .unwrap();
    assert!(first.is_some());
    let second = store
        .lease_next("mock", "h2", 60_000, UnixMillis(2))
        .await
        .unwrap();
    assert!(second.is_none(), "global cap=1 must reject second lease");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn per_runner_cap_isolates_runner_kinds() {
    let (rpc, store) = rpc_with_caps(QueueConfig {
        max_per_runner: Some(1),
        ..QueueConfig::unlimited()
    })
    .await;
    let (_, job_mock) = seed_repo_job(&rpc, "mock-repo", "mock").await;
    let (_, job_claude) = seed_repo_job(&rpc, "claude-repo", "claude").await;
    let _mock_ids = seed_stage_with_tasks(&store, job_mock, 2).await;
    let _claude_ids = seed_stage_with_tasks(&store, job_claude, 1).await;

    let mock_first = store
        .lease_next("mock", "hm1", 60_000, UnixMillis(1))
        .await
        .unwrap();
    assert!(mock_first.is_some());
    let mock_second = store
        .lease_next("mock", "hm2", 60_000, UnixMillis(2))
        .await
        .unwrap();
    assert!(mock_second.is_none(), "per-runner cap saturated for mock");

    let claude_ok = store
        .lease_next("claude", "hc1", 60_000, UnixMillis(3))
        .await
        .unwrap();
    assert!(
        claude_ok.is_some(),
        "different runner kind must not be blocked"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn per_repo_cap_isolates_repos() {
    let (rpc, store) = rpc_with_caps(QueueConfig {
        max_per_repo: Some(1),
        ..QueueConfig::unlimited()
    })
    .await;
    let (_, job_a) = seed_repo_job(&rpc, "repo-a", "mock").await;
    let (_, job_b) = seed_repo_job(&rpc, "repo-b", "mock").await;
    let _a = seed_stage_with_tasks(&store, job_a, 2).await;
    let _b = seed_stage_with_tasks(&store, job_b, 1).await;

    // First lease on repo-a saturates it.
    let _first = store
        .lease_next("mock", "h1", 60_000, UnixMillis(1))
        .await
        .unwrap()
        .expect("first lease");
    // Second lease attempt: the next enqueued task by ordinal is the
    // remaining repo-a task, which is blocked. We expect the planner
    // to skip ahead to repo-b — but per-task ordering is by ordinal
    // within stages, not across repos. So the second attempt picks
    // whichever runner-eligible task does not violate caps; with
    // SQLite's LIMIT 1 + ORDER BY ordinal, this returns the repo-b
    // task (ordinal 0) because it satisfies every cap.
    let second = store
        .lease_next("mock", "h2", 60_000, UnixMillis(2))
        .await
        .unwrap()
        .expect("second lease should pick repo-b");
    let row = store.get_task(second.id).await.unwrap().unwrap();
    assert_eq!(row.lease_holder.as_deref(), Some("h2"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_frees_cap_slot() {
    let (rpc, store) = rpc_with_caps(QueueConfig {
        max_global: Some(1),
        ..QueueConfig::unlimited()
    })
    .await;
    let (_, job) = seed_repo_job(&rpc, "demo", "mock").await;
    let ids = seed_stage_with_tasks(&store, job, 2).await;

    let first = store
        .lease_next("mock", "h1", 60_000, UnixMillis(1))
        .await
        .unwrap()
        .unwrap();
    assert!(store
        .lease_next("mock", "h2", 60_000, UnixMillis(2))
        .await
        .unwrap()
        .is_none());

    store
        .complete_task(first.id, "h1", CostCents::ZERO, 0, 0, UnixMillis(3))
        .await
        .unwrap();
    let next = store
        .lease_next("mock", "h3", 60_000, UnixMillis(4))
        .await
        .unwrap()
        .expect("slot freed");
    assert_eq!(next.id, ids[1]);
}
