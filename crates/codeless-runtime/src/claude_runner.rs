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
use codeless_types::{Event, JobStatus, TaskId};
use tokio::sync::{mpsc, Mutex};

use crate::event_bus::EventBus;
use crate::handover::{extract_handover, fallback_handover_from_text, write_handover};
use crate::runner::{Runner, RunnerContext, RunnerOutcome};
use crate::store::SqliteStore;
use crate::time::now_ms;
use crate::trio_emitter::{emit_trio_completed, emit_trio_started};
use codeless_types::{TodoKind, TodoStatus};

/// Per-run configuration the codeless driver hands to a Claude run.
/// The adapter does not retain any state between runs — each
/// `drive_job` builds a fresh `ClaudeRunnerAdapter` for the job it is
/// about to invoke. The shape stays minimal on purpose: more
/// upstream knobs land here only when a job-row column gives them a
/// home.
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
    /// Optional model override (Anthropic model id, e.g.
    /// `claude-opus-4-7`). `None` lets `claude-wrapper` pick.
    pub model: Option<String>,
    /// Per-run permission mode. `None` keeps the headless default
    /// (`Bypass`) — the worktree is the blast radius and there is no
    /// TTY user to answer mid-run prompts. The factory passes
    /// `Some(...)` when the user explicitly picked a non-bypass mode.
    pub permission_mode: Option<ai_runner::PermissionMode>,
    /// Provider-agnostic thinking budget. Mapped onto
    /// `claude-wrapper`'s prompt-trigger trick ("think" / "think hard"
    /// / "ultrathink") by `ai_runner`. Accepted labels: `low`,
    /// `medium`, `high`. `None` means no prefix.
    pub thinking_budget: Option<String>,
    /// Claude session id to resume — when `Some`, the upstream
    /// `claude-wrapper` runs `claude --continue <id>` instead of
    /// starting a fresh conversation. Used by A0 (intra-stage
    /// session continuation per SCOPE.md hard rule #1): a
    /// cost-cap / wall-clock / user-stop interrupting a stage
    /// captures the session id on the `Stage` row; the user
    /// resumes via `resume_job` and the next task on that stage
    /// picks up where the agent left off — same in-context files,
    /// same half-formed plan — rather than re-deriving from
    /// scratch. `None` (default) means a fresh session.
    pub resume_id: Option<String>,
    /// Path to the `codeless-mcp` binary. When set, the adapter
    /// generates a per-job MCP config file pointing at this binary
    /// (stdio transport) so the runner can invoke codeless-registered
    /// tools alongside its built-in set.
    pub mcp_binary_path: Option<String>,
    /// SQLite store used to look up trio TodoIds at handover-write
    /// time. `None` keeps legacy callers (the early test harness, the
    /// in-process integration tests) working without a store on hand;
    /// production wiring through `TemplateRunner` always populates it
    /// so the runtime-injected `Docs` trio row flips around the
    /// handover write rather than staying `Pending` forever.
    pub store: Option<Arc<SqliteStore>>,
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
silently invent scope.\n\n\
HANDOVER. End your final reply with a fenced code block whose \
info string is exactly `handover`, containing four `##` markdown \
headings in this fixed order: `Done`, `Next`, `What you need to \
know`, `Open questions`. Each section is a bullet list (use `- ` \
or `* `). Use `- (none)` for an intentionally empty section. The \
codeless runtime extracts this block verbatim as the session \
handover (DOCS/JOB-MODEL.md); a missing or malformed block \
forces the runtime to fall back to a truncated tail of your \
reply, which is worse for the next session.\n\n\
Example:\n\
```handover\n\
## Done\n\n- created people.csv with the requested columns\n\n\
## Next\n\n- (none)\n\n\
## What you need to know\n\n- file is utf-8, no BOM\n\n\
## Open questions\n\n- (none)\n\
```";

/// Parse the wire label for a permission mode (the same string the UI
/// puts on the job row: `default | accept_edits | plan | bypass`).
/// Returns `None` on any unrecognised value so callers can fall back
/// to the headless default rather than refusing the run.
pub fn parse_permission_mode(label: &str) -> Option<ai_runner::PermissionMode> {
    use ai_runner::PermissionMode as P;
    match label.trim().to_ascii_lowercase().as_str() {
        "default" => Some(P::Default),
        "accept_edits" | "accept-edits" => Some(P::AcceptEdits),
        "plan" => Some(P::Plan),
        "bypass" | "bypass_permissions" => Some(P::Bypass),
        _ => None,
    }
}

