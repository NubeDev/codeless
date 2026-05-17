use std::sync::Arc;

use futures_util::StreamExt;
use serde::Deserialize;
use tauri::State;
use tokio_util::sync::CancellationToken;

use codeless_rpc::{EventFilter, Since};
use codeless_types::EventEnvelope;

use crate::error::CommandResult;
use crate::state::{AppState, SubscriptionMap};

#[derive(Deserialize)]
pub struct SubscribeArgs {
    filter: EventFilter,
    since: Since,
}

#[tauri::command]
pub async fn rpc_subscribe(
    state: State<'_, AppState>,
    args: SubscribeArgs,
    channel: tauri::ipc::Channel<EventEnvelope>,
) -> CommandResult<()> {
    let stream = state.rpc.subscribe(args.filter, args.since).await?;
    let channel_id = channel.id();
    let token = CancellationToken::new();
    state.subs.lock().insert(channel_id, token.clone());

    let subs = Arc::clone(&state.subs);
    tokio::spawn(async move {
        let _guard = SubDropGuard { subs, channel_id };
        tokio::pin!(stream);
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                item = stream.next() => {
                    match item {
                        Some(Ok(env)) => {
                            if channel.send(env).is_err() {
                                break;
                            }
                        }
                        Some(Err(_)) | None => break,
                    }
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn rpc_unsubscribe(state: State<'_, AppState>, channel_id: u32) -> CommandResult<()> {
    if let Some(token) = state.subs.lock().remove(&channel_id) {
        token.cancel();
    }
    Ok(())
}

/// Removes the subscription entry when the forwarder task exits,
/// regardless of the exit reason (stream end, channel error,
/// cancellation).
struct SubDropGuard {
    subs: Arc<SubscriptionMap>,
    channel_id: u32,
}

impl Drop for SubDropGuard {
    fn drop(&mut self) {
        self.subs.lock().remove(&self.channel_id);
    }
}
