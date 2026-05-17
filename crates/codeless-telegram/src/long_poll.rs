//! Long-polling inbound loop. Counterpart of
//! `codeless_slack::socket_mode`. Calls `getUpdates` with a 30s
//! timeout and tracks `update_id + 1` as the next offset across
//! reconnects.
//!
//! Each inbound message is handed to [`crate::dispatcher::handle`]
//! which is responsible for parsing, calling the RPC, and posting
//! the reply.

use std::sync::Arc;

use crate::web_api::TelegramApi;

/// Spawn the long-poll loop. Returns once the underlying task has
/// been registered with the runtime; the loop itself runs forever
/// until the process exits.
pub async fn run(
    _api: Arc<TelegramApi>,
    _bot_username: String,
    _rpc: Arc<dyn codeless_rpc::RpcServer>,
) {
    todo!(
        "getUpdates loop: timeout=30, allowed_updates=[message], track offset, \
         backoff on network errors, hand each message to dispatcher::handle"
    )
}
