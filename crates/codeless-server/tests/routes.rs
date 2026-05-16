//! Route-level coverage for the axum surface in `codeless-server`.
//! Every REST method is exercised end-to-end through `tower::oneshot`
//! against a real `InProcessRpc` backed by an in-memory SQLite — the
//! same backend the CLI uses — so the test catches drift between the
//! wire shape `http-sse-client.ts` sends and what the router expects.
//!
//! The SSE smoke test at the bottom proves the live tail path: a
//! subscriber connects, the runtime emits an event, and the SSE
//! stream surfaces it in a `data:` frame. Replay-from-cursor is
//! covered by the runtime's own `since_replay` test; here we only
//! need to know the SSE adapter wires the stream through.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use codeless_rpc::{
    AddRepoArgs, ListReposResult, RpcServer, RunnerInfo, ServerInfo, SubmitJobArgs,
};
use codeless_runtime::InProcessRpc;
use codeless_server::{build_router, AppState};
use codeless_types::{GitAuth, Job, Repo};
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "test-token-0123456789";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_and_version_are_unauthenticated() {
    let (app, _) = fresh_app().await;
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/healthz")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
    assert_eq!(&body[..], b"ok");

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/version")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_info_returns_configured_snapshot_without_token() {
    let rpc = Arc::new(InProcessRpc::new().await.expect("rpc init"));
    let info = ServerInfo {
        version: "test-v".into(),
        runners: vec![
            RunnerInfo {
                id: "mock".into(),
                default: false,
            },
            RunnerInfo {
                id: "claude".into(),
                default: true,
            },
        ],
        fs_root: Some("/tmp/demo".into()),
        worktree_root: Some("/tmp/demo/.codeless/worktrees".into()),
        claude: None,
        available_cli_runners: Vec::new(),
        feature_flags: Default::default(),
    };
    let state = AppState::new(rpc, TOKEN).with_server_info(info.clone());
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/server/info")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let got: ServerInfo = serde_json::from_value(body_json(resp).await).unwrap();
    assert_eq!(got, info);
}

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn fresh_app() -> (axum::Router, Arc<InProcessRpc>) {
    let rpc = Arc::new(InProcessRpc::new().await.expect("rpc init"));
    let state = AppState::new(rpc.clone(), TOKEN);
    let app = build_router(state);
    (app, rpc)
}

