//! Per-Job chat forwarder for Slack. Mirrors
//! [`codeless_telegram::chat::ChatForwarder`] one-for-one against the
//! Slack Web API:
//!
//!   1. Subscribe to the event bus (`ChatMessageAppended`).
//!   2. Drop messages whose `transport == Slack` (echo suppression for
//!      the origin transport — the user already sees the row in their
//!      Slack client; re-posting would double-render).
//!   3. Drop messages whose `metadata_json.delivery.slack` is already
//!      set (presence-based idempotency for any stale event replay —
//!      the same receipt is what tells a restarted forwarder
//!      "already delivered, skip").
//!   4. Resolve every `(channel, thread)` on this transport bound to
//!      the message's Job via `list_chat_bindings_for_job`. Fan the
//!      body out to each one through [`crate::web_api::ChatPoster`]
//!      as plain text. Slack threads are identified by the parent
//!      message's `thread_ts`; a bound row with a non-empty
//!      `thread_id` posts as a reply inside that thread, an empty
//!      `thread_id` posts at channel level.
//!   5. After a successful post call `update_chat_message_delivery`
//!      so the receipt lands on `chat_messages.metadata_json
//!      .delivery.slack`. The runtime never touches `body` or
//!      `external_id` (JOB-CHAT.md "immutability bias").
//!
//! The forwarder is intentionally separate from the existing
//! [`codeless_bot_core::OutboundPublisher`] that fans `JobFailed` /
//! `JobStopped` cards. Failure cards are a different surface (Surface
//! 2 in `SCOPE-SLACK-INTEGRATION.md`) and they predate the per-Job
//! chat substrate; keeping the two loops independent means the chat
//! forwarder ships without perturbing the notification fan-out.
//!
//! The asymmetric echo-suppression rule itself lives in
//! [`codeless_bot_core::chat_forward`] so this module and the
//! Telegram-side equivalent cannot disagree — both call into the same
//! [`classify`] helper. Mirroring the file structure between the two
//! adapters is deliberate: a bug fix in either crate's `chat.rs`
//! should land in the other with the smallest possible diff.

use std::sync::Arc;

use codeless_bot_core::chat_forward::{classify, Decision};
use codeless_bot_core::EventSource;
use codeless_rpc::{ListChatBindingsForJobArgs, RpcServer, UpdateChatMessageDeliveryArgs};
use codeless_types::{ChatBinding, ChatMessage, ChatTransport, Event};
use futures_util::StreamExt;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::web_api::ChatPoster;

/// Background task that drains `ChatMessageAppended` events and
/// forwards the non-Slack-origin ones to every Slack binding pointing
/// at the message's Job. Hold the handle for the bot's lifetime;
/// [`ChatForwarder::shutdown`] signals the loop to exit at the next
/// event boundary and waits for the join.
pub struct ChatForwarder {
    join: JoinHandle<()>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl ChatForwarder {
    pub fn spawn(
        events: Arc<dyn EventSource>,
        rpc: Arc<dyn RpcServer>,
        poster: ChatPoster,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            run_loop(events, rpc, poster, shutdown_rx).await;
        });
        Self {
            join,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.join.await;
    }
}

async fn run_loop(
    events: Arc<dyn EventSource>,
    rpc: Arc<dyn RpcServer>,
    poster: ChatPoster,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut stream = match events.subscribe_all().await {
        Ok(s) => s,
        Err(e) => {
            // Same shape as the failure-card publisher: a subscription
            // open failure logs and exits rather than panicking so the
            // Socket Mode loop keeps serving commands when the event
            // bus is unavailable.
            tracing::warn!(
                error = %e,
                "slack: failed to open chat-forward subscription; forwarder disabled",
            );
            return;
        }
    };
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => return,
            next = stream.next() => {
                let Some(item) = next else { return };
                match item {
                    Ok(env) => {
                        if let Event::ChatMessageAppended { message, .. } = env.event {
                            handle_message(&rpc, &poster, message).await;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "slack: chat-forward stream error");
                    }
                }
            }
        }
    }
}

