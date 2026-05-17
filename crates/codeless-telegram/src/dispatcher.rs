//! Telegram-specific inbound message projection.
//!
//! The transport-agnostic dispatcher lives in
//! [`codeless_bot_core::Dispatcher`] — it owns the parser, the
//! thread-context resolver, the RPC call, and the reply formatter.
//! This module exists to bridge a Telegram-shaped
//! [`crate::web_api::UpdateMessage`] onto a
//! [`codeless_bot_core::InboundMessage`]:
//!
//!   1. Pick the text (skipping non-message updates like sticker
//!      reactions or service messages — those have no `text`).
//!   2. Strip a leading `@bot_username` mention so the parser does
//!      not see the platform's mention syntax.
//!   3. Resolve a thread key: `message_thread_id` (forum topics)
//!      first, then `reply_to_message.message_id` (plain chat
//!      replies). Both round-trip through the
//!      [`codeless_bot_core::ThreadMap`] as strings so the same map
//!      can serve a future second adapter without a generic.
//!
//! Posting the reply is the dispatcher's job; this module never
//! touches the transport.

use codeless_bot_core::InboundMessage;

use crate::web_api::UpdateMessage;

/// Project a Telegram update onto a [`InboundMessage`]. Returns
/// `None` when the update has no usable text (service message,
/// sticker, photo with no caption — the bot has nothing to react
/// to). The `bot_username` should not include the leading `@`.
pub fn project_update(msg: &UpdateMessage, bot_username: &str) -> Option<InboundMessage> {
    let raw = msg.text.as_deref()?;
    let text = strip_mention(raw, bot_username).to_string();
    let reply_to = msg.message_thread_id.map(|id| id.to_string()).or_else(|| {
        msg.reply_to_message
            .as_ref()
            .map(|r| r.message_id.to_string())
    });
    Some(InboundMessage {
        chat: msg.chat.id.to_string(),
        user: msg.from.as_ref().map(|f| f.id.to_string()),
        text,
        reply_to,
    })
}

/// Strip a leading `@bot_username` mention if present, with the
/// match anchored at the start so a `@user something` body intended
/// for a different user passes through unchanged. The comparison is
/// case-insensitive because Telegram clients sometimes normalize
/// usernames to lowercase on send.
fn strip_mention<'a>(text: &'a str, bot_username: &str) -> &'a str {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix('@') else {
        return text;
    };
    let (name, tail) = match rest.find(|c: char| c.is_whitespace()) {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    if name.eq_ignore_ascii_case(bot_username) {
        tail.trim_start()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_api::{ReplyToMessage, UpdateChat, UpdateFrom};

    fn msg(text: Option<&str>) -> UpdateMessage {
        UpdateMessage {
            message_id: 1,
            chat: UpdateChat { id: 555 },
            from: Some(UpdateFrom { id: 42 }),
            text: text.map(str::to_owned),
            reply_to_message: None,
            message_thread_id: None,
        }
    }

    #[test]
    fn project_returns_none_for_text_less_update() {
        assert!(project_update(&msg(None), "bot").is_none());
    }

    #[test]
    fn project_keeps_text_when_no_mention_present() {
        let inbound = project_update(&msg(Some("status")), "bot").unwrap();
        assert_eq!(inbound.text, "status");
        assert_eq!(inbound.chat, "555");
        assert_eq!(inbound.user.as_deref(), Some("42"));
        assert!(inbound.reply_to.is_none());
    }

    #[test]
    fn project_strips_leading_bot_mention() {
        let inbound = project_update(
            &msg(Some("@aidan_codeless_bot status")),
            "aidan_codeless_bot",
        )
        .unwrap();
        assert_eq!(inbound.text, "status");
    }

    #[test]
    fn project_mention_match_is_case_insensitive() {
        let inbound = project_update(
            &msg(Some("@Aidan_Codeless_Bot status")),
            "aidan_codeless_bot",
        )
        .unwrap();
        assert_eq!(inbound.text, "status");
    }

    #[test]
    fn project_leaves_mention_of_other_user_alone() {
        let inbound =
            project_update(&msg(Some("@someone_else status")), "aidan_codeless_bot").unwrap();
        assert_eq!(inbound.text, "@someone_else status");
    }

    #[test]
    fn project_prefers_message_thread_id_over_reply_to() {
        let mut m = msg(Some("resume"));
        m.message_thread_id = Some(99);
        m.reply_to_message = Some(ReplyToMessage { message_id: 7 });
        let inbound = project_update(&m, "bot").unwrap();
        assert_eq!(inbound.reply_to.as_deref(), Some("99"));
    }

    #[test]
    fn project_falls_back_to_reply_to_message_id_for_plain_chats() {
        let mut m = msg(Some("resume"));
        m.reply_to_message = Some(ReplyToMessage { message_id: 7 });
        let inbound = project_update(&m, "bot").unwrap();
        assert_eq!(inbound.reply_to.as_deref(), Some("7"));
    }
}
