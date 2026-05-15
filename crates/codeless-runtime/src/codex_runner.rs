//! Adapter that wraps `ai_runner::runners::codex::CodexRunner` so it
//! slots into `drive_job` as a `codeless_runtime::Runner`.
//!
//! Symmetric to `ClaudeRunnerAdapter` but without the headless system
//! prompt, permission mode, or session-resume plumbing — the upstream
//! `codex` CLI does not expose those knobs. The adapter forwards events
//! through `ai_runner_bridge` and maps `RunResult::error` to
//! `RunnerOutcome::Failed`. `OPENAI_API_KEY` is read by the upstream
//! binary from its own environment; this crate does not set it.

use std::sync::Arc;

use ai_runner::runners::codex::CodexRunner;
use ai_runner::{CliCfg, Runner as AiRunner, RunnerInput};
use async_trait::async_trait;
use codeless_adapters_host::ai_runner_bridge::forward_events;
use codeless_types::TaskId;
use tokio::sync::mpsc;

use crate::event_bus::EventBus;
use crate::runner::{Runner, RunnerContext, RunnerOutcome};
use crate::time::now_ms;

pub struct CodexRunnerAdapter {
    pub prompt: String,
    pub task_id: TaskId,
    pub model: Option<String>,
    pub event_buffer: usize,
}

impl CodexRunnerAdapter {
    pub fn new(prompt: impl Into<String>, task_id: TaskId) -> Self {
        Self {
            prompt: prompt.into(),
            task_id,
            model: None,
            event_buffer: 64,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        let s = model.into();
        self.model = if s.is_empty() { None } else { Some(s) };
        self
    }
}

#[async_trait]
impl Runner for CodexRunnerAdapter {
    async fn run(&self, ctx: RunnerContext) -> RunnerOutcome {
        let (tx, rx) = mpsc::channel(self.event_buffer);
        let cancel = ctx.cancel.clone();
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
            model: self.model.clone(),
            ..Default::default()
        });

        let upstream = CodexRunner;
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
                reason: format!("codex runner input mismatch: {e}"),
            },
            Ok(rr) => match rr.error {
                Some(msg) => RunnerOutcome::Failed { reason: msg },
                None => RunnerOutcome::Completed,
            },
        }
    }
}
