// Built-in runner factory shared by every host binary. Hosts that
// embed an `InProcessRpc` (`codeless serve`, `codeless-tauri-desktop`)
// reuse the same dispatch table so a job submitted from the desktop
// shell, the CLI, or the browser drives through identical adapters.
//
// `mock` is only built when no real runner is enabled, mirroring the
// CLI's `--enable-*` semantics. A `runner: "mock"` job submitted to a
// real-runner-enabled host returns `None` and the driver fails the
// job loudly rather than silently running a no-op against the repo.
//
// Hosts that need a different selection (tests, fixtures) build their
// own `RunnerFactory`; this type is the production default, not the
// only option.

use std::sync::Arc;
use std::time::Duration;

use codeless_types::{CostCents, Event, Job, TaskId, TaskStatus};

use crate::adapter_registry::EffectiveAdapterRegistry;
use crate::anthropic_runner::AnthropicRunnerAdapter;
use crate::claude_runner::{parse_permission_mode, ClaudeRunnerAdapter};
use crate::codex_runner::CodexRunnerAdapter;
use crate::copilot_runner::CopilotRunnerAdapter;
use crate::job_driver_loop::RunnerFactory;
use crate::mock_runner::{MockRunner, MockStep};
use crate::runner::{Runner, RunnerOutcome};
use crate::store::SqliteStore;
use crate::template::JobTemplate;
use crate::template_runner::TemplateRunner;

/// Boot-time view of the four runner enable bits. Hosts read this from
/// `runner_config` (via `EffectiveAdapterRegistry`) and hand it to
/// `DefaultRunnerFactory`; tests construct it directly. Splitting it out
/// keeps the factory's public surface free of free-floating booleans the
/// stage-1 adapter-registry work was designed to retire and gives the
/// future Settings → Adapters page a single struct to round-trip
/// through.
#[derive(Debug, Clone, Default)]
pub struct RunnerConfig {
    pub claude: bool,
    pub anthropic: bool,
    pub codex: bool,
    pub copilot: bool,
}

impl RunnerConfig {
    /// Project the relevant slice of `EffectiveAdapterRegistry` (which
    /// also carries the chat-adapter bits) onto the runner enable set.
    /// This is the boot-time path: `codeless serve` reads the table
    /// once, projects it here, and passes the result into the factory.
    pub fn from_effective(effective: &EffectiveAdapterRegistry) -> Self {
        Self {
            claude: effective.claude_enabled,
            anthropic: effective.anthropic_enabled,
            codex: effective.codex_enabled,
            copilot: effective.copilot_enabled,
        }
    }

    /// True when any *real* runner is enabled. Used by the factory to
    /// decide whether to gate `mock`: when at least one real runner is
    /// on, a `runner: "mock"` submission returns `None` and the driver
    /// fails the job loudly rather than silently running a no-op.
    pub fn any_real(&self) -> bool {
        self.claude || self.anthropic || self.codex || self.copilot
    }
}

/// Built-in runner factory. `mock` is gated off when any real runner
/// is enabled. `claude` and `anthropic` need configuration the host
/// must supply (the `claude` binary on PATH, the Anthropic API key);
/// an opt-in runner with missing config still builds — the runner
/// adapter surfaces the auth failure at run time as
/// `RunnerOutcome::Failed`.
pub struct DefaultRunnerFactory {
    /// Which runners the factory will build. Sourced from the
    /// `runner_config` table at boot; tests construct this directly.
    pub config: RunnerConfig,
    pub anthropic_api_key: Option<String>,
    /// Optional override for the claude headless system prompt. When
    /// set, replaces the built-in default in
    /// `ClaudeRunnerAdapter::DEFAULT_SYSTEM_PROMPT`. An empty string
    /// disables the prompt entirely.
    pub claude_system_prompt: Option<String>,
    /// Store handle for resume-aware stage execution. The
    /// `TemplateRunner` looks up each stage's captured `session_id`
    /// before invoking the inner Claude adapter so an interrupted
    /// stage resumes the same conversation via `--continue`.
    pub store: Arc<SqliteStore>,
    /// Resolved path to the `codeless-mcp` binary. When set, every
    /// Claude runner gets a per-job MCP config (stdio transport) so
    /// codeless-registered tools are available alongside built-in tools.
    pub mcp_binary_path: Option<String>,
}

impl DefaultRunnerFactory {
    pub fn real_runner_enabled(&self) -> bool {
        self.config.any_real()
    }
}

