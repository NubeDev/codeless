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
use ai_runner::{CliCfg, PermissionMode, Runner as AiRunner, RunnerInput};
use async_trait::async_trait;
use codeless_adapters_host::ai_runner_bridge::forward_events;
use codeless_types::TaskId;
use tokio::sync::mpsc;

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
    /// System prompt prepended to the user's prompt. The headless
    /// server-side default (see `DEFAULT_SYSTEM_PROMPT`) tells claude
    /// it's running inside a git worktree with no human to approve
    /// tool calls — without it, claude tends to write a runtime
    /// (`main.go`, a shell script) instead of just creating the file
    /// the user asked for. Override from the secrets file under
    /// `claude_system_prompt`; explicitly `Some("")` disables it.
    pub system_prompt: Option<String>,
}

/// Headless default. Codeless's runner has no human at the TTY to
/// approve mid-run permission prompts; claude tends to compensate by
/// emitting a script that *would* do the work if the user ran it
/// (writes `main.go` that writes `people.csv`, etc.). This prompt
/// frames the run for what it actually is — a one-shot file-editor
/// — so the assistant goes straight to the Write tool.
pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You are running headless inside an isolated git worktree with no \
interactive user. Use your file-editing tools (Read, Write, Edit, \
Glob, Grep, Bash) to satisfy the request directly.\n\n\
LITERAL FILE REQUESTS. When the user names a file format or \
extension (CSV, JSON, YAML, TOML, Markdown, SQL, .env, etc.), \
create that file directly with the requested content. Do NOT write \
a program in Go, Python, JavaScript, or any other language that \
generates the file when run. Example: \"make a people.csv with \
name and age columns\" means write `people.csv` containing CSV \
text, not `main.go` that writes `people.csv`.\n\n\
LANGUAGE INFERENCE. Pick the implementation language from the \
repo's existing files (Cargo.toml → Rust, package.json → \
TypeScript/Node, go.mod → Go, pyproject.toml / requirements.txt → \
Python). If the repo gives no signal and the user did not name a \
language, ASK before writing code. Do not default to Go.\n\n\
COMMIT YOUR WORK. Run `git add` and `git commit` so changes survive \
worktree cleanup; uncommitted edits are not visible to the user \
after the job ends.\n\n\
AMBIGUITY. Prefer the most literal reading. If the request is \
genuinely under-specified, use the AskUserQuestion tool — do not \
silently invent scope.";

impl ClaudeRunnerAdapter {
    pub fn new(prompt: impl Into<String>, task_id: TaskId) -> Self {
        Self {
            prompt: prompt.into(),
            task_id,
            event_buffer: 64,
            system_prompt: Some(DEFAULT_SYSTEM_PROMPT.to_owned()),
        }
    }

    /// Replace the headless default system prompt. Passing an empty
    /// string disables the prompt entirely; pass a real string to
    /// override the built-in framing.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        let s = prompt.into();
        self.system_prompt = if s.is_empty() { None } else { Some(s) };
        self
    }
}

#[async_trait]
impl Runner for ClaudeRunnerAdapter {
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
            system_prompt: self.system_prompt.clone(),
            work_dir: ctx
                .worktree_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            // Headless: there's no TTY user to approve mid-run tool
            // calls. Bypass every permission gate so claude actually
            // runs its tools instead of asking and aborting. The
            // worktree is the blast radius (isolated branch, isolated
            // checkout); cleanup is the user's call via the upcoming
            // gc_worktrees RPC, not ours.
            permission_mode: Some(PermissionMode::Bypass),
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
