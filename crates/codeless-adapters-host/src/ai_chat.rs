//! Single-turn chat dispatch onto an `ai-runner` CLI runner.
//!
//! The footer "AI agent" panel routes each prompt through here: pick a
//! CLI runner by wire id (`claude` / `codex` / `copilot`), run it once
//! against the current working directory, and stream upstream events
//! through a caller-supplied publisher closure. The closure owns the
//! codeless event-bus side, exactly as `ai_runner_bridge::forward_events`
//! does for the job-driven path — adapters-host stays a leaf in the
//! codeless crate graph and the runtime keeps owning persistence.
//!
//! Process spawn lives in `ai-runner` (and in this crate, by R1). Mobile
//! crates cannot reach this module: it sits on the host side of the
//! dependency graph because every CLI runner shells out a child.

use std::path::PathBuf;
use std::sync::Arc;

use ai_runner::{
    CliCfg, Event as AiEvent, PermissionMode, Provider, Registry, RunnerInput, SessionId,
};
use codeless_types::{Event, TaskId};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::ai_runner_bridge::forward_events;

/// Parse the wire runner id used by `AgentChatArgs.runner`. REST
/// providers (`anthropic`, `openai`) deliberately return `None` —
/// the browser uses its own keys for those over `DirectChatTransport`,
/// not via the runtime, so a call here is a misuse the caller should
/// reject with `InvalidArgument` rather than silently route somewhere
/// useless.
pub fn parse_cli_runner_id(id: &str) -> Option<Provider> {
    match id {
        "claude" => Some(Provider::Claude),
        "codex" => Some(Provider::Codex),
        "copilot" => Some(Provider::Copilot),
        _ => None,
    }
}

/// Probe each CLI runner registered in `registry` and return the wire
/// ids that report `ready() == true`. Probes touch the filesystem (CLI
/// runners locate their binary the same way the real run does) so this
/// runs once at boot and the result is cached in `ServerInfo`. Mock or
/// REST providers are excluded because the footer panel only routes
/// CLI runners through `agent_chat`.
pub async fn probe_available_cli_runners(registry: &Registry) -> Vec<String> {
    let candidates: [(&str, Provider); 3] = [
        ("claude", Provider::Claude),
        ("codex", Provider::Codex),
        ("copilot", Provider::Copilot),
    ];
    let mut out = Vec::new();
    for (id, provider) in candidates {
        if let Some(runner) = registry.get(&provider) {
            if runner.ready().await {
                out.push(id.to_owned());
            }
        }
    }
    out
}

/// Input to a single chat turn. Bundles the prompt-facing parameters
/// that vary per call, leaving infrastructure (registry, task id,
/// cancel token) at the call site.
pub struct ChatRunCfg {
    pub provider: Provider,
    pub prompt: String,
    pub cwd: PathBuf,
    /// Comma-separated list of built-in tool names forwarded as `--tools`
    /// to the claude binary. `None` leaves the full default tool set.
    /// Spec mode sets this to restrict the agent to read + edit tools.
    /// This is `--tools` (built-in tool restriction), not `--allowed-tools`
    /// (MCP server permissions — distinct flag, different semantics).
    pub tools: Option<String>,
}

/// Run one chat turn. Spawns the runner, drains its event stream
/// through the bridge translator, hands each `Event` to `publish`.
///
/// Cancellation: when `cancel` fires, the spawned child is killed
/// (CLI runners hold their `Child` with `kill_on_drop(true)`), the
/// upstream `mpsc::Sender` drops, the bridge forwarder drains the
/// remaining events, and this function returns `Ok(())`.
pub async fn run_chat<F, Fut, E>(
    registry: Arc<Registry>,
    cfg: ChatRunCfg,
    task_id: TaskId,
    publish: F,
    cancel: CancellationToken,
) -> Result<(), AgentChatError<E>>
where
    F: FnMut(Event) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), E>> + Send,
    E: Send + 'static,
{
    let runner = registry
        .get(&cfg.provider)
        .ok_or(AgentChatError::RunnerNotRegistered)?;

    let (tx, rx) = mpsc::channel::<AiEvent>(CHANNEL_CAPACITY);

    let forwarder = tokio::spawn(async move { forward_events(rx, task_id, publish).await });

    let input = RunnerInput::Cli(CliCfg {
        prompt: cfg.prompt,
        work_dir: Some(cfg.cwd.to_string_lossy().into_owned()),
        // Same headless rationale as `ClaudeRunnerAdapter`: no TTY user
        // is available to approve mid-run tool calls, and the runner
        // executes in the server's cwd rather than a worktree, so any
        // approval prompt would deadlock the chat turn.
        permission_mode: Some(PermissionMode::Bypass),
        // `CliCfg::tools` restricts which built-in tools (Bash, Read, …)
        // the agent may call, forwarded as `--tools` to the claude binary.
        // This is the right knob for spec mode's tool restriction — distinct
        // from `allowed_tools` which gates MCP server permissions only.
        tools: cfg.tools,
        ..Default::default()
    });

    let session_id: SessionId = task_id.to_string().into();
    let run_outcome = runner.run(input, session_id, tx, cancel).await;

    let forwarder_outcome = forwarder
        .await
        .map_err(|e| AgentChatError::ForwarderJoin(e.to_string()))?;
    forwarder_outcome.map_err(AgentChatError::Publish)?;

    if let Err(e) = run_outcome {
        tracing::warn!(error = %e, "ai-runner reported a runner-level error");
    }
    Ok(())
}

/// Channel capacity for the upstream `mpsc::Sender<ai_runner::Event>`.
/// Same value the job-side `ClaudeRunnerAdapter` uses: enough headroom
/// for the bridge to keep up under bus contention without letting a
/// runaway runner balloon memory.
const CHANNEL_CAPACITY: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum AgentChatError<E> {
    #[error("runner not registered for provider")]
    RunnerNotRegistered,
    #[error("event forwarder task panicked: {0}")]
    ForwarderJoin(String),
    #[error("event publish failed")]
    Publish(#[source] E),
}
