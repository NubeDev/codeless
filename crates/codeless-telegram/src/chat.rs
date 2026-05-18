//! Per-Job chat forwarder for Telegram. Wires the substrate from
//! `DOCS/JOB-CHAT.md` ("Transport adapters") onto the Bot API:
//!
//!   1. Subscribe to the event bus (`ChatMessageAppended`).
//!   2. Drop messages whose `transport == Telegram` (echo suppression
//!      for the origin transport — the user already sees the row in
//!      their Telegram client; re-posting would double-render).
//!   3. Drop messages whose `metadata_json.delivery.telegram` is
//!      already set (presence-based idempotency for any stale event
//!      replay — the same receipt is what tells a restarted forwarder
//!      "already delivered, skip").
//!   4. Resolve every `(channel, thread)` on this transport bound to
//!      the message's Job via `list_chat_bindings_for_job`. Fan the
//!      body out to each one through [`crate::web_api::TelegramApi`]
//!      as plain text (no MarkdownV2 framing — the substrate's body
//!      is human chat, not a failure card).
//!   5. After a successful send call `update_chat_message_delivery`
//!      so the receipt lands on `chat_messages.metadata_json
//!      .delivery.telegram`. The runtime never touches `body` or
//!      `external_id` (JOB-CHAT.md "immutability bias" — the
//!      originating columns are append-only).
//!
//! The forwarder is intentionally separate from the existing
//! [`codeless_bot_core::OutboundPublisher`] that fans `JobFailed` /
//! `JobStopped` cards. Failure cards are a different surface (Surface
//! 2 in `SCOPE-TELEGRAM-INTEGRATION.md`) and they predate the per-Job
//! chat substrate; keeping the two loops independent means the chat
//! forwarder ships without perturbing the notification fan-out.

use std::sync::Arc;

use codeless_bot_core::EventSource;
use codeless_rpc::{ListChatBindingsForJobArgs, RpcServer, UpdateChatMessageDeliveryArgs};
use codeless_types::{ChatBinding, ChatMessage, ChatTransport, Event};
use futures_util::StreamExt;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::web_api::{SendMessageArgs, TelegramApi};

/// Background task that drains `ChatMessageAppended` events and
/// forwards the non-Telegram-origin ones to every Telegram binding
/// pointing at the message's Job. Hold the handle for the bot's
/// lifetime; [`ChatForwarder::shutdown`] signals the loop to exit at
/// the next event boundary and waits for the join.
pub struct ChatForwarder {
    join: JoinHandle<()>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl ChatForwarder {
    pub fn spawn(events: Arc<dyn EventSource>, rpc: Arc<dyn RpcServer>, api: TelegramApi) -> Self {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            run_loop(events, rpc, api, shutdown_rx).await;
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
    api: TelegramApi,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut stream = match events.subscribe_all().await {
        Ok(s) => s,
        Err(e) => {
            // Same shape as the failure-card publisher: a subscription
            // open failure logs and exits rather than panicking so the
            // long-poll loop keeps serving commands when the event
            // bus is unavailable.
            tracing::warn!(
                error = %e,
                "telegram: failed to open chat-forward subscription; forwarder disabled",
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
                            handle_message(&rpc, &api, message).await;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "telegram: chat-forward stream error");
                    }
                }
            }
        }
    }
}

async fn handle_message(rpc: &Arc<dyn RpcServer>, api: &TelegramApi, message: ChatMessage) {
    // Echo suppression for the origin transport. The user already
    // typed this message in their Telegram client; the row exists so
    // the supervisor and other surfaces can see it, but Telegram must
    // not echo it back to the same channel.
    if matches!(message.transport, ChatTransport::Telegram) {
        return;
    }
    // Presence-based idempotency: a prior successful send already
    // wrote `metadata.delivery.telegram` — JOB-CHAT.md "Idempotency"
    // says to skip on presence rather than re-send. The check is on
    // the event-payload snapshot, which is sufficient because the
    // forwarder subscribes live-only (no replay).
    if has_delivery_receipt(&message) {
        return;
    }
    let bindings = match rpc
        .list_chat_bindings_for_job(ListChatBindingsForJobArgs {
            job_id: message.job_id,
            transport: ChatTransport::Telegram,
        })
        .await
    {
        Ok(r) => r.bindings,
        Err(e) => {
            tracing::warn!(
                job_id = %message.job_id,
                error = %e,
                "telegram: list_chat_bindings_for_job failed; skipping forward",
            );
            return;
        }
    };
    if bindings.is_empty() {
        // No `/codeless bind` has been done for this Job on Telegram.
        // The row still exists in `chat_messages`; other transports
        // (web, slack) will paint it. Nothing more to do here.
        return;
    }
    for binding in bindings {
        forward_to_binding(rpc, api, &message, &binding).await;
    }
}

