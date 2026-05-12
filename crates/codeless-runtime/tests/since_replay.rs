//! `subscribe(since)` must deliver every event with `cursor > since`
//! exactly once, regardless of whether the event lived in SQLite at
//! subscription time or arrives on the live broadcast tail after.
//! The boundary contract is the load-bearing piece — events that
//! land between SELECT and the start of the live drain must not be
//! dropped or duplicated.

use std::time::Duration;

use codeless_rpc::{AddRepoArgs, EventFilter, RpcServer, SubmitJobArgs};
use codeless_runtime::InProcessRpc;
use codeless_types::{Event, EventCursor, GitAuth};
use futures_util::StreamExt;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn collect_with_timeout(
    stream: &mut codeless_rpc::EventStream,
    target: usize,
) -> Vec<codeless_types::EventEnvelope> {
    let mut out = Vec::with_capacity(target);
    while out.len() < target {
        let env = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("timeout")
            .expect("stream end")
            .expect("stream error");
        out.push(env);
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn since_zero_replays_everything_then_attaches_live_tail() {
    let rpc = InProcessRpc::new().await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    let job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("first".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/job-1".into(),
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
        })
        .await
        .unwrap();

    let mut stream = rpc
        .subscribe(EventFilter::All, Some(EventCursor(0)))
        .await
        .unwrap();

    let later_job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("second".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/job-2".into(),
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
        })
        .await
        .unwrap();

    let envs = collect_with_timeout(&mut stream, 3).await;
    let cursors: Vec<i64> = envs.iter().map(|e| e.cursor.0).collect();
    assert_eq!(cursors, vec![1, 2, 3]);
    assert!(matches!(envs[0].event, Event::RepoAdded { .. }));
    match &envs[1].event {
        Event::JobQueued { job_id, .. } => assert_eq!(*job_id, job.id),
        other => panic!("expected job-queued for first job at cursor 2, got {other:?}"),
    }
    match &envs[2].event {
        Event::JobQueued { job_id, .. } => assert_eq!(*job_id, later_job.id),
        other => panic!("expected job-queued for second job at cursor 3, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn since_above_max_filters_strictly_by_cursor() {
    // Strict `cursor > since` semantics: a client that hands us a
    // `since` greater than anything we have skips both the empty
    // replay and any live events whose cursor remains below that
    // claimed high-water mark. The client is responsible for handing
    // us a real cursor it actually observed.
    let rpc = InProcessRpc::new().await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();

    let mut stream = rpc
        .subscribe(EventFilter::All, Some(EventCursor(100)))
        .await
        .unwrap();
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("x".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "codeless/job-x".into(),
        cost_cap_cents: 0,
        wall_clock_cap_ms: 60_000,
    })
    .await
    .unwrap();

    let polled = tokio::time::timeout(Duration::from_millis(250), stream.next()).await;
    assert!(
        polled.is_err(),
        "expected timeout (no events > cursor 100), got {polled:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_dedupes_overlap_with_live_tail() {
    // Subscribe with `since=1` AFTER cursor 1 (repo-added) and BEFORE
    // cursor 2 (job-queued). Cursor 2 must arrive exactly once — it
    // is reachable both via the SQLite replay (it has likely landed
    // in the DB by the time the SELECT runs) and via the live
    // broadcast tail (the subscription captured it). The dedupe
    // logic keys on `max_seen` derived from the replay's last
    // cursor.
    let rpc = InProcessRpc::new().await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("x".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "codeless/job-x".into(),
        cost_cap_cents: 0,
        wall_clock_cap_ms: 60_000,
    })
    .await
    .unwrap();

    let mut stream = rpc
        .subscribe(EventFilter::All, Some(EventCursor(1)))
        .await
        .unwrap();

    let envs = collect_with_timeout(&mut stream, 1).await;
    assert_eq!(envs[0].cursor.0, 2);
    // Confirm there is no second emission of cursor 2.
    let extra = tokio::time::timeout(Duration::from_millis(150), stream.next()).await;
    assert!(extra.is_err(), "cursor 2 was delivered twice: {extra:?}");
}
