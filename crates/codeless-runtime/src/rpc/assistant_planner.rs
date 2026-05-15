//! F2 planner: synthesise an assistant turn by dispatching the
//! thread's history through the same `run_chat` path the in-editor
//! AI panel uses. Tokens stream onto the bus tagged with
//! `(thread_id-as-job_id, task_id)` so a subscriber sees deltas
//! land live; the concatenated text is returned for the caller
//! (`append_assistant_message`) to persist as the row's content with
//! `meta_json = None` — chat replies are not action cards.
//!
//! Kept in a sibling module rather than folded into `assistant.rs`
//! because the planner is a distinct concept (model dispatch +
//! event-bus stream capture) from assistant CRUD / action dispatch
//! and the file rule prefers separation when the boundary is sharp.

use std::sync::Arc;

use ai_runner::Provider;
use codeless_adapters_host::ChatRunCfg;
use codeless_rpc::{RpcError, RpcResult};
use codeless_types::{AssistantMessage, AssistantMessageRole, AssistantThreadId, JobId, TaskId};
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use super::InProcessRpc;
use crate::time::now_ms;

/// CLI runner the F2 planner targets. REST runners (`anthropic`,
/// `openai`) are intentionally not reachable here — `agent_chat`
/// itself rejects them, and matching that surface keeps the typed
/// failure mode consistent across the two entry points.
const DEFAULT_PLANNER_RUNNER: Provider = Provider::Claude;

/// Outcome of one planner turn. `content` is what the caller
/// (`append_assistant_message`) persists verbatim as the assistant
/// row's text. The streamed token / completion envelopes that share
/// the same logical run are already on the bus tagged with the
/// synthetic `bus_job_id` (the thread id reused as a `JobId`); a
/// subscriber correlates them by that key.
#[derive(Debug)]
pub(super) struct PlannerTurn {
    pub content: String,
}

/// Whether the runtime is wired with an agent_chat registry + cwd.
/// `append_assistant_message` calls this before deciding between the
/// real planner and the NOOP fallback so tests / CLIs that boot
/// without `with_agent_chat` keep their existing behaviour without
/// having to inspect `RpcError::Internal` strings to detect the gap.
pub(super) fn planner_configured(rpc: &InProcessRpc) -> bool {
    rpc.agent_chat_registry.is_some() && rpc.agent_chat_cwd.is_some()
}

/// Run one planner turn.
///
/// `history` is the full prior transcript (in created_at-ascending
/// order, the same order `list_assistant_messages` returns) and is
/// folded into the prompt as labelled blocks. `user_content` is the
/// new turn the caller has *not yet persisted* — it is rendered as
/// the trailer so the model sees it as the message it is replying
/// to rather than as another historical entry.
pub(super) async fn run_planner_turn(
    rpc: &InProcessRpc,
    thread_id: AssistantThreadId,
    history: &[AssistantMessage],
    user_content: &str,
) -> RpcResult<PlannerTurn> {
    let registry = rpc.agent_chat_registry.as_ref().cloned().ok_or_else(|| {
        RpcError::Internal(
            "agent_chat registry is not configured on this runtime; \
             append_assistant_message requires `with_agent_chat`"
                .to_owned(),
        )
    })?;
    let cwd = rpc.agent_chat_cwd.clone().ok_or_else(|| {
        RpcError::Internal(
            "agent_chat cwd is not configured on this runtime; \
             append_assistant_message requires `with_agent_chat`"
                .to_owned(),
        )
    })?;

    let prompt = build_planner_prompt(history, user_content);
    let task_id = TaskId::new();
    // Bus envelopes key on `Option<JobId>`; the assistant surface has
    // no jobs row, so the synthetic id reuses the thread's ulid bytes.
    // The `events` table has no FK on `jobs`, and a UI subscriber that
    // wants the live stream filters on the same value.
    let bus_job_id = JobId(thread_id.0);
    let bus = Arc::clone(&rpc.bus);
    let collected: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&collected);

    let publish = move |event: codeless_types::Event| {
        let bus = Arc::clone(&bus);
        let sink = Arc::clone(&sink);
        async move {
            if let codeless_types::Event::AiToken { ref delta, .. } = event {
                sink.lock().push_str(delta);
            }
            bus.publish(Some(bus_job_id), None, Some(task_id), event, now_ms())
                .await
                .map(|_| ())
        }
    };

    // Local cancellation token: the assistant RPC blocks on the turn,
    // and stage 7 will land a cancel surface on top. For F2 the token
    // exists only so `run_chat` can wire it through to the runner.
    let cancel = CancellationToken::new();
    codeless_adapters_host::run_chat(
        registry,
        ChatRunCfg {
            provider: DEFAULT_PLANNER_RUNNER,
            prompt,
            cwd,
            tools: None,
        },
        task_id,
        publish,
        cancel,
    )
    .await
    .map_err(|e| RpcError::Internal(format!("planner run failed: {e}")))?;

    // The forwarder task that captured `sink` was joined inside
    // `run_chat`, so the inner Arc count is back to one and the take
    // is contention-free. A locked drain keeps the move trivial; the
    // alternative `Arc::try_unwrap` would panic if a stray clone
    // outlived us during a future refactor.
    let text = std::mem::take(&mut *collected.lock());
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(RpcError::Internal(
            "planner produced an empty reply".to_owned(),
        ));
    }
    Ok(PlannerTurn {
        content: trimmed.to_owned(),
    })
}

