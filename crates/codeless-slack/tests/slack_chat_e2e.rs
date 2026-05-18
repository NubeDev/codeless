//! End-to-end behaviour tests for the per-Job chat substrate's
//! asymmetric echo-suppression rule, run through the real Slack
//! [`ChatForwarder`] against an in-memory event bus and a canned
//! Slack Web API stub. Direct mirror of
//! `codeless-telegram/tests/bot_chat_e2e.rs` — the two adapters share
//! the [`codeless_bot_core::chat_forward::classify`] helper, so this
//! file exists to pin that the Slack-side wiring really threads
//! through it (rather than re-deriving the rule and silently drifting
//! from the Telegram side).
//!
//!   - [`origin_transport_skips_self_post`] — a row whose `transport`
//!     equals this forwarder's transport must NOT round-trip back to
//!     the channel. The user already sees the message in their own
//!     Slack client and a re-post would double-render.
//!   - [`cross_transport_forwards_with_receipt`] — a row from any
//!     other surface must reach the channel and the success path
//!     must write `metadata_json.delivery.slack` so a process
//!     restart that re-observes the same `ChatMessageAppended`
//!     skips on receipt presence.
//!
//! Both tests use the existing `InProcessRpc` event bus as the
//! "in-memory bus" rather than standing up a second one — that bus
//! is the production seam and reusing it gives the test the same
//! ordering guarantees the live forwarder gets.
//!
//! The [`CannedSlackApi`] helper is the smallest possible substitute
//! for the real Slack Web API: a wiremock instance preloaded with one
//! `chat.postMessage` matcher that returns a constant `ts`. It
//! records every request so the test can assert call counts without
//! depending on real network I/O.

use std::sync::Arc;
use std::time::Duration;

use codeless_bot_core::EventSource;
use codeless_rpc::{BindChatThreadArgs, PostJobMessageArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::rpc::InProcessRpc;
use codeless_slack::chat::ChatForwarder;
use codeless_slack::web_api::ChatPoster;
use codeless_types::{ChatRole, ChatTransport, GitAuth, JobId};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Wraps a `wiremock::MockServer` preloaded with a canned
/// `chat.postMessage` response. The Slack surface used by the
/// forwarder is one method (`chat.postMessage`); stubbing the rest
/// would be dead weight here.
///
/// The "canned" framing matches the stage's spec wording: tests do
/// not exercise the real Slack Web API, just the contract the
/// forwarder relies on (200 OK + string `ts` that ends up in the
/// delivery receipt).
struct CannedSlackApi {
    server: MockServer,
}

impl CannedSlackApi {
    async fn start() -> Self {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "channel": "C-sl",
                "ts": "1700.4242"
            })))
            .mount(&server)
            .await;
        Self { server }
    }

    fn poster(&self) -> ChatPoster {
        ChatPoster::new(Arc::new(reqwest::Client::new()), "xoxb-test")
            .with_base_url(self.server.uri() + "/api")
    }

    async fn send_count(&self) -> usize {
        self.server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.url.path().ends_with("/chat.postMessage"))
            .count()
    }
}

/// Local `EventSource` over the runtime bus. The forwarder only
/// needs `subscribe_all`; this is the same shape the Telegram-side
/// e2e test uses (kept independent rather than coupling them through
/// a shared helper module — both adapters should stay grokkable in
/// isolation).
struct BusEventSource {
    bus: Arc<codeless_runtime::event_bus::EventBus>,
}

#[async_trait::async_trait]
impl EventSource for BusEventSource {
    async fn subscribe_all(&self) -> codeless_rpc::RpcResult<codeless_rpc::EventStream> {
        let filter = codeless_runtime::event_bus::SubscribeFilter::All;
        Ok(self
            .bus
            .subscribe_since(filter, None)
            .await
            .map_err(|e| codeless_rpc::RpcError::Internal(format!("bus: {e}")))?)
    }
}

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
        transport: ChatTransport::Slack,
        channel_id: channel.to_string(),
        thread_id: None,
        job_id: job.id,
        bound_by: "@alice".into(),
    })
    .await
    .unwrap();
    (rpc, job.id)
}

fn events_from(rpc: &Arc<InProcessRpc>) -> Arc<dyn EventSource> {
    Arc::new(BusEventSource {
        bus: rpc.bus().clone(),
    })
}

async fn metadata_for_first_message(rpc: &Arc<InProcessRpc>, job_id: JobId) -> Value {
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
        .next()
        .and_then(|m| m.metadata_json)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null)
}

/// Slack-origin row → the forwarder's classify helper returns
/// `Skip`, so wiremock never sees a `chat.postMessage`. This pins
/// the origin-transport branch of the asymmetric rule against the
/// live loop, not just the pure helper.
#[tokio::test]
async fn origin_transport_skips_self_post() {
    let canned = CannedSlackApi::start().await;
    let (rpc, job_id) = fresh_rpc_with_bound_job("C-sl").await;
    let rpc_dyn: Arc<dyn RpcServer> = rpc.clone();
    let forwarder = ChatForwarder::spawn(events_from(&rpc), rpc_dyn, canned.poster());

    rpc.post_job_message(PostJobMessageArgs {
        job_id,
        transport: ChatTransport::Slack,
        external_id: Some("sl:1700.0001".into()),
        thread_key: Some("C-sl".into()),
        author: "sl-user-42".into(),
        role: ChatRole::User,
        body: "hi from slack client".into(),
        metadata_json: None,
    })
    .await
    .unwrap();

    // 100ms settle to give a hypothetical mis-classification the
    // chance to actually land in wiremock — without it the test
    // would pass even if the forwarder were broken in the direction
    // of fast-firing the send.
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        canned.send_count().await,
        0,
        "slack-origin row must not be echoed back to slack",
    );
    forwarder.shutdown().await;
}

/// Cross-transport row → the forwarder posts to wiremock once and
/// writes `metadata_json.delivery.slack = "<canned ts>"` back onto
/// the row, so a subsequent boot that re-observes the same envelope
/// skips on receipt presence.
#[tokio::test]
async fn cross_transport_forwards_with_receipt() {
    let canned = CannedSlackApi::start().await;
    let (rpc, job_id) = fresh_rpc_with_bound_job("C-sl").await;
    let rpc_dyn: Arc<dyn RpcServer> = rpc.clone();
    let forwarder = ChatForwarder::spawn(events_from(&rpc), rpc_dyn, canned.poster());

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

    // Poll until the post hits wiremock and the receipt round-trip
    // settles. A 1s ceiling absorbs scheduler jitter without making
    // the test slow.
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        let sent = canned.send_count().await;
        let meta = metadata_for_first_message(&rpc, job_id).await;
        if sent == 1 && meta["delivery"]["slack"] == "1700.4242" {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("cross-transport forward did not settle: sent={sent}, metadata={meta:?}",);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    forwarder.shutdown().await;
}