impl RunnerFactory for DefaultRunnerFactory {
    fn build(
        &self,
        job: &Job,
        pending_operator_comment: Option<String>,
    ) -> Option<Arc<dyn Runner>> {
        // `prompt` is documented as Optional on `SubmitJobArgs`; a
        // missing prompt is most likely a YAML-template job whose
        // stages list carries the real work. Branch on `template_yaml`
        // first: a parseable template means the multi-stage
        // `TemplateRunner` (claude-backed) is the right choice
        // regardless of the `runner` field — the user's template said
        // "drive these stages," and the runner string is the
        // transport, not the choice.
        if let Some(template_src) = job.template_yaml.as_ref() {
            match JobTemplate::parse_yaml(template_src) {
                Ok(template) if self.config.claude => {
                    let mut runner = TemplateRunner::new(template)
                        .with_store(self.store.clone())
                        .with_pending_operator_comment(pending_operator_comment.clone());
                    if let Some(sp) = compose_system_prompt(
                        self.claude_system_prompt.as_deref(),
                        job.system_prompt.as_deref(),
                    ) {
                        runner = runner.with_system_prompt(sp);
                    }
                    if let Some(ref mcp) = self.mcp_binary_path {
                        runner = runner.with_mcp_binary(mcp.clone());
                    }
                    return Some(Arc::new(runner));
                }
                Ok(template) => {
                    tracing::info!(
                        stages = template.stages.len(),
                        "running template via mock runner (claude disabled)"
                    );
                    let runner = TemplateRunner::new(template)
                        .with_mock_runner()
                        .with_pending_operator_comment(pending_operator_comment.clone());
                    return Some(Arc::new(runner));
                }
                Err(err) => {
                    tracing::warn!(
                        ?err,
                        "failed to parse template_yaml; falling back to prompt path"
                    );
                }
            }
        }
        // Single-prompt runners (claude/anthropic/codex/copilot
        // direct, mock) have no per-stage prompt builder, so the
        // operator-comment slot is dropped here. The resume path that
        // produces the comment only makes sense for multi-stage
        // template jobs.
        let _ = pending_operator_comment;
        let prompt = job.prompt.clone().unwrap_or_default();
        let real_runner_enabled = self.real_runner_enabled();
        match job.runner.as_str() {
            "mock" if !real_runner_enabled => {
                Some(Arc::new(MockRunner::new(demo_mock_script(&prompt))))
            }
            "claude" if self.config.claude => {
                let mut adapter = ClaudeRunnerAdapter::new(prompt, TaskId::new());
                if let Some(sp) = compose_system_prompt(
                    self.claude_system_prompt.as_deref(),
                    job.system_prompt.as_deref(),
                ) {
                    adapter = adapter.with_system_prompt(sp);
                }
                if let Some(m) = job.model.as_deref() {
                    adapter = adapter.with_model(m);
                }
                if let Some(pm) = job
                    .permission_mode
                    .as_deref()
                    .and_then(parse_permission_mode)
                {
                    adapter = adapter.with_permission_mode(pm);
                }
                if let Some(e) = job.effort.as_deref() {
                    adapter = adapter.with_effort(e);
                }
                if let Some(ref mcp) = self.mcp_binary_path {
                    adapter = adapter.with_mcp_binary(mcp.clone());
                }
                Some(Arc::new(adapter))
            }
            "anthropic" if self.config.anthropic => {
                let mut adapter = AnthropicRunnerAdapter::new(prompt, TaskId::new());
                adapter.api_key = self.anthropic_api_key.clone();
                Some(Arc::new(adapter))
            }
            "codex" if self.config.codex => {
                let mut adapter = CodexRunnerAdapter::new(prompt, TaskId::new());
                if let Some(m) = job.model.as_deref() {
                    adapter = adapter.with_model(m);
                }
                Some(Arc::new(adapter))
            }
            "copilot" if self.config.copilot => {
                let mut adapter = CopilotRunnerAdapter::new(prompt, TaskId::new());
                if let Some(m) = job.model.as_deref() {
                    adapter = adapter.with_model(m);
                }
                Some(Arc::new(adapter))
            }
            _ => None,
        }
    }
}

/// Merge the host's baseline system prompt with the per-job
/// persona-derived prompt. Baseline captures rules every job should
/// obey; job-level prompt is the persona's `instructions`. Both can be
/// absent; when both are present we keep the baseline first so persona
/// text refines it without re-stating universal rules.
pub fn compose_system_prompt(server: Option<&str>, job: Option<&str>) -> Option<String> {
    let server = server.map(str::trim).filter(|s| !s.is_empty());
    let job = job.map(str::trim).filter(|s| !s.is_empty());
    match (server, job) {
        (Some(s), Some(j)) => Some(format!("{s}\n\n{j}")),
        (Some(s), None) => Some(s.to_owned()),
        (None, Some(j)) => Some(j.to_owned()),
        (None, None) => None,
    }
}

/// Build a `MockRunner` script that emits enough events to be visibly
/// alive in the UI's JobTimeline. Real AI runners drive these same
/// event variants through `ctx.bus`. The `FAIL` prompt is a sentinel
/// for tests that need the failure path without provisioning a real
/// runner.
pub fn demo_mock_script(prompt: &str) -> Vec<MockStep> {
    if prompt == "FAIL" {
        return vec![MockStep::Finish(RunnerOutcome::Failed {
            reason: "mock runner: FAIL sentinel".into(),
        })];
    }

    let task_id = TaskId::new();
    let echo = if prompt.is_empty() {
        "demo: mock runner ran end-to-end".to_owned()
    } else {
        format!("mock-echo: {prompt}")
    };
    let mut steps = Vec::new();
    steps.push(MockStep::Emit(Event::TaskStarted { task_id }));
    for chunk in chunk_for_stream(&echo) {
        steps.push(MockStep::Emit(Event::AiToken {
            task_id,
            delta: chunk,
        }));
        steps.push(MockStep::Sleep(Duration::from_millis(120)));
    }
    steps.push(MockStep::Emit(Event::AiMessageComplete {
        task_id,
        input_tokens: 0,
        output_tokens: 0,
        cost_cents: CostCents(0),
    }));
    steps.push(MockStep::Emit(Event::TaskCompleted {
        task_id,
        status: TaskStatus::Completed,
    }));
    steps.push(MockStep::Finish(RunnerOutcome::Completed));
    steps
}

fn chunk_for_stream(s: &str) -> Vec<String> {
    s.split_inclusive(' ')
        .filter(|w| !w.trim().is_empty())
        .map(|w| w.to_owned())
        .collect()
}