async fn forward_to_binding(
    rpc: &Arc<dyn RpcServer>,
    api: &TelegramApi,
    message: &ChatMessage,
    binding: &ChatBinding,
) {
    let thread = if binding.thread_id.is_empty() {
        None
    } else {
        // The thread sentinel column carries either the empty string
        // (no thread on this transport) or a stringified Telegram
        // `message_thread_id` from a forum topic. Parse failure means
        // the binding was minted by a different transport's writer —
        // treat the channel as un-threaded rather than fail the post.
        binding.thread_id.parse::<i64>().ok()
    };
    let body = render_body(message);
    let sent = match api
        .send_message(SendMessageArgs {
            chat_id: &binding.channel_id,
            text: &body,
            parse_mode: None,
            reply_to_message_id: None,
            message_thread_id: thread,
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                chat = %binding.channel_id,
                job_id = %message.job_id,
                error = %e,
                "telegram: chat forward send failed; no delivery receipt written",
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
            transport: ChatTransport::Telegram,
            platform_id: sent.message_id.to_string(),
        })
        .await
    {
        tracing::warn!(
            chat = %binding.channel_id,
            job_id = %message.job_id,
            message_id = %message.id,
            error = %e,
            "telegram: delivery receipt write failed; future replay may double-send",
        );
    }
}

/// Render the body the way the Telegram side should display it. The
/// supervisor's chat replies are plain prose; the user-typed messages
/// on other transports likewise. Author prefix lets a Telegram
/// reader tell `web` from `supervisor` without opening the web UI.
fn render_body(message: &ChatMessage) -> String {
    let prefix = match message.transport {
        ChatTransport::Web => "web",
        ChatTransport::Cli => "cli",
        ChatTransport::Slack => "slack",
        ChatTransport::Supervisor => "supervisor",
        // Origin-Telegram is filtered upstream so this arm is
        // unreachable in practice; the fallback keeps the renderer
        // total in case a future caller wires the function in
        // isolation.
        ChatTransport::Telegram => "telegram",
    };
    format!("[{prefix}] {}: {}", message.author, message.body)
}

/// Presence check on `metadata_json.delivery.telegram`. The substrate
/// stores `metadata_json` as raw JSON text (`codeless_types` is
/// mobile-safe and does not pull `serde_json`), so the parse happens
/// here. A malformed metadata blob is treated as "no receipt" — the
/// forward goes through and the receipt write either succeeds (the
/// parsed-by-us new value lands) or logs.
fn has_delivery_receipt(message: &ChatMessage) -> bool {
    let Some(text) = message.metadata_json.as_deref() else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    v.get("delivery")
        .and_then(|d| d.get("telegram"))
        .map(|v| !v.is_null())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{ChatRole, JobId, MessageId, UnixMillis};
    use serde_json::json;

    fn web_message_with_metadata(meta: Option<&str>) -> ChatMessage {
        ChatMessage {
            id: MessageId::new(),
            job_id: JobId::new(),
            run_id: None,
            transport: ChatTransport::Web,
            external_id: None,
            thread_key: None,
            author: "alice".into(),
            role: ChatRole::User,
            body: "hi".into(),
            metadata_json: meta.map(str::to_owned),
            created_at: UnixMillis(0),
        }
    }

    #[test]
    fn receipt_present_when_delivery_telegram_set() {
        let meta = json!({"delivery": {"telegram": "tg:99"}}).to_string();
        let msg = web_message_with_metadata(Some(&meta));
        assert!(has_delivery_receipt(&msg));
    }

    #[test]
    fn receipt_absent_when_delivery_only_for_other_transport() {
        let meta = json!({"delivery": {"slack": "ts:1.1"}}).to_string();
        let msg = web_message_with_metadata(Some(&meta));
        assert!(!has_delivery_receipt(&msg));
    }

    #[test]
    fn receipt_absent_when_metadata_null() {
        let msg = web_message_with_metadata(None);
        assert!(!has_delivery_receipt(&msg));
    }

    #[test]
    fn receipt_absent_when_metadata_malformed() {
        let msg = web_message_with_metadata(Some("not json"));
        assert!(!has_delivery_receipt(&msg));
    }

    #[test]
    fn render_body_prefixes_with_transport_and_author() {
        let mut msg = web_message_with_metadata(None);
        msg.author = "alice".into();
        msg.body = "stage 3 finished".into();
        msg.transport = ChatTransport::Supervisor;
        assert_eq!(
            render_body(&msg),
            "[supervisor] alice: stage 3 finished".to_string(),
        );
    }
}
