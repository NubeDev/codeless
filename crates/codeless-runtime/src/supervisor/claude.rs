//! Claude-backed reply path for the supervisor's reactor, gated on
//! the `supervisor-claude` cargo feature (host-only by construction —
//! see the feature comment in `crates/codeless-runtime/Cargo.toml`).
//!
//! Wiring shape
//! ------------
//! The reactor in `supervisor::mod::react_to_chat` calls
//! `ClaudeReplyEngine::draft_reply(body)` per inbound non-supervisor
//! chat message and posts the returned text through
//! `Tools::post_chat_message`. The engine composes its `system_prompt`
//! from `prompt::SYSTEM_PROMPT` + `\n\n` + `prompt::TOOL_DESCRIPTIONS`
//! so the reviewer-readable text in `prompt.rs` is the single source
//! of truth for the model's framing; this file holds the runtime
//! plumbing (channel sink, ai-runner invocation), not the voice.
//!
//! Process-spawn confinement
//! -------------------------
//! `ai_runner::runners::claude::ClaudeRunner` routes process
//! invocations through the `ai-runner` crate, which is its own crate;
//! this module never names `std::process` or `tokio::process` (R1 of
//! CLAUDE.md + the lint test in `supervisor::mod`).

use ai_runner::{CliCfg, PermissionMode, Runner as AiRunner, RunnerInput};
use codeless_types::TaskId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::supervisor::prompt;

/// Compose the system prompt the supervisor hands to its Claude
/// session. Concatenating `SYSTEM_PROMPT` and `TOOL_DESCRIPTIONS` here
/// (rather than at `prompt.rs` definition time) keeps the two text
/// blocks independently reviewable: a doc reviewer pulls just
/// `SYSTEM_PROMPT` to evaluate the voice, just `TOOL_DESCRIPTIONS` to
/// evaluate the tool surface.
pub fn build_system_prompt() -> String {
    let mut s =
        String::with_capacity(prompt::SYSTEM_PROMPT.len() + prompt::TOOL_DESCRIPTIONS.len() + 4);
    s.push_str(prompt::SYSTEM_PROMPT);
    s.push_str("\n\n");
    s.push_str(prompt::TOOL_DESCRIPTIONS);
    s
}

/// Thin wrapper around `ai-runner`'s CLI Claude runner that produces a
/// single assistant reply from the user's chat message. Stateless
/// across turns: each `draft_reply` builds a fresh `RunnerInput::Cli`
/// and reads the aggregated assistant text off `RunResult.text`.
///
/// Why one-shot per turn: the supervisor's conversation context lives
/// in the persisted `chat_messages` table (see C1 of JOB-CHAT.md), not
/// in a Claude session id. A future stage may switch to `--continue`
/// for cheaper turns, but that requires capturing the session id onto
/// a per-Run column first; until then, each reply is its own request
/// and the chat thread is the durable transcript.
pub struct ClaudeReplyEngine {
    system_prompt: String,
    /// Model id forwarded to `claude-wrapper`. `None` lets the wrapper
    /// pick. Default points at the cheapest model (Haiku) per the
    /// JOB-CHAT.md (C2) §"Costs" budget.
    model: Option<String>,
}

impl Default for ClaudeReplyEngine {
    fn default() -> Self {
        Self {
            system_prompt: build_system_prompt(),
            model: Some("claude-haiku-latest".to_string()),
        }
    }
}

impl ClaudeReplyEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the model id. Empty string clears back to the wrapper
    /// default.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        let s = model.into();
        self.model = if s.is_empty() { None } else { Some(s) };
        self
    }

    /// Run one turn against the upstream Claude CLI. Returns the
    /// assistant's text reply, ready to hand to
    /// `Tools::post_chat_message`. `None` when the underlying runner
    /// failed or produced no assistant text — the reactor falls back
    /// to its hand-rolled matcher in that case rather than staying
    /// silent.
    pub async fn draft_reply(&self, user_body: &str) -> Option<String> {
        // Per-turn sink. The events are not republished onto the
        // codeless `EventBus`; the supervisor's voice contract says
        // every visible utterance flows through `post_chat_message`,
        // not through `AiToken` envelopes addressed to a Task that
        // does not exist for this turn.
        let (tx, mut rx) = mpsc::channel::<ai_runner::Event>(64);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let input = RunnerInput::Cli(CliCfg {
            prompt: user_body.to_string(),
            system_prompt: Some(self.system_prompt.clone()),
            model: self.model.clone(),
            // Supervisor turns must not touch disk; `Plan` keeps the
            // wrapper from invoking shell tools even if the model
            // tries to. The supervisor's read tools live on the
            // Codeless side of the boundary, not inside the wrapper.
            permission_mode: Some(PermissionMode::Plan),
            ..Default::default()
        });

        let runner = ai_runner::runners::claude::ClaudeRunner;
        let turn_id = TaskId::new();
        let cancel = CancellationToken::new();
        let res = runner
            .run(input, turn_id.to_string().into(), tx, cancel)
            .await;
        let _ = drain.await;

        match res {
            Ok(rr) if rr.error.is_none() => {
                let text = rr.text;
                if text.trim().is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The composed system prompt must contain both halves verbatim
    /// — concatenation order matters because the model reads the role
    /// framing before the tool surface. A future refactor that
    /// flipped the order would silently change behaviour, so the
    /// assertion checks the position of each fragment.
    #[test]
    fn build_system_prompt_concatenates_role_before_tools() {
        let s = build_system_prompt();
        let role_pos = s.find(prompt::SYSTEM_PROMPT).expect("role block present");
        let tools_pos = s
            .find(prompt::TOOL_DESCRIPTIONS)
            .expect("tools block present");
        assert!(role_pos < tools_pos, "role must come before tools");
    }
}
