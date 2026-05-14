//! Heartbeat helper pins:
//! - A live heartbeat task pushes `lease_expires_at` forward against
//!   wall-clock time.
//! - Aborting the JoinHandle stops the renewal.
//! - If the lease is stolen (another holder wins), the heartbeat
//!   exits cleanly on its next tick rather than spinning.
//!
//! Startup reaper pins: an expired `running` lease that survived a
//! previous core's death is returned to `enqueued` when a fresh
//! `InProcessRpc::with_db` runs against the same pool.

use std::sync::Arc;
use std::time::Duration;

use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::{spawn_heartbeat, InProcessRpc};
use codeless_types::{
    CostCents, GitAuth, JobId, Stage, StageId, StageStatus, Task, TaskId, TaskStatus, UnixMillis,
};
use sqlx::SqlitePool;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn fresh_job(rpc: &InProcessRpc) -> JobId {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: Some("mock".into()),
        })
        .await
        .unwrap();
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("x".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "codeless/job-1".into(),
        workspace_mode: None,
        cost_cap_cents: 0,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
        start_immediately: true,
    })
    .await
    .unwrap()
    .id
}

async fn fresh_stage(rpc: &InProcessRpc, job_id: JobId) -> StageId {
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
    };
    rpc.store().insert_stage(&stage).await.unwrap();
    stage.id
}

fn enqueued_task(stage_id: StageId) -> Task {
    Task {
        id: TaskId::new(),
        stage_id,
        ordinal: 0,
        status: TaskStatus::Enqueued,
        depends_on: vec![],
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
async fn heartbeat_extends_lease_then_stops_on_abort() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = fresh_job(&rpc).await;
    let stage = fresh_stage(&rpc, job).await;
    let store = Arc::clone(rpc.store());
    let t = enqueued_task(stage);
    store.enqueue_task(&t).await.unwrap();
    let leased = store
        .lease_next("mock", "holder", 200, UnixMillis(1))
        .await
        .unwrap()
        .unwrap();
    let initial_expiry = leased.lease_expires_at.unwrap().0;

    let handle = spawn_heartbeat(
        Arc::clone(&store),
        t.id,
        "holder".into(),
        Duration::from_millis(30),
        Duration::from_secs(60),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mid = store.get_task(t.id).await.unwrap().unwrap();
    assert_eq!(mid.lease_holder.as_deref(), Some("holder"));
    let mid_expiry = mid.lease_expires_at.unwrap().0;
    assert!(
        mid_expiry > initial_expiry,
        "lease_expires_at must move forward: initial={initial_expiry} mid={mid_expiry}",
    );

    handle.abort();
    let _ = handle.await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let frozen = store
        .get_task(t.id)
        .await
        .unwrap()
        .unwrap()
        .lease_expires_at
        .unwrap()
        .0;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let later = store
        .get_task(t.id)
        .await
        .unwrap()
        .unwrap()
        .lease_expires_at
        .unwrap()
        .0;
    assert_eq!(frozen, later, "no further renewal after abort");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_exits_when_lease_is_stolen() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = fresh_job(&rpc).await;
    let stage = fresh_stage(&rpc, job).await;
    let store = Arc::clone(rpc.store());
    let t = enqueued_task(stage);
    store.enqueue_task(&t).await.unwrap();
    store
        .lease_next("mock", "old", 200, UnixMillis(1))
        .await
        .unwrap()
        .unwrap();

    let handle = spawn_heartbeat(
        Arc::clone(&store),
        t.id,
        "old".into(),
        Duration::from_millis(30),
        Duration::from_secs(60),
    );

    // Forcibly transfer the lease by clearing then re-leasing under
    // a different holder, simulating a reaper + fresh claim.
    store
        .release_expired_leases(UnixMillis(10_000))
        .await
        .unwrap();
    store
        .lease_next("mock", "new", 60_000, UnixMillis(10_001))
        .await
        .unwrap()
        .unwrap();

    let result = tokio::time::timeout(Duration::from_millis(500), handle).await;
    assert!(result.is_ok(), "heartbeat must exit after losing its lease");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_reaper_reclaims_expired_running_tasks() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let rpc = InProcessRpc::with_db(pool.clone()).await.unwrap();
    let job = fresh_job(&rpc).await;
    let stage = fresh_stage(&rpc, job).await;
    let store = Arc::clone(rpc.store());
    let t = enqueued_task(stage);
    store.enqueue_task(&t).await.unwrap();
    // Lease and forge an already-expired `lease_expires_at` directly
    // on the row to imitate "previous core died with leases in
    // flight". Bypassing `lease_next` here is fine; we are setting up
    // the precondition the startup reaper exists to fix.
    store
        .lease_next("mock", "ghost", 60_000, UnixMillis(1))
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE tasks SET lease_expires_at = 0 WHERE id = ?")
        .bind(t.id.to_string())
        .execute(rpc.pool())
        .await
        .unwrap();

    // Drop the old rpc and build a fresh one against the same pool.
    drop(rpc);
    let _resumed = InProcessRpc::with_db(pool.clone()).await.unwrap();
    let reaped = store.get_task(t.id).await.unwrap().unwrap();
    assert_eq!(reaped.status, TaskStatus::Enqueued);
    assert!(reaped.lease_holder.is_none());
    assert!(reaped.lease_expires_at.is_none());
}