/// Render a single-shot prompt from the thread's prior turns and the
/// new user message. The shape is plain labelled blocks so a CLI
/// runner that does not natively understand role-tagged transcripts
/// still produces a coherent reply; the trailer separates the live
/// turn from history so the model does not mistake it for another
/// historical entry.
fn build_planner_prompt(history: &[AssistantMessage], user_content: &str) -> String {
    let mut out = String::from(PLANNER_SYSTEM_PREAMBLE);
    if !history.is_empty() {
        out.push_str("\n\n## Conversation so far\n");
        for msg in history {
            let label = match msg.role {
                AssistantMessageRole::User => "User",
                AssistantMessageRole::Assistant => "Assistant",
                AssistantMessageRole::Tool => "Tool",
                AssistantMessageRole::System => "System",
            };
            out.push_str("\n### ");
            out.push_str(label);
            out.push('\n');
            out.push_str(&msg.content);
            if !msg.content.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out.push_str("\n\n## Current user message\n");
    out.push_str(user_content);
    out.push_str("\n\nReply to the user. Plain prose only — action cards land in a later stage.\n");
    out
}

const PLANNER_SYSTEM_PREAMBLE: &str = "You are Codeless's in-app assistant. \
The user is talking to you from the workspace assistant pane. \
Answer their questions about jobs, repos, and the codebase concisely. \
Do not invent action cards; the dispatcher that turns them into \
real RPC calls is not yet wired.";

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ai_runner::{
        Event as AiEvent, EventKind, RunResult, Runner as AiRunner, RunnerError, RunnerInput,
        SessionId,
    };
    use async_trait::async_trait;
    use codeless_types::Event;
    use std::sync::Mutex as StdMutex;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken as Cancel;

    /// MockRunner-style fake for the planner: emits a scripted set of
    /// `Text` chunks then a `Done` envelope. Mirrors the shape the
    /// claude wrapper would produce, without spawning a child process.
    pub(crate) struct FakeChatRunner {
        provider: Provider,
        chunks: Vec<String>,
        seen_prompt: Arc<StdMutex<Option<String>>>,
    }

    impl FakeChatRunner {
        pub fn new(chunks: impl IntoIterator<Item = impl Into<String>>) -> Self {
            Self {
                provider: Provider::Claude,
                chunks: chunks.into_iter().map(Into::into).collect(),
                seen_prompt: Arc::new(StdMutex::new(None)),
            }
        }

        pub fn seen_prompt_handle(&self) -> Arc<StdMutex<Option<String>>> {
            Arc::clone(&self.seen_prompt)
        }
    }

    #[async_trait]
    impl AiRunner for FakeChatRunner {
        fn provider(&self) -> &Provider {
            &self.provider
        }

        async fn ready(&self) -> bool {
            true
        }

        async fn run(
            &self,
            input: RunnerInput,
            session_id: SessionId,
            on_event: mpsc::Sender<AiEvent>,
            _cancel: Cancel,
        ) -> Result<RunResult, RunnerError> {
            if let RunnerInput::Cli(cfg) = &input {
                *self.seen_prompt.lock().unwrap() = Some(cfg.prompt.clone());
            }
            for chunk in &self.chunks {
                let _ = on_event
                    .send(AiEvent {
                        session_id: session_id.clone(),
                        provider: "claude".into(),
                        kind: EventKind::Text {
                            content: chunk.clone(),
                        },
                    })
                    .await;
            }
            let _ = on_event
                .send(AiEvent {
                    session_id: session_id.clone(),
                    provider: "claude".into(),
                    kind: EventKind::Done {
                        duration_ms: 1,
                        cost_usd: 0.0,
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                })
                .await;
            Ok(RunResult::default())
        }
    }

    fn registry_with(runner: Arc<dyn AiRunner>) -> Arc<ai_runner::Registry> {
        let r = ai_runner::Registry::new();
        r.register(runner);
        Arc::new(r)
    }

    async fn rpc_with_planner(
        chunks: Vec<&'static str>,
    ) -> (InProcessRpc, Arc<StdMutex<Option<String>>>) {
        let runner = Arc::new(FakeChatRunner::new(chunks));
        let seen = runner.seen_prompt_handle();
        let registry = registry_with(runner);
        let rpc = InProcessRpc::new()
            .await
            .unwrap()
            .with_agent_chat(registry, std::env::temp_dir());
        (rpc, seen)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn planner_concatenates_streamed_chunks() {
        let (rpc, seen) = rpc_with_planner(vec!["hello, ", "world", "!"]).await;
        let thread_id = AssistantThreadId::new();

        let turn = run_planner_turn(&rpc, thread_id, &[], "hi there")
            .await
            .unwrap();
        assert_eq!(turn.content, "hello, world!");

        let prompt = seen.lock().unwrap().clone().expect("runner saw a prompt");
        assert!(prompt.contains("Current user message"), "prompt: {prompt}");
        assert!(prompt.contains("hi there"), "prompt: {prompt}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn planner_publishes_ai_token_events_to_the_bus() {
        let (rpc, _seen) = rpc_with_planner(vec!["alpha", "-", "beta"]).await;
        let thread_id = AssistantThreadId::new();
        let bus_job_id = JobId(thread_id.0);

        // Subscribe before invoking the planner so the live tail sees
        // every token; the bus's catch-up cursor would otherwise depend
        // on the order of operations and the test would race the run.
        let mut stream = rpc
            .bus()
            .subscribe_since(crate::event_bus::SubscribeFilter::Job(bus_job_id), None)
            .await
            .unwrap();

        let turn = run_planner_turn(&rpc, thread_id, &[], "ping")
            .await
            .unwrap();
        assert_eq!(turn.content, "alpha-beta");

        use futures_util::StreamExt;
        let mut deltas: Vec<String> = Vec::new();
        let mut task_ids: Vec<TaskId> = Vec::new();
        let mut saw_complete = false;
        // run_chat has joined its forwarder by the time it returns, so
        // every event is already queued on the broadcast tail. A short
        // bounded drain keeps the test from hanging if the stream is
        // somehow empty (which would itself be the failure to assert).
        for _ in 0..16 {
            let next = tokio::time::timeout(
                std::time::Duration::from_millis(200),
                stream.next(),
            )
            .await;
            match next {
                Ok(Some(Ok(env))) => match env.event {
                    Event::AiToken { delta, task_id } => {
                        task_ids.push(task_id);
                        deltas.push(delta);
                    }
                    Event::AiMessageComplete { .. } => {
                        saw_complete = true;
                        break;
                    }
                    _ => {}
                },
                _ => break,
            }
        }
        assert_eq!(deltas, vec!["alpha", "-", "beta"]);
        assert!(saw_complete, "AiMessageComplete envelope must arrive");
        // Every token in one turn must share the same task id so a
        // subscriber can stitch them into one streamed message.
        assert!(task_ids.windows(2).all(|w| w[0] == w[1]));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn planner_includes_history_in_prompt() {
        let (rpc, seen) = rpc_with_planner(vec!["ack"]).await;
        let thread_id = AssistantThreadId::new();
        let history = vec![
            AssistantMessage {
                id: codeless_types::AssistantMessageId::new(),
                thread_id,
                role: AssistantMessageRole::User,
                content: "prior question".into(),
                meta_json: None,
                created_at: now_ms(),
            },
            AssistantMessage {
                id: codeless_types::AssistantMessageId::new(),
                thread_id,
                role: AssistantMessageRole::Assistant,
                content: "prior reply".into(),
                meta_json: None,
                created_at: now_ms(),
            },
        ];
        let _ = run_planner_turn(&rpc, thread_id, &history, "follow-up").await.unwrap();
        let prompt = seen.lock().unwrap().clone().unwrap();
        assert!(prompt.contains("prior question"));
        assert!(prompt.contains("prior reply"));
        assert!(prompt.contains("follow-up"));
        // History block precedes the live turn so the model treats the
        // trailer as "the message I am replying to".
        let history_at = prompt.find("Conversation so far").unwrap();
        let live_at = prompt.find("Current user message").unwrap();
        assert!(history_at < live_at);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn planner_rejects_empty_reply() {
        let (rpc, _seen) = rpc_with_planner(vec!["", "   ", "\n"]).await;
        let err = run_planner_turn(&rpc, AssistantThreadId::new(), &[], "hi")
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::Internal(_)), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn planner_unconfigured_returns_internal() {
        let rpc = InProcessRpc::new().await.unwrap();
        assert!(!planner_configured(&rpc));
        let err = run_planner_turn(&rpc, AssistantThreadId::new(), &[], "hi")
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::Internal(_)), "got {err:?}");
    }
}
