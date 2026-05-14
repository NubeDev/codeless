//! End-to-end resumability across a simulated core restart. Sets up
//! a file-backed SQLite database, builds a runtime against it, lands
//! a non-trivial amount of state (repo + job + stage + tasks +
//! events + a leased task with expired lease), closes the runtime
//! and the pool, then rebuilds a fresh runtime against the same file
//! and asserts every row survived. This is the contract Phase 2a
//! exists to deliver: a core that dies mid-flight comes back up
//! against its own SQLite file with no data loss.

use std::sync::Arc;

use codeless_rpc::{
    AddRepoArgs, EventFilter, GetJobArgs, ListJobsArgs, ListReposResult, RpcServer, SubmitJobArgs,
};
use codeless_runtime::InProcessRpc;
use codeless_types::{
    CostCents, EventCursor, GitAuth, Stage, StageId, StageStatus, Task, TaskId, TaskStatus,
    UnixMillis,
};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqlitePool};
use tempfile::TempDir;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn open_pool(db_path: &std::path::Path) -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .disable_statement_logging();
    sqlx::pool::PoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("open file pool")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_preserves_repos_jobs_tasks_events_and_cursors() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("codeless.db");

    // -- session 1: write a bunch of state, then drop the runtime. --
    let (repo_id, job_id, task_ids, last_cursor) = {
        let pool = open_pool(&db_path).await;
        let rpc = InProcessRpc::with_db(pool).await.unwrap();
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
        let job = rpc
            .submit_job(SubmitJobArgs {
                repo_id: repo.id,
                prompt: Some("hello".into()),
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
            .unwrap();

        let store = Arc::clone(rpc.store());
        let stage = Stage {
            id: StageId::new(),
            job_id: job.id,
            ordinal: 0,
            name: "build".into(),
            status: StageStatus::Pending,
            verify_cmd: None,
            started_at: None,
            ended_at: None,
            session_id: None,
        };
        store.insert_stage(&stage).await.unwrap();

        let mut tasks = Vec::new();
        for i in 0..3 {
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
            tasks.push(t.id);
        }

        // Lease one task and forge an already-expired expiry so the
        // startup reaper has something to reclaim on restart.
        store
            .lease_next("mock", "ghost", 60_000, UnixMillis(1))
            .await
            .unwrap()
            .unwrap();
        sqlx::query("UPDATE tasks SET lease_expires_at = 0 WHERE id = ?")
            .bind(tasks[0].to_string())
            .execute(rpc.pool())
            .await
            .unwrap();

        let last_cursor = sqlx::query_scalar::<_, i64>("SELECT MAX(cursor) FROM events")
            .fetch_one(rpc.pool())
            .await
            .unwrap();

        // Explicitly close the pool so the file is fully released
        // before session 2 opens it — sqlite WAL handles concurrent
        // opens fine, but the test reads cleaner with one pool at a
        // time.
        rpc.pool().close().await;
        drop(rpc);

        (repo.id, job.id, tasks, last_cursor)
    };
    assert!(
        last_cursor >= 2,
        "session 1 must have at least repo+job events"
    );

    // -- session 2: fresh runtime against the same file. --
    let pool = open_pool(&db_path).await;
    let rpc = InProcessRpc::with_db(pool).await.unwrap();

    let ListReposResult { repos } = rpc.list_repos().await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].id, repo_id);

    let job = rpc.get_job(GetJobArgs { job_id }).await.unwrap();
    assert_eq!(job.id, job_id);
    let listed = rpc
        .list_jobs(ListJobsArgs {
            repo_id: Some(repo_id),
        })
        .await
        .unwrap();
    assert_eq!(listed.jobs.len(), 1);

    let store = Arc::clone(rpc.store());
    // Startup reaper ran inside `with_db`; the previously-leased
    // task must be back to enqueued and ready to be claimed.
    let reaped = store.get_task(task_ids[0]).await.unwrap().unwrap();
    assert_eq!(reaped.status, TaskStatus::Enqueued);
    assert!(reaped.lease_holder.is_none());

    let claimed = store
        .lease_next("mock", "fresh", 60_000, UnixMillis(10_000))
        .await
        .unwrap()
        .expect("re-claim after restart");
    assert_eq!(claimed.id, task_ids[0]);

    // Event cursor must keep climbing — the new stop_job event
    // should land at last_cursor + 1, proving the AUTOINCREMENT
    // allocator survived the restart.
    rpc.stop_job(codeless_rpc::StopJobArgs { job_id })
        .await
        .unwrap();
    let new_max = sqlx::query_scalar::<_, i64>("SELECT MAX(cursor) FROM events")
        .fetch_one(rpc.pool())
        .await
        .unwrap();
    assert!(
        new_max > last_cursor,
        "cursor must keep climbing across restart: last={last_cursor} new={new_max}",
    );

    // `subscribe(since=last_cursor)` must replay every event added
    // after session 1's last cursor (including the job-stopped we
    // just published) — proving the persisted log is the source of
    // truth for catch-up across a restart.
    use futures_util::StreamExt;
    let mut stream = rpc
        .subscribe(EventFilter::All, Some(EventCursor(last_cursor)))
        .await
        .unwrap();
    let env = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("timeout")
        .expect("stream end")
        .expect("stream error");
    assert!(env.cursor.0 > last_cursor);
}
