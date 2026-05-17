//! Slack `events_api` envelope decoding — the wire shape that wraps
//! every inbound Socket Mode payload — plus the projection from that
//! shape onto the transport-agnostic [`InboundMessage`] the
//! [`codeless_bot_core::Dispatcher`] expects.
//!
//! This is the one piece of inbound plumbing that does not generalise
//! across chat platforms. The envelope structure (`type`,
//! `envelope_id`, `payload.event`) and the rules for what counts as a
//! dispatchable message (skip bot echoes, skip subtyped events, accept
//! `app_mention` and DM `message`) are specific to Slack's Socket
//! Mode protocol; their Telegram analogues live in
//! `codeless-telegram` and project onto the same [`InboundMessage`]
//! type from the other side of the trait.
//!
//! Dispatch itself — the parser, the RPC seam, the reply renderer —
//! is the shared [`codeless_bot_core::Dispatcher`].

use codeless_bot_core::{Dispatcher, InboundMessage};
use serde::Deserialize;

/// Slack `events_api` envelope shape, projected onto only the fields
/// the dispatcher reads. Everything else (team id, event timestamps,
/// reactions sub-payloads, etc.) is ignored — `#[serde(other)]` is not
/// needed because serde's default already discards unknown fields.
#[derive(Debug, Deserialize)]
pub struct EnvelopePayload {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub envelope_id: Option<String>,
    pub payload: Option<EventsApiPayload>,
}

#[derive(Debug, Deserialize)]
pub struct EventsApiPayload {
    pub event: Option<EventBody>,
}

#[derive(Debug, Deserialize)]
pub struct EventBody {
    #[serde(rename = "type")]
    pub kind: String,
    pub channel: Option<String>,
    pub user: Option<String>,
    pub text: Option<String>,
    pub thread_ts: Option<String>,
    /// Slack messages from the bot itself echo back with `bot_id` set;
    /// ignoring them prevents a reply loop where the bot answers its
    /// own confirmation.
    pub bot_id: Option<String>,
    /// Slack message subtypes (`bot_message`, `message_changed`,
    /// `message_deleted`, …) we do not want to dispatch on. The
    /// dispatcher skips any event with a non-empty subtype so an
    /// edit of a previously-typed command does not produce a second
    /// run.
    pub subtype: Option<String>,
}

/// Pull the dispatchable subset of an envelope. Returns `None` when
/// the envelope is not a dispatchable user message (bot echo, message
/// edit, hello frame, ack-only event). The caller still acks every
/// envelope by id; this function only decides whether to *run* the
/// command parser against the body.
pub fn extract_inbound(env: &EnvelopePayload) -> Option<InboundMessage> {
    if env.kind.as_deref() != Some("events_api") {
        return None;
    }
    let event = env.payload.as_ref()?.event.as_ref()?;
    // Two event types deliver operator commands: `app_mention` (the
    // bot was @-tagged in a channel) and `message` in a DM. Other
    // event kinds (reactions, channel joins, …) are ignored — the
    // SCOPE doc rules out reactions-as-decisions explicitly, and DM
    // message events are how Slack routes a direct conversation.
    if event.kind != "app_mention" && event.kind != "message" {
        return None;
    }
    if event.bot_id.is_some() {
        return None;
    }
    if event.subtype.is_some() {
        return None;
    }
    let channel = event.channel.clone()?;
    let text = event.text.clone()?;
    Some(InboundMessage {
        chat: channel,
        user: event.user.clone(),
        text,
        reply_to: event.thread_ts.clone(),
    })
}

/// Decode one Slack text frame into an [`EnvelopePayload`]. Returns
/// `Err` only for clearly malformed JSON; callers downcast the
/// `Option<...>` fields to decide whether to dispatch.
pub fn decode_envelope(text: &str) -> Result<EnvelopePayload, serde_json::Error> {
    serde_json::from_str(text)
}

/// Convenience: extract + dispatch in one call. The Socket Mode pump
/// uses this after acking each envelope so a slow `chat.postMessage`
/// reply does not stall the next inbound envelope.
pub async fn dispatch_envelope(disp: &Dispatcher, env: &EnvelopePayload) {
    if let Some(msg) = extract_inbound(env) {
        disp.dispatch_message(msg).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_inbound_ignores_bot_echoes() {
        let raw = serde_json::json!({
            "type": "events_api",
            "envelope_id": "abc",
            "payload": {
                "event": {
                    "type": "message",
                    "channel": "C1",
                    "text": "hi",
                    "bot_id": "B1",
                },
            },
        });
        let env: EnvelopePayload = serde_json::from_value(raw).unwrap();
        assert!(extract_inbound(&env).is_none());
    }

    #[test]
    fn extract_inbound_ignores_subtyped_messages() {
        let raw = serde_json::json!({
            "type": "events_api",
            "envelope_id": "abc",
            "payload": {
                "event": {
                    "type": "message",
                    "channel": "C1",
                    "text": "old",
                    "subtype": "message_changed",
                },
            },
        });
        let env: EnvelopePayload = serde_json::from_value(raw).unwrap();
        assert!(extract_inbound(&env).is_none());
    }

    #[test]
    fn extract_inbound_picks_up_app_mention() {
        let raw = serde_json::json!({
            "type": "events_api",
            "envelope_id": "abc",
            "payload": {
                "event": {
                    "type": "app_mention",
                    "channel": "C1",
                    "user": "U1",
                    "text": "<@U_BOT> status",
                    "thread_ts": "1700.0001",
                },
            },
        });
        let env: EnvelopePayload = serde_json::from_value(raw).unwrap();
        let msg = extract_inbound(&env).expect("dispatchable");
        assert_eq!(msg.chat, "C1");
        assert_eq!(msg.user.as_deref(), Some("U1"));
        assert_eq!(msg.text, "<@U_BOT> status");
        assert_eq!(msg.reply_to.as_deref(), Some("1700.0001"));
    }

    #[test]
    fn extract_inbound_ignores_non_events_api_envelopes() {
        let raw = serde_json::json!({"type": "hello"});
        let env: EnvelopePayload = serde_json::from_value(raw).unwrap();
        assert!(extract_inbound(&env).is_none());
    }
}
