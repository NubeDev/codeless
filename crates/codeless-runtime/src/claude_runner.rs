//! Adapter that wraps `ai_runner::runners::ClaudeRunner` so it can be
//! handed to `drive_job` as a `codeless_runtime::Runner`.
//!
//! Two concerns the adapter owns:
//!
//! 1. **Event plumbing.** `ai-runner` streams its events on an
//!    `mpsc::Sender<ai_runner::Event>`. The adapter spawns a forwarder
//!    that drains that channel and republishes each event through
//!    `EventBus` via the `ai_runner_bridge` translation layer in
//!    `codeless-adapters-host`. The forwarder runs concurrently with
//!    the upstream `run`; both are joined before we return.
//! 2. **Outcome mapping.** `ai_runner::RunResult::error` carries
//!    upstream-side failures (network, model, parse). The adapter
//!    maps a non-`None` error to `RunnerOutcome::Failed`; everything
//!    else is `Completed`. `WrongInputKind` from the upstream Runner
//!    contract is internal misuse, so it surfaces as `Failed` too.

use std::sync::Arc;

use ai_runner::runners::claude::ClaudeRunner;
use ai_runner::{CliCfg, Runner as AiRunner, RunnerInput};
use async_trait::async_trait;
use codeless_adapters_host::ai_runner_bridge::forward_events;
use codeless_types::TaskId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::event_bus::EventBus;
use crate::runner::{Runner, RunnerContext, RunnerOutcome};
use crate::time::now_ms;

/// Per-run configuration the codeless driver hands to a Claude run.
/// The adapter does not retain any state between runs — each
/// `drive_job` builds a fresh `ClaudeRunnerAdapter` for the job it is
/// about to invoke. The shape stays minimal on purpose: more
/// upstream knobs land here only when a job-row column gives them a
/// home. Tools, MCP, model selection are explicit-future work.
pub struct ClaudeRunnerAdapter {
    pub prompt: String,
    pub task_id: TaskId,
    /// Channel capacity for the upstream `mpsc::Sender<ai_runner::Event>`.
    /// Backpressure semantics on the upstream sender are documented in
    /// `ai_runner::OnEvent`: REST runners `.await` and slow consumers
    /// throttle the producer; CLI runners (this one) use `try_send` and
    /// drop events on overflow with a `tracing::warn!`. 64 is enough
    /// headroom for the bridge to keep up under bus contention without
    /// blowing memory on a misbehaving run.
    pub event_buffer: usize,
}

impl ClaudeRunnerAdapter {
    pub fn new(prompt: impl Into<String>, task_id: TaskId) -> Self {
        Self {
            prompt: prompt.into(),
            task_id,
            event_buffer: 64,
        }
    }
}

#[async_trait]
impl Runner for ClaudeRunnerAdapter {
    async fn run(&self, ctx: RunnerContext) -> RunnerOutcome {
        let (tx, rx) = mpsc::channel(self.event_buffer);
        let cancel = CancellationToken::new();

        let bus: Arc<EventBus> = Arc::clone(&ctx.bus);
        let job_id = ctx.job_id;
        let task_id = self.task_id;
        let forwarder = tokio::spawn(async move {
            forward_events(rx, task_id, move |event| {
                let bus = Arc::clone(&bus);
                async move {
                    bus.publish(Some(job_id), None, Some(task_id), event, now_ms())
                        .await
                        .map(|_| ())
                }
            })
            .await
        });

        let input = RunnerInput::Cli(CliCfg {
            prompt: self.prompt.clone(),
            work_dir: ctx
                .worktree_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            ..Default::default()
        });

        let upstream = ClaudeRunner;
        let run_result = upstream
            .run(input, job_id.to_string().into(), tx, cancel)
            .await;

        let forwarder_result = forwarder.await;
        if let Err(e) = forwarder_result {
            tracing::warn!(error = %e, "bridge forwarder task panicked");
            return RunnerOutcome::Failed {
                reason: format!("event forwarder panicked: {e}"),
            };
        }
        if let Ok(Err(e)) = forwarder_result.as_ref().map(|inner| inner.as_ref()) {
            tracing::warn!(error = %e, "bridge publish failed");
            return RunnerOutcome::Failed {
                reason: format!("event publish: {e}"),
            };
        }

        match run_result {
            Err(e) => RunnerOutcome::Failed {
                reason: format!("claude runner input mismatch: {e}"),
            },
            Ok(rr) => match rr.error {
                Some(msg) => RunnerOutcome::Failed { reason: msg },
                None => RunnerOutcome::Completed,
            },
        }
    }
}
