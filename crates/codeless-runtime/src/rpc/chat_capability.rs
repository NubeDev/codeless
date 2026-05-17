//! Server-side derivation of a chat thread's effective tool surface.
//!
//! The substrate doc (`DOCS/PLUGIN-SUBSTRATE.md` item 3) pins this
//! shape: a thread's allowed-tools list is derived from durable server
//! state (the thread row, the persona it points at), never from a
//! client-passed routing discriminator. The acceptance is the
//! `CommonChat` "kind" prop: removing it from a call site must not
//! change which tools the runner will execute, because the server
//! never consulted it in the first place.
//!
//! Today the only durable state that can influence the tool list is
//! the job row (`agent_chat` resolves `session_id` to a job row when
//! one exists; footer chats pass a synthetic id and find no job). PS5
//! adds `allowed_tools` to the persona row; PS3, this stage, lands the
//! seam so PS5 is a one-line fill-in rather than a rewrite.
//!
//! `ChatMode` is the one client-side input the function accepts — the
//! spec-vs-work distinction is a per-turn *intent* signal, not a kind
//! discriminator, and the user toggles it deliberately from the
//! composer. The cleaner separation is that mode shapes the *prompt*
//! (preamble banner, primer text) while durable state shapes the
//! *capabilities*; until PS5 lands and personas carry their own clamps
//! the spec-mode clamp lives here as the work the function does.

use codeless_rpc::ChatMode;
use codeless_types::Job;

/// What the chat path needs to know about a thread's effective tool
/// surface to spawn a runner. Two orthogonal axes:
///
/// - `cli_tool_clamp` — the value forwarded as `--tools` to the
///   claude-style CLI runner, restricting which *built-in* tools
///   (Bash, Read, Edit, …) the agent may call. `None` means no clamp,
///   i.e. the runner's default set.
/// - `allowed_tools` — the persona-derived substrate-doc allowed-tools
///   list (literal ids and `prefix.*` globs, per
///   `codeless_types::allowed_tools`). This is the MCP-tool cap the
///   plugin substrate enforces. `None` means the persona has not
///   constrained MCP tools (PS5 fills this in; today it is always
///   `None`).
///
/// The two axes are deliberately distinct because the runner consumes
/// them via separate flags on the CLI side (`--tools` vs
/// `--allowed-tools`) with different semantics; collapsing them into
/// one list here would invent a coupling the substrate doc does not
/// describe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ChatCapabilities {
    pub(super) cli_tool_clamp: Option<String>,
    pub(super) allowed_tools: Option<Vec<String>>,
}

/// Derive the effective tool surface for one chat turn. Pure: the
/// function depends only on the durable job row (resolved by the
/// caller from `session_id`) and the per-turn `ChatMode`. No part of
/// the result is read from a UI routing prop such as the `CommonChat`
/// `kind` discriminator — the substrate-doc acceptance for PS3 is that
/// removing `kind` from a call site cannot change the answer here.
///
/// The function is the *only* place the chat path decides which tools
/// a turn may invoke. Code in `agent_chat` that wants to know the
/// effective clamp must call this; no other branching on `ChatMode`
/// for capability purposes is permitted.
pub(super) fn derive_chat_capabilities(
    _active_job: Option<&Job>,
    mode: ChatMode,
) -> ChatCapabilities {
    // Spec mode clamps CLI built-ins to the read+edit set so the agent
    // can author the job spec but cannot run Bash, hit the network, or
    // git commit. The list mirrors the spec-mode primer's "available"
    // claim; keep them in sync — the banner and the clamp are two ends
    // of the same contract.
    let cli_tool_clamp = match mode {
        ChatMode::Spec => Some(SPEC_MODE_CLI_TOOL_CLAMP.to_owned()),
        ChatMode::Work => None,
    };

    // PS5 fills this in from the active persona's `allowed_tools`
    // column. Until then the chat path does not have an MCP tool cap
    // to apply; the seam exists so the eventual change is in
    // `derive_chat_capabilities`, not in every caller.
    let allowed_tools = None;

    ChatCapabilities {
        cli_tool_clamp,
        allowed_tools,
    }
}

/// CLI built-in clamp applied when `ChatMode::Spec` is active. Mirrors
/// the prior inline `SPEC_MODE_ALLOWED_TOOLS` constant — moved here so
/// every capability decision flows through `derive_chat_capabilities`.
const SPEC_MODE_CLI_TOOL_CLAMP: &str = "Read,Edit,Write,Glob,Grep,LS,TodoWrite";

#[cfg(test)]
mod tests {
    use super::*;

    fn job_stub() -> Job {
        Job {
            id: codeless_types::JobId::new(),
            repo_id: codeless_types::RepoId::new(),
            prompt: None,
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/x".into(),
            workspace_mode: codeless_types::WorkspaceMode::Worktree,
            worktree_path: None,
            cost_cap_cents: codeless_types::CostCents::ZERO,
            wall_clock_cap_ms: 0,
            cost_cents: codeless_types::CostCents::ZERO,
            status: codeless_types::JobStatus::Draft,
            stop_reason: None,
            pending_operator_comment: None,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            precheck_override_once: false,
            created_at: codeless_types::UnixMillis(0),
            started_at: None,
            ended_at: None,
        }
    }

    #[test]
    fn spec_mode_clamps_cli_tools_regardless_of_job_presence() {
        let with_job = derive_chat_capabilities(Some(&job_stub()), ChatMode::Spec);
        let without_job = derive_chat_capabilities(None, ChatMode::Spec);
        assert_eq!(with_job, without_job);
        assert_eq!(
            with_job.cli_tool_clamp.as_deref(),
            Some(SPEC_MODE_CLI_TOOL_CLAMP),
        );
        assert!(with_job.allowed_tools.is_none());
    }

    #[test]
    fn work_mode_leaves_cli_tools_unclamped() {
        let caps = derive_chat_capabilities(Some(&job_stub()), ChatMode::Work);
        assert!(caps.cli_tool_clamp.is_none());
        assert!(caps.allowed_tools.is_none());
    }

    #[test]
    fn capabilities_depend_only_on_job_and_mode() {
        // PS3 acceptance: the answer is a function of the durable thread
        // row and the per-turn mode, nothing else. Two identical inputs
        // yield identical outputs; the function takes no other
        // arguments — there is no client discriminator to thread.
        let job = job_stub();
        for mode in [ChatMode::Work, ChatMode::Spec] {
            let a = derive_chat_capabilities(Some(&job), mode);
            let b = derive_chat_capabilities(Some(&job), mode);
            assert_eq!(a, b);
        }
    }
}