impl ClaudeRunnerAdapter {
    pub fn new(prompt: impl Into<String>, task_id: TaskId) -> Self {
        Self {
            prompt: prompt.into(),
            task_id,
            event_buffer: 64,
            system_prompt: Some(DEFAULT_SYSTEM_PROMPT.to_owned()),
            model: None,
            permission_mode: None,
            thinking_budget: None,
            resume_id: None,
            mcp_binary_path: None,
            store: None,
        }
    }

    /// Attach a SQLite store so the adapter can flip the `Docs` trio
    /// row around the per-stage handover write. The store is `Arc`-ed
    /// because the driver already shares one with the rest of the
    /// runtime; cloning the handle is cheap.
    pub fn with_store(mut self, store: Arc<SqliteStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Resume the upstream claude session with the given id. The
    /// wrapper renders `claude --continue <id>`; the agent picks up
    /// the same conversation rather than re-deriving the codebase.
    /// Empty string clears back to "fresh session".
    pub fn with_resume_id(mut self, id: impl Into<String>) -> Self {
        let s = id.into();
        self.resume_id = if s.is_empty() { None } else { Some(s) };
        self
    }

    /// Replace the headless default system prompt. Passing an empty
    /// string disables the prompt entirely; pass a real string to
    /// override the built-in framing.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        let s = prompt.into();
        self.system_prompt = if s.is_empty() { None } else { Some(s) };
        self
    }

