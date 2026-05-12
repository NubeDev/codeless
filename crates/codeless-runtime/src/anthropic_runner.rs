//! Adapter that wraps `ai_runner::runners::anthropic::AnthropicRunner`
//! so it slots into `drive_job` as a `codeless_runtime::Runner`.
//!
//! Symmetric to `ClaudeRunnerAdapter`: an mpsc channel paired with a
//! `forward_events` task carries `ai_runner::Event`s through the
//! adapters-host bridge onto `EventBus`, and `RunResult::error` is
//! mapped to `RunnerOutcome::Failed`. The only shape difference is
//! the transport: REST instead of CLI, so the upstream runner
//! receives a `RestCfg` rather than a `CliCfg`.

use std::sync::Arc;

use ai_runner::runners::anthropic::AnthropicRunner;
use ai_runner::{RestCfg, Runner as AiRunner, RunnerInput};
use async_trait::async_trait;
use codeless_adapters_host::ai_runner_bridge::forward_events;
use codeless_types::TaskId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::event_bus::EventBus;
use crate::runner::{Runner, RunnerContext, RunnerOutcome};
use crate::time::now_ms;

pub struct AnthropicRunnerAdapter {
    pub prompt: String,
    pub task_id: TaskId,
    pub model: Option<String>,
    pub api_key: Option<String>,
    /// Optional REST endpoint override. Wired through to
    /// `RestCfg::base_url`; the test harness uses this to point the
    /// upstream Anthropic SDK at a `wiremock` mock server instead of
    /// the real api.anthropic.com.
    pub base_url: Option<String>,
    pub event_buffer: usize,
}

impl AnthropicRunnerAdapter {
    pub fn new(prompt: impl Into<String>, task_id: TaskId) -> Self {
        Self {
            prompt: prompt.into(),
            task_id,
            model: None,
            api_key: None,
            base_url: None,
            event_buffer: 64,
        }
    }
}

#[async_trait]
impl Runner for AnthropicRunnerAdapter {
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

        let input = RunnerInput::Rest(RestCfg {
            prompt: self.prompt.clone(),
            model: self.model.clone(),
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            ..Default::default()
        });

        let upstream = AnthropicRunner;
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
                reason: format!("anthropic runner input mismatch: {e}"),
            },
            Ok(rr) => match rr.error {
                Some(msg) => RunnerOutcome::Failed { reason: msg },
                None => RunnerOutcome::Completed,
            },
        }
    }
}
