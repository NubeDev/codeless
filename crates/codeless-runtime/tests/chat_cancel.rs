//! Chat-cancel registry: the `agent_chat` spawn registers a per-turn
//! `CancellationToken` on the runtime so an out-of-band
//! `cancel_chat_task` RPC can fire it. The registry is in-memory and
//! single-tenant; entries are removed by a drop-guard on the spawned
//! task so successful turns never leak. `cancel_chat_task` is
//! idempotent — a missing entry (the turn already ended) is `Ok(())`.

use codeless_rpc::{CancelChatTaskArgs, RpcServer};
use codeless_runtime::InProcessRpc;
use codeless_types::TaskId;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn cancel_chat_task_fires_registered_token() {
    let rpc = InProcessRpc::new().await.expect("rpc");
    let task_id = TaskId::new();
    let token = CancellationToken::new();
    rpc.chat_cancels().lock().insert(task_id, token.clone());

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
    let task_id = TaskId::new();
    let token = CancellationToken::new();
    rpc.chat_cancels().lock().insert(task_id, token.clone());

    rpc.cancel_chat_task(CancelChatTaskArgs { task_id })
        .await
        .unwrap();
    rpc.cancel_chat_task(CancelChatTaskArgs { task_id })
        .await
        .unwrap();

    assert!(token.is_cancelled());
    assert!(rpc.chat_cancels().lock().contains_key(&task_id));
}
