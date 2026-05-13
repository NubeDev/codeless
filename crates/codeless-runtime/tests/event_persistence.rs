//! Every `EventBus::publish` must:
//! - leave one row in the `events` table with the correct
//!   `type` discriminator and the variant fields under `payload`,
//! - hand out a monotonic cursor from the autoincrement column,
//! - still broadcast to live subscribers (the persistence half does
//!   not replace the live-tail half — both happen).

use codeless_rpc::{AddRepoArgs, EventFilter, RpcServer, SubmitJobArgs};
use codeless_runtime::InProcessRpc;
use codeless_types::{Event, GitAuth};
use futures_util::StreamExt;
use sqlx::Row;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_repo_persists_repo_added_with_cursor_one() {
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

    let rows = sqlx::query("SELECT cursor, type, payload FROM events ORDER BY cursor")
        .fetch_all(rpc.pool())
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let cursor: i64 = rows[0].get("cursor");
    let typ: String = rows[0].get("type");
    let payload: String = rows[0].get("payload");
    assert_eq!(cursor, 1);
    assert_eq!(typ, "repo-added");
    let payload_obj: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(
        payload_obj["repo_id"].as_str().unwrap(),
        repo.id.to_string()
    );
    assert!(
        payload_obj.get("type").is_none(),
        "type must not leak into payload"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cursors_are_strictly_monotonic_across_publish_calls() {
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
    let _ = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("hi".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/job-x".into(),
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            start_immediately: true,
        })
        .await
        .unwrap();

    let cursors: Vec<i64> = sqlx::query("SELECT cursor FROM events ORDER BY cursor")
        .fetch_all(rpc.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.get("cursor"))
        .collect();
    assert_eq!(cursors, vec![1, 2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_subscribers_still_receive_after_persistence() {
    let rpc = InProcessRpc::new().await.unwrap();
    let mut stream = rpc.subscribe(EventFilter::All, None).await.unwrap();
    rpc.add_repo(AddRepoArgs {
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
    let env = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("stream timeout")
        .expect("stream end")
        .expect("stream error");
    assert!(matches!(env.event, Event::RepoAdded { .. }));
    assert_eq!(env.cursor.0, 1);
}
