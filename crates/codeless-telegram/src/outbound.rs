//! Outbound publisher. Subscribes to the runtime event bus and
//! posts a MarkdownV2 failure card to the configured
//! `telegram_chat_id` on `JobFailed` / `JobStopped`, debounced per
//! job (5 minutes). The returned `message_id` is registered in the
//! shared `codeless_bot_core::thread_map::ThreadMap` so a bare-verb
//! reply in the same thread (`stop`, `resume bypass`) resolves to
//! the right job id without the operator retyping it.
//!
//! Counterpart of `codeless_slack::outbound::OutboundPublisher`.

use std::sync::Arc;

use crate::web_api::TelegramApi;

pub async fn run(
    _api: Arc<TelegramApi>,
    _chat_id: String,
    _rpc: Arc<dyn codeless_rpc::RpcServer>,
) {
    todo!(
        "1. subscribe to event bus via rpc.subscribe_events() \
         2. filter JobFailed | JobStopped \
         3. per-job 5-minute debounce (reuse codeless_bot_core debounce) \
         4. render via codeless_bot_core::notify \
         5. send_message with parse_mode=MarkdownV2 \
         6. register returned message_id in ThreadMap"
    )
}
