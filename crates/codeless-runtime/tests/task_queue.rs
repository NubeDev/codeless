//! Task queue contract pinned by these tests:
//! - `enqueue_task` + `lease_next` claim the next enqueued row for
//!   the requested runner kind, ordered by `ordinal`.
//! - Two concurrent `lease_next` calls on a single-task queue:
//!   exactly one wins (no double-lease).
//! - `lease_next` skips tasks whose `depends_on` is not all
//!   `completed` yet.
//! - `release_expired_leases` flips an expired `running` lease back
//!   to `enqueued` and clears the holder fields.
//! - `complete_task` + `fail_task` only act on rows whose
//!   `lease_holder` matches (CAS semantics).

use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::InProcessRpc;
use codeless_types::{
    CostCents, GitAuth, JobId, Stage, StageId, StageStatus, Task, TaskId, TaskStatus, UnixMillis,
};

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn fresh_job(rpc: &InProcessRpc, runner: &str) -> JobId {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: format!("demo-{runner}"),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: Some(runner.into()),
        })
        .await
        .unwrap();
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("x".into()),
        template_yaml: None,
        runner: runner.into(),
        branch: "codeless/job-1".into(),
        cost_cap_cents: 0,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
    })
    .await
    .unwrap()
    .id
}

async fn fresh_stage(rpc: &InProcessRpc, job_id: JobId, ordinal: u32) -> StageId {
    let stage = Stage {
        id: StageId::new(),
        job_id,
        ordinal,
        name: format!("stage-{ordinal}"),
        status: StageStatus::Pending,
        verify_cmd: None,
        started_at: None,
        ended_at: None,
    };
    rpc.store().insert_stage(&stage).await.unwrap();
    stage.id
}

fn task(stage_id: StageId, ordinal: u32, depends_on: Vec<TaskId>) -> Task {
    Task {
        id: TaskId::new(),
        stage_id,
        ordinal,
        status: TaskStatus::Enqueued,
        depends_on,
        lease_holder: None,
        lease_expires_at: None,
        cost_cents: CostCents::ZERO,
        input_tokens: 0,
        output_tokens: 0,
        started_at: None,
        ended_at: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lease_next_returns_enqueued_tasks_in_ordinal_order() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = fresh_job(&rpc, "mock").await;
    let stage = fresh_stage(&rpc, job, 0).await;
    let store = rpc.store();
    let t1 = task(stage, 1, vec![]);
    let t2 = task(stage, 2, vec![]);
    store.enqueue_task(&t1).await.unwrap();
    store.enqueue_task(&t2).await.unwrap();

    let leased1 = store
        .lease_next("mock", "h1", 60_000, UnixMillis(1))
        .await
        .unwrap()
        .expect("first lease");
    assert_eq!(leased1.id, t1.id);
    assert_eq!(leased1.status, TaskStatus::Running);
    assert_eq!(leased1.lease_holder.as_deref(), Some("h1"));

    let leased2 = store
        .lease_next("mock", "h2", 60_000, UnixMillis(2))
        .await
        .unwrap()
        .expect("second lease");
    assert_eq!(leased2.id, t2.id);
    assert_eq!(leased2.lease_holder.as_deref(), Some("h2"));

    let empty = store
        .lease_next("mock", "h3", 60_000, UnixMillis(3))
        .await
        .unwrap();
    assert!(empty.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lease_next_does_not_double_lease_under_contention() {
    let rpc = std::sync::Arc::new(InProcessRpc::new().await.unwrap());
    let job = fresh_job(&rpc, "mock").await;
    let stage = fresh_stage(&rpc, job, 0).await;
    let t = task(stage, 1, vec![]);
    rpc.store().enqueue_task(&t).await.unwrap();

    let rpc_a = std::sync::Arc::clone(&rpc);
    let rpc_b = std::sync::Arc::clone(&rpc);
    let (a, b) = tokio::join!(
        async move {
            rpc_a
                .store()
                .lease_next("mock", "a", 60_000, UnixMillis(1))
                .await
        },
        async move {
            rpc_b
                .store()
                .lease_next("mock", "b", 60_000, UnixMillis(1))
                .await
        }
    );
    let winners = [a.unwrap(), b.unwrap()];
    let won: Vec<_> = winners.iter().filter_map(|x| x.as_ref()).collect();
    assert_eq!(won.len(), 1, "exactly one caller must win the lease");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lease_next_skips_tasks_with_unsatisfied_dependencies() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = fresh_job(&rpc, "mock").await;
    let stage = fresh_stage(&rpc, job, 0).await;
    let store = rpc.store();
    let prereq = task(stage, 1, vec![]);
    let dependent = task(stage, 2, vec![prereq.id]);
    store.enqueue_task(&prereq).await.unwrap();
    store.enqueue_task(&dependent).await.unwrap();

    let leased = store
        .lease_next("mock", "h", 60_000, UnixMillis(1))
        .await
        .unwrap()
        .expect("first lease");
    assert_eq!(leased.id, prereq.id);

    let none = store
        .lease_next("mock", "h", 60_000, UnixMillis(2))
        .await
        .unwrap();
    assert!(
        none.is_none(),
        "dependent must not be leasable while prereq is still running",
    );

    store
        .complete_task(prereq.id, "h", CostCents::ZERO, 0, 0, UnixMillis(3))
        .await
        .unwrap();
    let next = store
        .lease_next("mock", "h", 60_000, UnixMillis(4))
        .await
        .unwrap()
        .expect("dependent lease");
    assert_eq!(next.id, dependent.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn release_expired_leases_returns_tasks_to_the_queue() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = fresh_job(&rpc, "mock").await;
    let stage = fresh_stage(&rpc, job, 0).await;
    let store = rpc.store();
    let t = task(stage, 1, vec![]);
    store.enqueue_task(&t).await.unwrap();
    let leased = store
        .lease_next("mock", "dead", 500, UnixMillis(100))
        .await
        .unwrap()
        .expect("first lease");
    assert_eq!(leased.lease_expires_at, Some(UnixMillis(600)));

    let reclaimed = store
        .release_expired_leases(UnixMillis(1_000))
        .await
        .unwrap();
    assert_eq!(reclaimed, 1);

    let regotten = store.get_task(t.id).await.unwrap().unwrap();
    assert_eq!(regotten.status, TaskStatus::Enqueued);
    assert!(regotten.lease_holder.is_none());
    assert!(regotten.lease_expires_at.is_none());

    let next = store
        .lease_next("mock", "alive", 60_000, UnixMillis(1_500))
        .await
        .unwrap()
        .expect("re-lease");
    assert_eq!(next.id, t.id);
    assert_eq!(next.lease_holder.as_deref(), Some("alive"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn complete_task_refuses_when_holder_does_not_match() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = fresh_job(&rpc, "mock").await;
    let stage = fresh_stage(&rpc, job, 0).await;
    let store = rpc.store();
    let t = task(stage, 1, vec![]);
    store.enqueue_task(&t).await.unwrap();
    store
        .lease_next("mock", "real", 60_000, UnixMillis(1))
        .await
        .unwrap()
        .expect("lease");

    let stolen = store
        .complete_task(t.id, "imposter", CostCents::ZERO, 0, 0, UnixMillis(2))
        .await
        .unwrap();
    assert!(!stolen, "completion by non-holder must be a no-op");

    let still_running = store.get_task(t.id).await.unwrap().unwrap();
    assert_eq!(still_running.status, TaskStatus::Running);
    assert_eq!(still_running.lease_holder.as_deref(), Some("real"));
}
