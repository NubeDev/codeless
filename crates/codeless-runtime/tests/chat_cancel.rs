//! Chat-cancel registry: the `agent_chat` spawn registers a per-turn
//! `CancellationToken` on the runtime so an out-of-band
//! `cancel_chat_task` RPC can fire it. The registry is in-memory and
//! single-tenant; entries are removed by a drop-guard on the spawned
//! task so successful turns never leak. `cancel_chat_task` is
//! idempotent — a missing entry (the turn already ended) is `Ok(())`.

use codeless_rpc::{AddRepoArgs, CancelChatTaskArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::{ChatCancelEntry, InProcessRpc};
use codeless_types::{GitAuth, JobId, TaskId};
use tokio_util::sync::CancellationToken;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn seed_job(rpc: &InProcessRpc) -> codeless_types::Job {
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
        prompt: Some("hi".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "codeless/chat-cancel".into(),
        workspace_mode: None,
        cost_cap_cents: 100,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        start_immediately: false,
    })
    .await
    .unwrap()
}

fn insert_entry(rpc: &InProcessRpc, task_id: TaskId, job_id: JobId) -> CancellationToken {
    let token = CancellationToken::new();
    rpc.chat_cancels().lock().insert(
        task_id,
        ChatCancelEntry {
            job_id,
            token: token.clone(),
        },
    );
    token
}

#[tokio::test]
async fn cancel_chat_task_fires_registered_token() {
    let rpc = InProcessRpc::new().await.expect("rpc");
    let job = seed_job(&rpc).await;
    let task_id = TaskId::new();
    let token = insert_entry(&rpc, task_id, job.id);

    rpc.cancel_chat_task(CancelChatTaskArgs { task_id })
        .await
        .expect("cancel_chat_task");

    assert!(token.is_cancelled(), "token should have been fired");
}

#[tokio::test]
async fn cancel_chat_task_is_idempotent_for_unknown_task() {
    // The drop-guard on `agent_chat` removes the entry once the
    // spawned task ends; the UI may issue stop after the natural end
    // of the stream, so an unknown id must not surface as an error.
    let rpc = InProcessRpc::new().await.expect("rpc");
    let task_id = TaskId::new();

    rpc.cancel_chat_task(CancelChatTaskArgs { task_id })
        .await
        .expect("cancel_chat_task on unknown id should be Ok");
}

#[tokio::test]
async fn cancel_chat_task_does_not_remove_entry() {
    // The drop-guard owns removal so the registry's lifecycle stays
    // in one place. A second `cancel` against the same id is still
    // a no-op fire (the token is already cancelled) and still leaves
    // the entry around for the spawned task's drop-guard to evict.
    let rpc = InProcessRpc::new().await.expect("rpc");
    let job = seed_job(&rpc).await;
    let task_id = TaskId::new();
    let token = insert_entry(&rpc, task_id, job.id);

    rpc.cancel_chat_task(CancelChatTaskArgs { task_id })
        .await
        .unwrap();
    rpc.cancel_chat_task(CancelChatTaskArgs { task_id })
        .await
        .unwrap();

    assert!(token.is_cancelled());
    assert!(rpc.chat_cancels().lock().contains_key(&task_id));
}
