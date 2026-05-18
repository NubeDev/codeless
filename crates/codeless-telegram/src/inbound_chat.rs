//! Per-Job chat substrate inbound handler for Telegram. Runs
//! alongside the existing command [`crate::dispatcher`] so the
//! `/status` / `/stop` / `/resume` command surface keeps working
//! unchanged; this module owns the two new paths from
//! `DOCS/JOB-CHAT.md`:
//!
//!   1. `/codeless bind <job_id>` — write the
//!      `(transport, channel, thread) -> job_id` row via
//!      `bind_chat_thread` so future inbound messages on the same
//!      thread can resolve to the Job.
//!   2. Any other text typed inside a bound thread — call
//!      `post_job_message` with `transport = Telegram` and the
//!      platform `message_id` as `external_id`. The partial unique
//!      index on `(transport, external_id)` is what guards against
//!      Telegram's at-least-once redelivery.
//!
//! The handler intentionally does NOT shortcut command messages —
//! `/status`, `/stop`, etc. flow through the existing dispatcher and
//! also become rows in `chat_messages` (so the supervisor sees what
//! the operator typed). The two surfaces compose because:
//!
//!   - The command dispatcher posts its reply on its own;
//!     `post_job_message` only mirrors the user's inbound text.
//!   - The chat forwarder echo-suppresses Telegram-origin rows, so
//!     the mirrored row never bounces back to the channel.

use std::sync::Arc;

use codeless_rpc::{
    BindChatThreadArgs, GetChatBindingArgs, PostJobMessageArgs, RpcError, RpcServer,
};
use codeless_types::{ChatRole, ChatTransport, JobId};

use crate::web_api::{SendMessageArgs, TelegramApi, UpdateMessage};

/// Entry point called from the long-poll loop for every inbound
/// message. The function is a no-op for transports that have no
/// bearing on the per-Job chat substrate (service messages, stickers,
/// updates without a `text` field) — those return early without
/// touching the runtime.
///
/// `api` is held for the bind-command acknowledgement post; without
/// it the operator typing `/codeless bind <job>` would see nothing
/// happen even on a successful runtime call, which is a worse
/// failure mode than the extra round-trip.
pub async fn handle_inbound(
    rpc: &Arc<dyn RpcServer>,
    api: &TelegramApi,
    update_msg: &UpdateMessage,
) {
    let Some(text) = update_msg.text.as_deref() else {
        return;
    };
    let trimmed = text.trim();
    let chat_id = update_msg.chat.id.to_string();
    let thread_id = update_msg
        .message_thread_id
        .map(|t| t.to_string())
        .unwrap_or_default();
    let author = update_msg
        .from
        .as_ref()
        .map(|f| f.id.to_string())
        .unwrap_or_else(|| "anonymous".to_string());
    let external_id = update_msg.message_id.to_string();

    if let Some(rest) = parse_bind_command(trimmed) {
        run_bind(
            rpc,
            api,
            &chat_id,
            &thread_id,
            &author,
            rest,
            update_msg.message_id,
        )
        .await;
        return;
    }

    // Mirror the message into the per-Job chat thread if the channel
    // is bound. An unbound channel's chat messages are intentionally
    // dropped — the substrate only ingests text the operator has
    // pointed at a Job via `/codeless bind <job>`. The command
    // dispatcher's command-reply path still handles command-shaped
    // input.
    if let Err(e) = mirror_to_chat(rpc, &chat_id, &thread_id, &author, &external_id, trimmed).await
    {
        match e {
            // `Conflict` is the partial-unique-index guard kicking in:
            // Telegram redelivered an `update_id` we already ingested,
            // which is the at-least-once defence working as designed.
            // Drop the duplicate silently rather than spam a warn line
            // on every redelivery the platform throws at us.
            RpcError::Conflict(_) => {}
            other => {
                tracing::warn!(
                    chat = %chat_id,
                    error = %other,
                    "telegram: post_job_message for chat-mirror failed",
                );
            }
        }
    }
}

/// Strip the leading `/codeless bind` (or the bare-word `codeless
/// bind`) prefix and return the trailing job-id token, if any. The
/// slash-prefix is the conventional Telegram form; the bare-word
/// alternative is the same shape the bot-core command parser already
/// accepts so a thread reply without the slash still works.
fn parse_bind_command(text: &str) -> Option<&str> {
    // `strip_prefix` alone would match `/codeless bindXYZ` and silently
    // accept it; require the verb to be followed by a word boundary
    // (end of string, whitespace, or a non-alphanumeric char) so a
    // typo'd bind verb falls through to the chat-mirror path rather
    // than landing on the bind branch with `Xyz` as the "job id".
    let rest = text
        .strip_prefix("/codeless bind")
        .or_else(|| text.strip_prefix("codeless bind"))?;
    match rest.chars().next() {
        None => Some(""),
        Some(c) if c.is_whitespace() => Some(rest.trim()),
        _ => None,
    }
}

