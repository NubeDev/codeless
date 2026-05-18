//! `CodelessProvider` — implements [`ai_ui_core::Provider`] against
//! codeless's `ai-runner`.
//!
//! The provider holds an [`ai_runner::Registry`] and a chosen
//! [`ai_runner::Provider`]; per request it spawns a runner task feeding
//! an `mpsc::channel<Event>`, then re-shapes those events into
//! OpenAI-compatible streaming chunks via [`crate::sse::event_to_chunks`].
//!
//! Both CLI and REST runners are supported uniformly because both
//! ultimately emit `ai_runner::Event`. We pick the [`RunnerInput`] variant
//! that matches the chosen provider and forward the system prompt into
//! each config's `system_prompt` field.

use std::sync::Arc;

use ai_runner::{
    CliCfg, Event, OnEvent, Provider as RunnerProviderTag, Registry, RestCfg, RunnerInput,
    SessionId,
};
use ai_ui_core::{ChatStream, Provider, ProviderContext};
use ai_ui_types::ChatMessage;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// `ai-ui` provider backed by codeless's `ai-runner` stack.
///
/// Clone-cheap — internally an `Arc` around the runner registry.
#[derive(Clone)]
pub struct CodelessProvider {
    registry: Arc<Registry>,
    provider_tag: RunnerProviderTag,
    /// Channel capacity for the runner → adapter event stream. Matches
    /// the `ai-runner` default callsites in `codeless-runtime`.
    channel_capacity: usize,
}

impl CodelessProvider {
    /// Build a provider that dispatches to `provider_tag` via `registry`.
    pub fn new(registry: Arc<Registry>, provider_tag: RunnerProviderTag) -> Self {
        Self {
            registry,
            provider_tag,
            channel_capacity: 64,
        }
    }

    /// Override the runner-event channel capacity. Defaults to 64.
    pub fn with_channel_capacity(mut self, capacity: usize) -> Self {
        self.channel_capacity = capacity;
        self
    }

    /// Build the `RunnerInput` for this run.
    ///
    /// Per-provider transport selection:
    /// - `Claude`, `Codex`, `Copilot` → [`RunnerInput::Cli`].
    /// - `Anthropic`, `OpenAi` → [`RunnerInput::Rest`].
    ///
    /// `messages` is collapsed into a single prompt: the last `user`
    /// message becomes `prompt`; any earlier turns become `history`
    /// for REST runners. CLI runners drop history (the wrappers don't
    /// expose it in a stable way); that limitation is shared with the
    /// codeless job loop today.
    fn build_input(&self, ctx: ProviderContext, messages: Vec<ChatMessage>) -> RunnerInput {
        let (last_user, history) = split_last_user(&messages);
        let prompt = last_user.unwrap_or_default();
        let system_prompt = Some(ctx.system_prompt);

        match self.provider_tag {
            RunnerProviderTag::Claude | RunnerProviderTag::Codex | RunnerProviderTag::Copilot => {
                RunnerInput::Cli(CliCfg {
                    prompt,
                    system_prompt,
                    ..Default::default()
                })
            }
            RunnerProviderTag::Anthropic | RunnerProviderTag::OpenAi => {
                RunnerInput::Rest(RestCfg {
                    prompt,
                    system_prompt,
                    history,
                    ..Default::default()
                })
            }
        }
    }
}

impl Provider for CodelessProvider {
    fn stream_chat(&self, ctx: ProviderContext, messages: Vec<ChatMessage>) -> ChatStream {
        let chat_id = format!("chatcmpl-{}", ulid::Ulid::new());
        let session_id = SessionId::from(format!("ai-ui-{chat_id}"));
        let cancel = CancellationToken::new();
        let (tx, rx): (OnEvent, mpsc::Receiver<Event>) = mpsc::channel(self.channel_capacity);

        let runner_opt = self.registry.get(&self.provider_tag);
        let input = self.build_input(ctx, messages);
        let provider_tag = self.provider_tag.clone();

        // Drive the runner in a detached task. Errors are surfaced as a
        // synthetic `Error` event so the adapter side can translate them
        // through the same path as in-stream errors.
        let drive_tx = tx.clone();
        let drive_sid = session_id.clone();
        tokio::spawn(async move {
            let runner = match runner_opt {
                Some(r) => r,
                None => {
                    let _ = drive_tx
                        .send(Event {
                            session_id: drive_sid,
                            provider: provider_tag.to_string(),
                            kind: ai_runner::EventKind::Error {
                                message: format!(
                                    "provider `{provider_tag}` not registered in ai-runner registry"
                                ),
                            },
                        })
                        .await;
                    return;
                }
            };

            match runner
                .run(input, drive_sid.clone(), drive_tx.clone(), cancel)
                .await
            {
                Ok(result) => {
                    if let Some(err) = result.error {
                        let _ = drive_tx
                            .send(Event {
                                session_id: drive_sid,
                                provider: provider_tag.to_string(),
                                kind: ai_runner::EventKind::Error { message: err },
                            })
                            .await;
                    }
                }
                Err(e) => {
                    warn!(error = %e, "ai-runner returned WrongInputKind");
                    let _ = drive_tx
                        .send(Event {
                            session_id: drive_sid,
                            provider: provider_tag.to_string(),
                            kind: ai_runner::EventKind::Error {
                                message: e.to_string(),
                            },
                        })
                        .await;
                }
            }
        });

        // Re-shape Events → OpenAI streaming chunks.
        let stream = ReceiverStream::new(rx).flat_map(move |event| {
            let chunks = crate::sse::event_to_chunks(&chat_id, &event);
            futures::stream::iter(chunks)
        });

        Box::pin(stream) as ChatStream
    }
}

/// Split `messages` into `(last_user_content, prior_history)`.
///
/// `prior_history` keeps the original order of the messages **before** the
/// last user message; everything after the last user (typically nothing,
/// since the client just sent its turn) is dropped because the runner
/// expects the in-flight prompt to live in `CliCfg::prompt` /
/// `RestCfg::prompt`, not in history.
fn split_last_user(messages: &[ChatMessage]) -> (Option<String>, Vec<ai_runner::HistoryMessage>) {
    let last_user_idx = messages.iter().rposition(|m| m.role == "user");
    let last_user = last_user_idx.map(|i| stringify_content(&messages[i].content));
    let history = messages
        .iter()
        .take(last_user_idx.unwrap_or(messages.len()))
        .map(|m| ai_runner::HistoryMessage {
            role: m.role.clone(),
            content: stringify_content(&m.content),
        })
        .collect();
    (last_user, history)
}

fn stringify_content(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(role: &str, content: serde_json::Value) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content,
        }
    }

    #[test]
    fn split_last_user_picks_final_user_turn() {
        let msgs = vec![
            msg("user", json!("hi")),
            msg("assistant", json!("hello")),
            msg("user", json!("build me a dashboard")),
        ];
        let (last, history) = split_last_user(&msgs);
        assert_eq!(last.as_deref(), Some("build me a dashboard"));
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "hi");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].content, "hello");
    }

    #[test]
    fn split_last_user_handles_no_user_messages() {
        let msgs = vec![msg("system", json!("you are helpful"))];
        let (last, history) = split_last_user(&msgs);
        assert!(last.is_none());
        // No user → everything is history.
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn stringify_content_passes_string_through_and_json_encodes_objects() {
        assert_eq!(stringify_content(&json!("plain")), "plain");
        assert_eq!(
            stringify_content(&json!([{"type": "text", "text": "rich"}])),
            r#"[{"text":"rich","type":"text"}]"#
        );
    }
}
