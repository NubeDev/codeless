//! Integration tests for the per-Job chat substrate forwarder. The
//! tests stand up a real `InProcessRpc`, a real `ChatForwarder`, and
//! a `wiremock`-backed Telegram Bot API endpoint, then drive the
//! three behaviour bullets that JOB-CHAT.md "Transport adapters"
//! pins:
//!
//!   - Web-origin (non-Telegram) → forwarded to the bound channel
//!     and the delivery receipt lands on
//!     `chat_messages.metadata_json.delivery.telegram`.
//!   - Telegram-origin (the same row Telegram already sees in its
//!     own client) → NOT forwarded (echo suppression for the origin
//!     transport).
//!   - A message whose stored row already carries
//!     `metadata.delivery.telegram` (a prior forwarder boot already
//!     delivered) → NOT forwarded (presence-based idempotency).

use std::sync::Arc;
use std::time::Duration;

use codeless_bot_core::EventSource;
use codeless_rpc::{BindChatThreadArgs, PostJobMessageArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::rpc::InProcessRpc;
use codeless_telegram::chat::ChatForwarder;
use codeless_telegram::web_api::TelegramApi;
use codeless_types::{ChatRole, ChatTransport, GitAuth, JobId};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

async fn fresh_rpc_with_bound_job(channel: &str) -> (Arc<InProcessRpc>, JobId) {
    let rpc = Arc::new(InProcessRpc::new().await.unwrap());
    let repo = rpc
        .add_repo(codeless_rpc::AddRepoArgs {
            name: "r".into(),
            clone_url: "u".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: GitAuth::Ssh {
                key_path: "/tmp/k".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    let job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("hi".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "b".into(),
            workspace_mode: None,
            cost_cap_cents: 0,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            start_immediately: false,
        })
        .await
        .unwrap();
    rpc.bind_chat_thread(BindChatThreadArgs {
        transport: ChatTransport::Telegram,
        channel_id: channel.to_string(),
        thread_id: None,
        job_id: job.id,
        bound_by: "@alice".into(),
    })
    .await
    .unwrap();
    (rpc, job.id)
}

fn api_against(server: &MockServer) -> TelegramApi {
    TelegramApi::new_with_client(Arc::new(reqwest::Client::new()), "12345:test-secret")
        .with_base_url(server.uri())
}

/// Local `EventSource` over the runtime bus — bypasses the `subscribe`
/// RPC and the in-process loopback shape. The forwarder code only
/// needs `subscribe_all`; this stub is the smallest thing that
/// satisfies the trait without dragging the full `RpcServer`
/// machinery into the test setup.
struct BusEventSource {
    bus: Arc<codeless_runtime::event_bus::EventBus>,
}

#[async_trait::async_trait]
impl EventSource for BusEventSource {
    async fn subscribe_all(&self) -> codeless_rpc::RpcResult<codeless_rpc::EventStream> {
        let local = codeless_runtime::event_bus::SubscribeFilter::All;
        Ok(self
            .bus
            .subscribe_since(local, None)
            .await
            .map_err(|e| codeless_rpc::RpcError::Internal(format!("bus: {e}")))?)
    }
}

fn events_from(rpc: &Arc<InProcessRpc>) -> Arc<dyn EventSource> {
    Arc::new(BusEventSource {
        bus: rpc.bus().clone(),
    })
}

async fn metadata_delivery(rpc: &Arc<InProcessRpc>, job_id: JobId) -> Vec<Value> {
    let listed = rpc
        .list_job_messages(codeless_rpc::ListJobMessagesArgs {
            job_id,
            before: None,
            limit: 100,
        })
        .await
        .unwrap();
    listed
        .messages
        .into_iter()
        .map(|m| {
            m.metadata_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null)
        })
        .collect()
}

async fn await_send_count(server: &MockServer, expected: usize) {
    // Poll for the expected number of sendMessage hits rather than
    // sleeping a fixed window. The forwarder is event-driven, so the
    // loop exits as soon as the receipt write has rippled through;
    // the 1s ceiling is generous enough to absorb scheduler jitter
    // without making the test slow.
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        let count = server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.url.path().ends_with("/sendMessage"))
            .count();
        if count >= expected {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("expected at least {expected} sendMessage hits, got {count}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn expect_no_send(server: &MockServer) {
    // Settle a beat so a late-arriving send would have a chance to
    // land; then assert nothing fired. 100ms matches the same pattern
    // the bot-core publisher tests use for their no-post assertions.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let count = server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/sendMessage"))
        .count();
    assert_eq!(count, 0, "expected zero sendMessage hits, got {count}");
}

#[tokio::test]
async fn web_origin_message_is_forwarded_and_receipt_written() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bot12345:test-secret/sendMessage"))
        .respond_with(|req: &Request| {
            let body: Value = req.body_json().expect("json");
            assert_eq!(body["chat_id"], "C-tg");
            ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": { "message_id": 1001i64, "chat": { "id": 0i64 } }
            }))
        })
        .mount(&server)
        .await;

    let (rpc, job_id) = fresh_rpc_with_bound_job("C-tg").await;
    let rpc_dyn: Arc<dyn RpcServer> = rpc.clone();
    let forwarder = ChatForwarder::spawn(events_from(&rpc), rpc_dyn, api_against(&server));

    // Web-origin: not the Telegram client, so the forwarder must
    // deliver into the bound channel.
    rpc.post_job_message(PostJobMessageArgs {
        job_id,
        transport: ChatTransport::Web,
        external_id: None,
        thread_key: None,
        author: "alice".into(),
        role: ChatRole::User,
        body: "hello from web".into(),
        metadata_json: None,
    })
    .await
    .unwrap();

    await_send_count(&server, 1).await;

    // The receipt has to be on the row by the time the forwarder
    // settles. A short poll is cleaner than another fixed sleep.
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        let meta = metadata_delivery(&rpc, job_id).await;
        if meta.iter().any(|m| m["delivery"]["telegram"] == "1001") {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("receipt did not land on metadata.delivery.telegram: {meta:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    forwarder.shutdown().await;
}

#[tokio::test]
async fn telegram_origin_message_is_not_echoed_back_to_telegram() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bot12345:test-secret/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": { "message_id": 9999i64, "chat": { "id": 0i64 } }
        })))
        .mount(&server)
        .await;

    let (rpc, job_id) = fresh_rpc_with_bound_job("C-tg").await;
    let rpc_dyn: Arc<dyn RpcServer> = rpc.clone();
    let forwarder = ChatForwarder::spawn(events_from(&rpc), rpc_dyn, api_against(&server));

    // Telegram-origin: the platform already shows this row to the
    // user in their client; the forwarder must skip rather than
    // re-post and double-render.
    rpc.post_job_message(PostJobMessageArgs {
        job_id,
        transport: ChatTransport::Telegram,
        external_id: Some("tg:42".into()),
        thread_key: Some("C-tg".into()),
        author: "tg-user-42".into(),
        role: ChatRole::User,
        body: "hi from telegram client".into(),
        metadata_json: None,
    })
    .await
    .unwrap();

    expect_no_send(&server).await;
    forwarder.shutdown().await;
}

#[tokio::test]
async fn message_with_existing_delivery_receipt_is_not_re_forwarded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bot12345:test-secret/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": { "message_id": 9999i64, "chat": { "id": 0i64 } }
        })))
        .mount(&server)
        .await;

    let (rpc, job_id) = fresh_rpc_with_bound_job("C-tg").await;
    let rpc_dyn: Arc<dyn RpcServer> = rpc.clone();
    let forwarder = ChatForwarder::spawn(events_from(&rpc), rpc_dyn, api_against(&server));

    // Pre-armed receipt: a prior forwarder boot already delivered
    // and wrote `metadata.delivery.telegram`. Presence-based
    // idempotency must skip the re-send.
    rpc.post_job_message(PostJobMessageArgs {
        job_id,
        transport: ChatTransport::Web,
        external_id: None,
        thread_key: None,
        author: "alice".into(),
        role: ChatRole::User,
        body: "already delivered".into(),
        metadata_json: Some(r#"{"delivery":{"telegram":"prior-id"}}"#.into()),
    })
    .await
    .unwrap();

    expect_no_send(&server).await;
    forwarder.shutdown().await;
}