async fn run_bind(
    rpc: &Arc<dyn RpcServer>,
    api: &TelegramApi,
    chat_id: &str,
    thread_id: &str,
    bound_by: &str,
    rest: &str,
    reply_to: i64,
) {
    let job_id = match rest.split_whitespace().next() {
        Some(s) => match s.parse::<JobId>() {
            Ok(j) => j,
            Err(_) => {
                post_ack(api, chat_id, reply_to, "[fail] bind: expected a job id").await;
                return;
            }
        },
        None => {
            post_ack(
                api,
                chat_id,
                reply_to,
                "[fail] bind: usage `/codeless bind <job_id>`",
            )
            .await;
            return;
        }
    };
    let thread_id_arg = if thread_id.is_empty() {
        None
    } else {
        Some(thread_id.to_string())
    };
    match rpc
        .bind_chat_thread(BindChatThreadArgs {
            transport: ChatTransport::Telegram,
            channel_id: chat_id.to_string(),
            thread_id: thread_id_arg,
            job_id,
            bound_by: bound_by.to_string(),
        })
        .await
    {
        Ok(_) => {
            post_ack(
                api,
                chat_id,
                reply_to,
                &format!("[ok] bound this thread to {job_id}"),
            )
            .await;
        }
        Err(e) => {
            post_ack(api, chat_id, reply_to, &format!("[fail] bind: {e}")).await;
        }
    }
}

async fn mirror_to_chat(
    rpc: &Arc<dyn RpcServer>,
    chat_id: &str,
    thread_id: &str,
    author: &str,
    external_id: &str,
    body: &str,
) -> Result<(), RpcError> {
    // Resolve the binding before posting — `post_job_message` requires
    // a `job_id` and we have only `(channel, thread)`. The
    // `get_chat_binding` lookup is the same path the substrate's
    // store-side helper exposes; transport adapters keep no in-memory
    // cache (per `JOB-CHAT.md`) so a freshly-rebound channel is
    // visible on the next inbound without an adapter restart.
    let thread_arg = if thread_id.is_empty() {
        None
    } else {
        Some(thread_id.to_string())
    };
    let binding_res = rpc
        .get_chat_binding(GetChatBindingArgs {
            transport: ChatTransport::Telegram,
            channel_id: chat_id.to_string(),
            thread_id: thread_arg,
        })
        .await?;
    let Some(binding) = binding_res.binding else {
        return Ok(());
    };
    let job_id = binding.job_id;
    let thread_key = if thread_id.is_empty() {
        None
    } else {
        Some(thread_id.to_string())
    };
    rpc.post_job_message(PostJobMessageArgs {
        job_id,
        transport: ChatTransport::Telegram,
        external_id: Some(external_id.to_string()),
        thread_key,
        author: author.to_string(),
        role: ChatRole::User,
        body: body.to_string(),
        metadata_json: None,
    })
    .await?;
    Ok(())
}

async fn post_ack(api: &TelegramApi, chat_id: &str, reply_to: i64, text: &str) {
    if let Err(e) = api
        .send_message(SendMessageArgs {
            chat_id,
            text,
            parse_mode: None,
            reply_to_message_id: Some(reply_to),
            message_thread_id: None,
        })
        .await
    {
        tracing::warn!(chat = %chat_id, error = %e, "telegram: bind ack post failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bind_command_accepts_slash_form() {
        assert_eq!(
            parse_bind_command("/codeless bind 01H").map(str::to_string),
            Some("01H".to_string()),
        );
    }

    #[test]
    fn parse_bind_command_accepts_bare_form() {
        assert_eq!(
            parse_bind_command("codeless bind 01H").map(str::to_string),
            Some("01H".to_string()),
        );
    }

    #[test]
    fn parse_bind_command_returns_none_for_other_commands() {
        assert!(parse_bind_command("/status").is_none());
        assert!(parse_bind_command("hello").is_none());
    }

    #[test]
    fn parse_bind_command_returns_empty_string_when_id_missing() {
        // Caller distinguishes "/codeless bind" (no id) from
        // "/codeless bindXYZ" (no space) — the latter must NOT be a
        // bind command. Only the former (with whitespace or end of
        // input after the verb) hits the empty-string branch in
        // `run_bind` and renders the usage error.
        assert_eq!(
            parse_bind_command("/codeless bind").map(str::to_string),
            Some("".to_string())
        );
        assert!(parse_bind_command("/codeless bindX").is_none());
    }
}