fn post(path: &str, body: Value, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_bearer_is_unauthorized() {
    let (app, _) = fresh_app().await;
    let resp = app
        .oneshot(post("/rpc/list_repos", json!({}), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_bearer_is_unauthorized() {
    let (app, _) = fresh_app().await;
    let resp = app
        .oneshot(post("/rpc/list_repos", json!({}), Some("nope")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_repos_accepts_empty_object_body() {
    let (app, _) = fresh_app().await;
    let resp = app
        .oneshot(post("/rpc/list_repos", json!({}), Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: ListReposResult = serde_json::from_value(body_json(resp).await).unwrap();
    assert!(v.repos.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_repo_round_trips_via_list_repos() {
    let (app, _) = fresh_app().await;

    let add_body = serde_json::to_value(AddRepoArgs {
        name: "demo".into(),
        clone_url: "https://example.test/demo.git".into(),
        default_branch: "main".into(),
        local_path: "/tmp/demo".into(),
        git_auth: token_auth(),
        concurrency_cap: None,
        default_runner: None,
    })
    .unwrap();

    let resp = app
        .clone()
        .oneshot(post("/rpc/add_repo", add_body, Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let repo: Repo = serde_json::from_value(body_json(resp).await).unwrap();
    assert_eq!(repo.name, "demo");

    let resp = app
        .oneshot(post("/rpc/list_repos", json!({}), Some(TOKEN)))
        .await
        .unwrap();
    let listed: ListReposResult = serde_json::from_value(body_json(resp).await).unwrap();
    assert_eq!(listed.repos.len(), 1);
    assert_eq!(listed.repos[0].id, repo.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_job_unknown_repo_maps_to_404() {
    let (app, _) = fresh_app().await;
    let bogus = codeless_types::RepoId::new();
    let body = serde_json::to_value(SubmitJobArgs {
        repo_id: bogus,
        prompt: Some("hi".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "main".into(),
        workspace_mode: None,
        cost_cap_cents: 100,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        auto_bypass_policy: None,
        start_immediately: true,
    })
    .unwrap();

    let resp = app
        .oneshot(post("/rpc/submit_job", body, Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_then_get_job_round_trip() {
    let (app, _) = fresh_app().await;

    let add_body = serde_json::to_value(AddRepoArgs {
        name: "demo".into(),
        clone_url: "https://example.test/demo.git".into(),
        default_branch: "main".into(),
        local_path: "/tmp/demo".into(),
        git_auth: token_auth(),
        concurrency_cap: None,
        default_runner: None,
    })
    .unwrap();
    let resp = app
        .clone()
        .oneshot(post("/rpc/add_repo", add_body, Some(TOKEN)))
        .await
        .unwrap();
    let repo: Repo = serde_json::from_value(body_json(resp).await).unwrap();

    let submit_body = serde_json::to_value(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("hi".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "feat/x".into(),
        workspace_mode: None,
        cost_cap_cents: 200,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        auto_bypass_policy: None,
        start_immediately: true,
    })
    .unwrap();
    let resp = app
        .clone()
        .oneshot(post("/rpc/submit_job", submit_body, Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let job: Job = serde_json::from_value(body_json(resp).await).unwrap();
    assert_eq!(job.repo_id, repo.id);

    let get_body = json!({ "job_id": job.id });
    let resp = app
        .oneshot(post("/rpc/get_job", get_body, Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let fetched: Job = serde_json::from_value(body_json(resp).await).unwrap();
    assert_eq!(fetched.id, job.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_unknown_job_maps_to_404() {
    let (app, _) = fresh_app().await;
    let bogus = codeless_types::JobId::new();
    let resp = app
        .oneshot(post(
            "/rpc/stop_job",
            json!({ "job_id": bogus }),
            Some(TOKEN),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_reviews_empty_returns_empty_array() {
    let (app, _) = fresh_app().await;
    let resp = app
        .oneshot(post("/rpc/list_reviews", json!({}), Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = body_json(resp).await;
    assert_eq!(v["reviews"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approve_unknown_review_maps_to_404() {
    let (app, _) = fresh_app().await;
    let bogus = codeless_types::ReviewId::new();
    let resp = app
        .oneshot(post(
            "/rpc/approve_review",
            json!({ "review_id": bogus }),
            Some(TOKEN),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_sse_streams_live_event() {
    use std::net::SocketAddr;

    let (app, rpc) = fresh_app().await;

    // Bind to an ephemeral port — Sse responses stream until the body
    // is dropped, and tower::oneshot fully buffers the body before
    // returning. Run the router on a real listener so we can read the
    // SSE frame as it arrives without deadlock.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!(
        "http://{addr}/events?scope=all&token={token}",
        token = TOKEN
    );

    let read_handle = tokio::spawn(async move {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let req = format!(
            "GET /events?scope=all&token={TOKEN} HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await.unwrap();

        let mut buf = vec![0u8; 8192];
        let mut acc = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                panic!("timed out waiting for SSE event; got: {acc:?}");
            }
            let n = tokio::time::timeout(remaining, stream.read(&mut buf))
                .await
                .expect("read timeout")
                .expect("read err");
            if n == 0 {
                panic!("connection closed before event: {acc:?}");
            }
            acc.push_str(&String::from_utf8_lossy(&buf[..n]));
            if acc.contains("repo-added") {
                return acc;
            }
        }
    });

    // Give the SSE reader a beat to send the request and the server a
    // beat to register the subscriber before the event publishes;
    // otherwise the live tail will miss it and the test hangs out.
    tokio::time::sleep(Duration::from_millis(150)).await;

    rpc.add_repo(AddRepoArgs {
        name: "sse-demo".into(),
        clone_url: "https://example.test/sse.git".into(),
        default_branch: "main".into(),
        local_path: "/tmp/sse".into(),
        git_auth: token_auth(),
        concurrency_cap: None,
        default_runner: None,
    })
    .await
    .expect("add_repo");

    let acc = read_handle.await.expect("reader task");
    assert!(acc.contains("data:"), "no SSE data frame in: {acc}");
    assert!(acc.contains("repo-added"));

    server.abort();
    let _ = url; // suppress unused warning if logging is added later
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_round_trip_through_router() {
    use codeless_adapters_host::HostFs;
    use tempfile::tempdir;

    let tmp = tempdir().unwrap();
    let rpc = Arc::new(
        InProcessRpc::new()
            .await
            .expect("rpc init")
            .with_fs(Arc::new(HostFs::new(tmp.path()).unwrap())),
    );
    let state = AppState::new(rpc, TOKEN);
    let app = build_router(state);

    let write = app
        .clone()
        .oneshot(post(
            "/rpc/fs_write_file",
            json!({ "path": "hello.txt", "content": "world" }),
            Some(TOKEN),
        ))
        .await
        .unwrap();
    assert_eq!(write.status(), StatusCode::OK);

    let read = app
        .clone()
        .oneshot(post(
            "/rpc/fs_read_file",
            json!({ "path": "hello.txt" }),
            Some(TOKEN),
        ))
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    let body = body_json(read).await;
    assert_eq!(body["content"], "world");

    let escape = app
        .oneshot(post(
            "/rpc/fs_read_file",
            json!({ "path": "../etc/passwd" }),
            Some(TOKEN),
        ))
        .await
        .unwrap();
    assert_eq!(escape.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_without_root_returns_internal() {
    let (app, _) = fresh_app().await;
    let resp = app
        .oneshot(post(
            "/rpc/fs_read_dir",
            json!({ "path": "." }),
            Some(TOKEN),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
