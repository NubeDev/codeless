//! Echo-suppression decision shared by every `ChatMessageAppended`
//! outbound forwarder. The rule comes straight from `DOCS/JOB-CHAT.md`
//! "Transport adapters" and is asymmetric on purpose:
//!
//!   - **Origin transport**: a row whose `transport` field equals the
//!     forwarder's own transport must NOT be re-posted. The user
//!     already sees that message in the originating client; pushing
//!     it back would double-render.
//!   - **Cross transport**: a row that came in on a different surface
//!     (web, CLI, supervisor, another bot) gets forwarded. After the
//!     send succeeds the forwarder writes a delivery receipt onto
//!     `metadata_json.delivery.<transport>`. Subsequent envelopes for
//!     the same row (process restart, replayed event) are skipped on
//!     presence of that receipt — receipts are the persistent record
//!     that "this row has already been delivered on this transport".
//!
//! The whole rule lives here so the Telegram and Slack adapters cannot
//! drift. Both must compute the same `Skip` / `Forward` answer for
//! the same input, otherwise a cross-transport message would either
//! get echoed twice or dropped entirely depending on which adapter
//! has the wrong copy of the check.
//!
//! The helper is pure (no I/O, no async). The fan-out loop, the API
//! calls, and the receipt-write UPDATE all stay in the per-transport
//! crate that owns them — only the decision moves here.

use codeless_types::{ChatMessage, ChatTransport};

/// What the forwarder for `this_transport` should do with one
/// `ChatMessageAppended` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Drop the message: either it originated on this transport
    /// (echo suppression) or a prior boot already delivered it
    /// (presence-based idempotency).
    Skip,
    /// Push the message out through this transport's send API and,
    /// on success, write the delivery receipt to
    /// `metadata_json.delivery.<transport>`.
    Forward,
}

/// Apply the asymmetric echo-suppression rule for `this_transport`
/// against `message`. The two `Skip` branches collapse to one
/// `Decision` so the caller does not have to know which gate fired
/// — the right answer is "do nothing" either way.
pub fn classify(this_transport: ChatTransport, message: &ChatMessage) -> Decision {
    if message.transport == this_transport {
        return Decision::Skip;
    }
    if has_delivery_receipt_for(this_transport, message) {
        return Decision::Skip;
    }
    Decision::Forward
}

/// True when `metadata_json.delivery.<transport_wire_name>` is set
/// to a non-null JSON value on the persisted row carried inside the
/// event payload. JOB-CHAT.md names this the "presence" check; a
/// missing receipt means the send did not happen, a non-null receipt
/// means it did.
///
/// Malformed `metadata_json` is treated as "no receipt" — the
/// forwarder will attempt the send and the success path either lands
/// a clean receipt (overwriting the garbage) or logs. That is a
/// better failure mode than dropping the message because the
/// metadata could not be parsed.
pub fn has_delivery_receipt_for(this_transport: ChatTransport, message: &ChatMessage) -> bool {
    let Some(text) = message.metadata_json.as_deref() else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    v.get("delivery")
        .and_then(|d| d.get(transport_wire_name(this_transport)))
        .map(|v| !v.is_null())
        .unwrap_or(false)
}

/// Lowercase ASCII wire name for a transport variant. Matches the
/// `#[serde(rename_all = "lowercase")]` derive on
/// [`ChatTransport`] and the SQL column contents — these are the
/// strings that flow through `metadata_json.delivery.*`.
///
/// Kept as an explicit `match` (not `serde_json::to_string`) because
/// the helper runs on every event and an allocation per envelope
/// is wasteful when the lookup is a five-arm static table.
pub fn transport_wire_name(transport: ChatTransport) -> &'static str {
    match transport {
        ChatTransport::Web => "web",
        ChatTransport::Cli => "cli",
        ChatTransport::Telegram => "telegram",
        ChatTransport::Slack => "slack",
        ChatTransport::Supervisor => "supervisor",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{ChatRole, JobId, MessageId, UnixMillis};
    use serde_json::json;

    fn msg(transport: ChatTransport, metadata_json: Option<String>) -> ChatMessage {
        ChatMessage {
            id: MessageId::new(),
            job_id: JobId::new(),
            run_id: None,
            transport,
            external_id: None,
            thread_key: None,
            author: "alice".into(),
            role: ChatRole::User,
            body: "hi".into(),
            metadata_json,
            created_at: UnixMillis(0),
        }
    }

    #[test]
    fn skip_when_origin_matches_this_transport() {
        let m = msg(ChatTransport::Telegram, None);
        assert_eq!(
            classify(ChatTransport::Telegram, &m),
            Decision::Skip,
            "telegram-origin row must not be echoed back to telegram",
        );
    }

    #[test]
    fn forward_when_origin_differs_and_no_receipt() {
        let m = msg(ChatTransport::Web, None);
        assert_eq!(classify(ChatTransport::Telegram, &m), Decision::Forward);
    }

    #[test]
    fn skip_when_receipt_for_this_transport_present() {
        let meta = json!({"delivery": {"telegram": "tg:99"}}).to_string();
        let m = msg(ChatTransport::Web, Some(meta));
        assert_eq!(classify(ChatTransport::Telegram, &m), Decision::Skip);
    }

    #[test]
    fn forward_when_receipt_only_for_other_transport() {
        // A slack-receipted web message must still forward on
        // telegram — the per-transport receipt namespace is what
        // lets the same row deliver on multiple surfaces.
        let meta = json!({"delivery": {"slack": "ts:1.1"}}).to_string();
        let m = msg(ChatTransport::Web, Some(meta));
        assert_eq!(classify(ChatTransport::Telegram, &m), Decision::Forward);
    }

    #[test]
    fn forward_when_metadata_is_malformed_json() {
        let m = msg(ChatTransport::Web, Some("not json".into()));
        assert_eq!(classify(ChatTransport::Telegram, &m), Decision::Forward);
    }

    #[test]
    fn receipt_check_uses_lowercase_wire_name() {
        // Adapters that look up the receipt using the Rust
        // identifier ("Telegram") would silently double-send; pin
        // the lowercase contract here so a refactor that flips the
        // casing fails this test rather than production traffic.
        let meta = json!({"delivery": {"Telegram": "tg:99"}}).to_string();
        let m = msg(ChatTransport::Web, Some(meta));
        assert!(!has_delivery_receipt_for(ChatTransport::Telegram, &m));
    }

    #[test]
    fn transport_wire_names_match_serde_lowercase_convention() {
        for (t, name) in [
            (ChatTransport::Web, "web"),
            (ChatTransport::Cli, "cli"),
            (ChatTransport::Telegram, "telegram"),
            (ChatTransport::Slack, "slack"),
            (ChatTransport::Supervisor, "supervisor"),
        ] {
            assert_eq!(transport_wire_name(t), name);
        }
    }
}