async fn handle_message(rpc: &Arc<dyn RpcServer>, poster: &ChatPoster, message: ChatMessage) {
    // The asymmetric echo-suppression rule lives in
    // `codeless_bot_core::chat_forward` so the Telegram and Slack
    // forwarders cannot disagree. Both the origin-transport skip and
    // the presence-based receipt skip collapse into the single
    // `Decision::Skip` answer here — see JOB-CHAT.md "Transport
    // adapters" for the rule statement.
    if matches!(classify(ChatTransport::Slack, &message), Decision::Skip) {
        return;
    }
    let bindings = match rpc
        .list_chat_bindings_for_job(ListChatBindingsForJobArgs {
            job_id: message.job_id,
            transport: ChatTransport::Slack,
        })
        .await
    {
        Ok(r) => r.bindings,
        Err(e) => {
            tracing::warn!(
                job_id = %message.job_id,
                error = %e,
                "slack: list_chat_bindings_for_job failed; skipping forward",
            );
            return;
        }
    };
    if bindings.is_empty() {
        // No `/codeless bind` has been done for this Job on Slack.
        // The row still exists in `chat_messages`; other transports
        // (web, telegram) will paint it. Nothing more to do here.
        return;
    }
    for binding in bindings {
        forward_to_binding(rpc, poster, &message, &binding).await;
    }
}

async fn forward_to_binding(
    rpc: &Arc<dyn RpcServer>,
    poster: &ChatPoster,
    message: &ChatMessage,
    binding: &ChatBinding,
) {
    // Slack's thread identifier is the parent message's `ts`
    // (a string like `"1700.0001"`). The binding row stores the
    // empty string when the channel is un-threaded; an un-threaded
    // post passes `None` to `ChatPoster::post`, an in-thread post
    // forwards the ts as-is.
    let thread_ts = if binding.thread_id.is_empty() {
        None
    } else {
        Some(binding.thread_id.as_str())
    };
    let body = render_body(message);
    let posted = match poster.post(&binding.channel_id, &body, thread_ts).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                channel = %binding.channel_id,
                job_id = %message.job_id,
                error = %e,
                "slack: chat forward post failed; no delivery receipt written",
            );
            return;
        }
    };
    // Receipt write is best-effort logged: a failure here means a
    // future replay (process restart) would re-send. JOB-CHAT.md
    // names the trade-off as `metadata.delivery.*` being the
    // presence check; without a receipt the next forwarder boot
    // double-sends, which is a less bad failure than dropping the
    // message would have been. The runtime side is one UPDATE.
    if let Err(e) = rpc
        .update_chat_message_delivery(UpdateChatMessageDeliveryArgs {
            message_id: message.id,
            transport: ChatTransport::Slack,
            platform_id: posted.ts,
        })
        .await
    {
        tracing::warn!(
            channel = %binding.channel_id,
            job_id = %message.job_id,
            message_id = %message.id,
            error = %e,
            "slack: delivery receipt write failed; future replay may double-send",
        );
    }
}

/// Render the body the way the Slack side should display it. The
/// supervisor's chat replies are plain prose; the user-typed messages
/// on other transports likewise. Author prefix lets a Slack reader
/// tell `web` from `supervisor` without opening the web UI.
fn render_body(message: &ChatMessage) -> String {
    let prefix = match message.transport {
        ChatTransport::Web => "web",
        ChatTransport::Cli => "cli",
        ChatTransport::Telegram => "telegram",
        ChatTransport::Supervisor => "supervisor",
        // Origin-Slack is filtered upstream so this arm is
        // unreachable in practice; the fallback keeps the renderer
        // total in case a future caller wires the function in
        // isolation.
        ChatTransport::Slack => "slack",
    };
    format!("[{prefix}] {}: {}", message.author, message.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{ChatRole, JobId, MessageId, UnixMillis};

    #[test]
    fn render_body_prefixes_with_transport_and_author() {
        let msg = ChatMessage {
            id: MessageId::new(),
            job_id: JobId::new(),
            run_id: None,
            transport: ChatTransport::Supervisor,
            external_id: None,
            thread_key: None,
            author: "alice".into(),
            role: ChatRole::User,
            body: "stage 3 finished".into(),
            metadata_json: None,
            created_at: UnixMillis(0),
        };
        assert_eq!(
            render_body(&msg),
            "[supervisor] alice: stage 3 finished".to_string(),
        );
    }
}
