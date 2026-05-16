//! `AnthropicRunnerAdapter` against a `wiremock` stand-in for
//! api.anthropic.com. Pins SCOPE.md "Testing strategy" — REST
//! runners are tested against a faked API, never the live one.
//!
//! The Anthropic Messages SSE protocol is hand-crafted in the
//! `text/event-stream` response body: `message_start` → assistant
//! `content_block_*` text deltas → `message_delta` with output-token
//! usage → `message_stop`. The whole chain exercises:
//!   wiremock SSE → anthropic-ai-sdk → ai_runner::AnthropicRunner →
//!   `ai_runner::Event` mpsc → `ai_runner_bridge::forward_events` →
//!   `EventBus::publish` → `codeless_types::Event` on the bus.

use std::sync::Arc;
use std::time::Duration;

use codeless_rpc::{AddRepoArgs, EventFilter, RpcServer, SubmitJobArgs};
use codeless_runtime::{drive_job, AnthropicRunnerAdapter, InProcessRpc};
use codeless_types::{Event, GitAuth, JobStatus, TaskId};
use futures_util::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse_frame(event: &str, data: &str) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// Hand-crafted Anthropic Messages SSE body. Mirrors the on-wire shape
/// of the live API closely enough that `anthropic-ai-sdk`'s `StreamEvent`
/// deserialisation accepts every frame, exercising token-count
/// extraction (`input_tokens` from `message_start`, `output_tokens`
/// from `message_delta`).
fn sse_body(input_tokens: u32, output_tokens: u32) -> String {
    let message_start = format!(
        r#"{{"type":"message_start","message":{{"id":"msg_test","type":"message","role":"assistant","content":[],"model":"claude-opus-4-5","stop_reason":null,"stop_sequence":null,"usage":{{"input_tokens":{input_tokens},"output_tokens":0}}}}}}"#
    );
    let content_block_start =
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
    let delta_hello =
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello "}}"#;
    let delta_world =
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world"}}"#;
    let content_block_stop = r#"{"type":"content_block_stop","index":0}"#;
    let message_delta = format!(
        r#"{{"type":"message_delta","delta":{{"stop_reason":"end_turn","stop_sequence":null}},"usage":{{"output_tokens":{output_tokens}}}}}"#
    );
    let message_stop = r#"{"type":"message_stop"}"#;
    [
        sse_frame("message_start", &message_start),
        sse_frame("content_block_start", content_block_start),
        sse_frame("content_block_delta", delta_hello),
        sse_frame("content_block_delta", delta_world),
        sse_frame("content_block_stop", content_block_stop),
        sse_frame("message_delta", &message_delta),
        sse_frame("message_stop", message_stop),
    ]
    .concat()
}

async fn submit(rpc: &InProcessRpc) -> codeless_types::JobId {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/codeless-demo-not-used".into(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("hi".into()),
        template_yaml: None,
        runner: "anthropic".into(),
        branch: "codeless/job-anthropic".into(),
        workspace_mode: None,
        cost_cap_cents: 500,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        auto_bypass_policy: None,
        start_immediately: true,
    })
    .await
    .unwrap()
    .id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anthropic_runner_streams_tokens_through_bridge() {
    let mock = MockServer::start().await;
    let body = sse_body(42, 17);
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(body.into_bytes(), "text/event-stream"),
        )
        .mount(&mock)
        .await;

    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = submit(&rpc).await;

    let mut stream = rpc
        .subscribe(EventFilter::Job { job_id }, None)
        .await
        .unwrap();

    let task_id = TaskId::new();
    let mut adapter = AnthropicRunnerAdapter::new("hi", task_id);
    adapter.api_key = Some("test-key".into());
    adapter.base_url = Some(mock.uri());
    let adapter: Arc<dyn codeless_runtime::Runner> = Arc::new(adapter);

    drive_job(&rpc, job_id, adapter, None).await.unwrap();

    let events = tokio::time::timeout(Duration::from_secs(5), async {
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            let env = item.expect("stream error");
            let terminal = matches!(env.event, Event::JobCompleted { .. });
            out.push(env.event);
            if terminal {
                return out;
            }
        }
        out
    })
    .await
    .expect("timed out");

    let mut text = String::new();
    let mut complete_in = 0i64;
    let mut complete_out = 0i64;
    let mut saw_complete = false;
    let mut saw_started = false;
    let mut saw_terminal = false;
    for ev in &events {
        match ev {
            Event::JobStarted { .. } => saw_started = true,
            Event::AiToken { task_id: t, delta } if *t == task_id => text.push_str(delta),
            Event::AiMessageComplete {
                task_id: t,
                input_tokens,
                output_tokens,
                ..
            } if *t == task_id => {
                saw_complete = true;
                complete_in = *input_tokens;
                complete_out = *output_tokens;
            }
            Event::JobCompleted { .. } => saw_terminal = true,
            _ => {}
        }
    }
    assert!(saw_started, "missing JobStarted; got {events:#?}");
    assert!(saw_complete, "missing AiMessageComplete; got {events:#?}");
    assert!(saw_terminal, "missing JobCompleted; got {events:#?}");
    assert_eq!(text, "hello world", "text deltas land in order");
    assert_eq!(complete_in, 42, "input_tokens flow from message_start");
    assert_eq!(complete_out, 17, "output_tokens flow from message_delta");

    let job_row = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .unwrap();
    assert_eq!(job_row.status, JobStatus::Completed);
}