    /// Override the model the wrapper passes to claude. Pass an empty
    /// string to clear back to the wrapper's default.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        let s = model.into();
        self.model = if s.is_empty() { None } else { Some(s) };
        self
    }

    /// Override the headless `Bypass` default with a specific
    /// permission mode. The driver passes whatever the user picked
    /// in the Submit Job dialog through here verbatim.
    pub fn with_permission_mode(mut self, mode: ai_runner::PermissionMode) -> Self {
        self.permission_mode = Some(mode);
        self
    }

    /// Provider-agnostic thinking budget label (`low | medium | high`).
    /// Empty string clears.
    pub fn with_effort(mut self, effort: impl Into<String>) -> Self {
        let s = effort.into();
        self.thinking_budget = if s.is_empty() { None } else { Some(s) };
        self
    }

    /// Path to the `codeless-mcp` binary. When set, the adapter
    /// writes a per-job MCP config (stdio transport) and passes it
    /// to the runner so codeless-registered tools are available.
    pub fn with_mcp_binary(mut self, path: impl Into<String>) -> Self {
        let s = path.into();
        self.mcp_binary_path = if s.is_empty() { None } else { Some(s) };
        self
    }

    /// Generate a temp MCP config JSON pointing at the codeless-mcp
    /// binary (stdio transport). Returns `None` when no binary path
    /// is configured.
    fn build_mcp_config(&self, worktree: Option<&std::path::Path>) -> Option<String> {
        let bin = self.mcp_binary_path.as_deref()?;

        let worktree_str = worktree
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());

        let json = serde_json::json!({
            "mcpServers": {
                "codeless": {
                    "command": bin,
                    "args": [],
                    "env": {
                        "CODELESS_WORKTREE_ROOT": worktree_str
                    }
                }
            }
        });

        let path = std::env::temp_dir().join(format!(
            "codeless-mcp-{}-{}.json",
            std::process::id(),
            self.task_id
        ));
        let bytes = serde_json::to_vec_pretty(&json).ok()?;
        std::fs::write(&path, bytes).ok()?;
        Some(path.to_string_lossy().into_owned())
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
        // Tee assistant text into a side buffer so we can extract a
        // structured handover block after the run completes. Wrapped
        // in `Arc<Mutex<…>>` so the forwarder task (a separate tokio
        // task) and this task can share it; contention is non-existent
        // in practice because the forwarder is the only writer until
        // it returns.
        let assistant_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let buf_for_forwarder = Arc::clone(&assistant_buf);
        let forwarder = tokio::spawn(async move {
            forward_events(rx, task_id, move |event| {
                let bus = Arc::clone(&bus);
                let buf = Arc::clone(&buf_for_forwarder);
                async move {
                    if let Event::AiToken { delta, .. } = &event {
                        let mut guard = buf.lock().await;
                        guard.push_str(delta);
                    }
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
            model: self.model.clone(),
            // Headless default is `Bypass` — no TTY user to approve
            // mid-run tool calls, and the worktree is the blast radius.
            // The user can override per-job via the Submit dialog by
            // picking `default | accept_edits | plan`; that lands here
            // as `Some(...)` and replaces the default.
            permission_mode: Some(self.permission_mode.unwrap_or(PermissionMode::Bypass)),
            thinking_budget: self.thinking_budget.clone(),
            resume_id: self.resume_id.clone(),
            mcp_config_path: self.build_mcp_config(ctx.worktree_path.as_deref()),
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

        // Pin the upstream-supplied session id onto the stage row
        // before mapping the outcome. The bus envelope carries
        // `stage_id` so `StageRecorder` resolves the right row without
        // a side channel; the recorder dedupes at the SQL level so the
        // first non-empty capture wins even if a future code path
        // double-publishes. Skipped when no stage frame is in scope
        // (single-runner driver path) or when the upstream did not
        // surface a session id.
        let session_id = match run_result.as_ref() {
            Ok(rr) => rr.session_id.clone(),
            Err(_) => None,
        };
        if let (Some(stage_id), Some(sid)) = (ctx.stage_id, session_id) {
            if !sid.is_empty() {
                if let Err(err) = ctx
                    .bus
                    .publish(
                        Some(ctx.job_id),
                        Some(stage_id),
                        Some(self.task_id),
                        Event::StageSessionCaptured {
                            stage_id,
                            session_id: sid,
                        },
                        now_ms(),
                    )
                    .await
                {
                    tracing::warn!(?err, "stage session-id publish failed; continuing");
                }
            }
        }

        let outcome = match run_result {
            Err(e) => RunnerOutcome::Failed {
                reason: format!("claude runner input mismatch: {e}"),
            },
            Ok(rr) => match rr.error {
                Some(msg) => RunnerOutcome::Failed { reason: msg },
                None => RunnerOutcome::Completed,
            },
        };

        // Drop a structured handover into the worktree when the run
        // produced text we can parse. Done here (rather than in the
        // driver) because only the adapter has the accumulated
        // assistant message buffer. The handover is keyed by stage
        // (JOB-MODEL.md H1) — when this adapter runs without a stage
        // frame (legacy single-runner driver path, in-process tests)
        // we have no place to address the file, so we skip rather
        // than writing to a job-level fallback that keyed discovery
        // (H3) could not resolve.
        if let (Some(worktree), Some(stage_id)) = (ctx.worktree_path.as_ref(), ctx.stage_id) {
            let status = match &outcome {
                RunnerOutcome::Completed => JobStatus::Completed,
                RunnerOutcome::Failed { .. } => JobStatus::Failed,
            };
            let assistant_text = assistant_buf.lock().await.clone();
            let handover = match extract_handover(&assistant_text) {
                Some(h) => h,
                None => fallback_handover_from_text("claude", status, &assistant_text, 2000),
            };
            // Trio: flip the runtime-injected `Docs` row around the
            // handover write so the stage-completion gate has the row
            // to resolve. Skipped when no store is wired in (legacy
            // test harness path) — the trio is store-backed and there
            // would be no row to flip.
            if let Some(store) = self.store.as_deref() {
                emit_trio_started(&ctx, store, self.task_id, stage_id, TodoKind::Docs).await;
            }
            let write_result = write_handover(worktree, job_id, stage_id, &handover).await;
            let trio_status = match &write_result {
                Ok(path) => {
                    tracing::info!(handover = %path.display(), "claude handover written");
                    TodoStatus::Done
                }
                Err(err) => {
                    tracing::warn!(
                        ?err,
                        "failed to write claude handover; next session will read no prior handover"
                    );
                    TodoStatus::Failed
                }
            };
            if let Some(store) = self.store.as_deref() {
                emit_trio_completed(
                    &ctx,
                    store,
                    self.task_id,
                    stage_id,
                    TodoKind::Docs,
                    trio_status,
                )
                .await;
            }
        } else if ctx.stage_id.is_none() && ctx.worktree_path.is_some() {
            tracing::debug!(
                %job_id,
                "claude runner produced output without a stage frame; skipping handover write"
            );
        }

        outcome
    }
}
