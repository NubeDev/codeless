//! Inbound message dispatcher. Strips a leading `@bot_username`
//! mention, calls the shared command parser in
//! `codeless_bot_core::command`, invokes the matching `RpcServer`
//! method, and posts the rendered reply via
//! [`crate::web_api::TelegramApi::send_message`].
//!
//! Thread context resolution: prefer `message_thread_id` (forum
//! topic), fall back to `reply_to_message.message_id` (plain chat).
//! The resolved value is what `codeless_bot_core::thread_map`
//! keys on.

use std::sync::Arc;

use crate::web_api::TelegramApi;

/// One inbound `message` update. Fields are only the subset the
/// dispatcher reads; the long-poll loop is free to ignore the rest.
pub struct InboundMessage {
    pub chat_id: i64,
    pub message_id: i64,
    pub message_thread_id: Option<i64>,
    pub reply_to_message_id: Option<i64>,
    pub text: String,
    pub from_user_id: Option<i64>,
}

pub async fn handle(
    _api: Arc<TelegramApi>,
    _bot_username: &str,
    _rpc: Arc<dyn codeless_rpc::RpcServer>,
    _msg: InboundMessage,
) {
    todo!(
        "1. strip leading @bot_username mention from text \
         2. codeless_bot_core::command::parse(text) \
         3. resolve thread context (message_thread_id || reply_to_message_id) \
         4. dispatch to RpcServer method \
         5. render reply via codeless_bot_core::reply \
         6. send_message with parse_mode=MarkdownV2"
    )
}
