//! End-to-end coverage of `HttpRpcClient` against a real
//! `codeless-server` running over an ephemeral loopback port. The
//! same `InProcessRpc` backs both sides, so any drift between the
//! HTTP wire and the in-process trait surface shows up here.
//!
//! `subscribe` is deferred to the next stage; the test asserts the
//! current stub error rather than skipping it, so when stage 2
//! flips the impl this test fails loud and stays useful.

use std::sync::Arc;

use codeless_adapters_host::HostFs;
use codeless_client::{HttpRpcClient, HttpRpcClientConfig};
use codeless_rpc::{
    AddRepoArgs, EventFilter, FsReadDirArgs, FsReadFileArgs, FsStatArgs, FsWriteFileArgs,
    ListJobsArgs, ListReviewsArgs, RpcError, RpcServer, StopJobArgs, SubmitJobArgs,
};
use codeless_runtime::InProcessRpc;
use codeless_server::{build_router, AppState};
use codeless_types::{Event, GitAuth, JobStatus, RepoId};
use futures_util::StreamExt;

const TOKEN: &str = "round-trip-token-xyz";

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn spawn_server() -> (HttpRpcClient, Arc<InProcessRpc>) {
    spawn_server_with(|r| r).await
}

async fn spawn_server_with(
    customize: impl FnOnce(InProcessRpc) -> InProcessRpc,
) -> (HttpRpcClient, Arc<InProcessRpc>) {
    let rpc = customize(InProcessRpc::new().await.expect("rpc init"));
    let rpc = Arc::new(rpc);
    let state = AppState::new(rpc.clone(), TOKEN);
    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = HttpRpcClient::new(HttpRpcClientConfig {
        base_url: format!("http://{addr}"),
        token: Some(TOKEN.into()),
    })
    .expect("client init");
    (client, rpc)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_then_list_repos_round_trip() {
    let (client, _server) = spawn_server().await;

    let listed = client.list_repos().await.expect("list_repos");
    assert!(listed.repos.is_empty());

    let repo = client
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
        .expect("add_repo");
    assert_eq!(repo.name, "demo");

    let listed = client.list_repos().await.unwrap();
    assert_eq!(listed.repos.len(), 1);
    assert_eq!(listed.repos[0].id, repo.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_then_stop_job_round_trip() {
    let (client, _server) = spawn_server().await;

    let repo = client
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

    let job = client
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("hi".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "feat/x".into(),
            workspace_mode: None,
            cost_cap_cents: 100,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            start_immediately: true,
        })
        .await
        .unwrap();
    assert_eq!(job.status, JobStatus::Queued);

    client
        .stop_job(StopJobArgs { job_id: job.id })
        .await
        .unwrap();

    let listed = client
        .list_jobs(ListJobsArgs { repo_id: None })
        .await
        .unwrap();
    let fetched = &listed.jobs[0];
    assert_eq!(fetched.status, JobStatus::Stopped);
    assert!(fetched.ended_at.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_repo_surfaces_as_not_found() {
    let (client, _server) = spawn_server().await;
    let bogus = RepoId::new();
    let err = client
        .submit_job(SubmitJobArgs {
            repo_id: bogus,
            prompt: Some("hi".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "main".into(),
            workspace_mode: None,
            cost_cap_cents: 0,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            start_immediately: true,
        })
        .await
        .expect_err("expected NotFound");
    assert!(matches!(err, RpcError::NotFound(_)), "got: {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_token_surfaces_as_internal_unauthorized() {
    let rpc = Arc::new(InProcessRpc::new().await.unwrap());
    let state = AppState::new(rpc, TOKEN);
    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let bad = HttpRpcClient::new(HttpRpcClientConfig {
        base_url: format!("http://{addr}"),
        token: Some("not-the-right-token".into()),
    })
    .unwrap();
    let err = bad.list_repos().await.expect_err("expected error");
    match err {
        RpcError::Internal(msg) => {
            assert!(
                msg.contains("401") || msg.contains("unauthorized"),
                "got: {msg}"
            );
        }
        other => panic!("expected Internal(unauthorized), got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_reviews_returns_empty() {
    let (client, _server) = spawn_server().await;
    let r = client
        .list_reviews(ListReviewsArgs::default())
        .await
        .unwrap();
    assert!(r.reviews.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_replays_persisted_events() {
    let (client, server) = spawn_server().await;
    let repo = server
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

    let mut stream = client
        .subscribe(EventFilter::All, Some(codeless_types::EventCursor(0)))
        .await
        .expect("subscribe");

    let env = tokio::time::timeout(std::time::Duration::from_secs(3), stream.next())
        .await
        .expect("timeout waiting for replay")
        .expect("stream ended")
        .expect("envelope");
    match env.event {
        Event::RepoAdded { repo_id } => assert_eq!(repo_id, repo.id),
        other => panic!("expected RepoAdded, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_streams_live_event() {
    let (client, server) = spawn_server().await;

    let mut stream = client
        .subscribe(EventFilter::All, None)
        .await
        .expect("subscribe");

    // Give the SSE handshake a beat to register with the EventBus
    // before publishing; otherwise the live tail can miss the event
    // and the test hangs.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    server
        .add_repo(AddRepoArgs {
            name: "live".into(),
            clone_url: "https://example.test/live.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/live".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();

    let env = tokio::time::timeout(std::time::Duration::from_secs(3), stream.next())
        .await
        .expect("timeout waiting for live event")
        .expect("stream ended")
        .expect("envelope");
    assert!(matches!(env.event, Event::RepoAdded { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_rejects_wrong_token() {
    let (_client, _server) = spawn_server().await;
    // Build a client with a bad token pointed at a fresh server so
    // the auth gate is the first thing it hits.
    let rpc = Arc::new(InProcessRpc::new().await.unwrap());
    let state = AppState::new(rpc, TOKEN);
    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let bad = HttpRpcClient::new(HttpRpcClientConfig {
        base_url: format!("http://{addr}"),
        token: Some("wrong".into()),
    })
    .unwrap();
    match bad.subscribe(EventFilter::All, None).await {
        Ok(_) => panic!("subscribe should fail with bad token"),
        Err(RpcError::Internal(msg)) => {
            assert!(msg.contains("401") || msg.contains("unauthorized"))
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn base_url_with_trailing_slash_rejected() {
    let r = HttpRpcClient::new(HttpRpcClientConfig {
        base_url: "http://example.test/".into(),
        token: None,
    });
    assert!(r.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_round_trip_through_http_client() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_owned();
    let (client, _server) =
        spawn_server_with(move |r| r.with_fs(Arc::new(HostFs::new(&root).unwrap()))).await;

    client
        .fs_write_file(FsWriteFileArgs {
            path: "doc.md".into(),
            content: "hello".into(),
        })
        .await
        .expect("write");

    let read = client
        .fs_read_file(FsReadFileArgs {
            path: "doc.md".into(),
        })
        .await
        .expect("read");
    assert_eq!(read.content, "hello");

    let dir = client
        .fs_read_dir(FsReadDirArgs { path: ".".into() })
        .await
        .expect("read_dir");
    let names: Vec<_> = dir.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["doc.md"]);

    let stat = client
        .fs_stat(FsStatArgs {
            path: "doc.md".into(),
        })
        .await
        .expect("stat");
    assert!(stat.kind.is_some());
    assert_eq!(stat.size, Some(5));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_traversal_surfaces_as_invalid_argument() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_owned();
    let (client, _server) =
        spawn_server_with(move |r| r.with_fs(Arc::new(HostFs::new(&root).unwrap()))).await;

    let err = client
        .fs_read_file(FsReadFileArgs {
            path: "../etc/passwd".into(),
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, RpcError::InvalidArgument(_)),
        "expected InvalidArgument, got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_without_root_surfaces_as_internal() {
    let (client, _server) = spawn_server().await;
    let err = client
        .fs_read_dir(FsReadDirArgs { path: ".".into() })
        .await
        .unwrap_err();
    assert!(
        matches!(err, RpcError::Internal(_)),
        "expected Internal, got {err:?}"
    );
}
