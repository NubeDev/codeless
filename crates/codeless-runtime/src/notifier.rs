//! Outbound notification surface. `Notifier` is the abstract sink the
//! event-watcher hands every fan-out target — webhook today, Slack /
//! email / mobile push later — so the dispatch loop in
//! `spawn_notifier` never has to know which backend it is talking to.
//!
//! Only two event kinds trigger fan-out by SCOPE.md Phase 2c policy:
//! `JobFailed` (something needs operator attention) and
//! `ReviewRequested` (something needs operator approval). Other
//! events are ignored so a chatty AI run does not flood the
//! configured webhook with token deltas.
//!
//! Concrete HTTP / signing logic lives in
//! `codeless-adapters-host::webhook`; this crate only owns the trait
//! and the payload shape so the runtime stays free of host-side I/O
//! concerns.

use std::sync::Arc;

use async_trait::async_trait;
use codeless_rpc::RpcError;
use codeless_types::{Event, EventCursor, JobId, ReviewId, StageId, UnixMillis};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use crate::event_bus::{EventBus, SubscribeFilter};

/// Discriminator on the wire — the receiving side can pattern-match
/// without re-parsing the inner event. Stays kebab-case to match the
/// rest of the wire vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotificationKind {
    JobFailed,
    ReviewRequested,
}

/// Payload the `Notifier` receives. Carries enough identifiers for
/// the recipient to look up the originating row without re-deriving
/// them from the inner event variant, and the original `Event` for
/// callers that want the full shape (e.g. to render a richer Slack
/// card later).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPayload {
    pub kind: NotificationKind,
    pub cursor: EventCursor,
    pub job_id: Option<JobId>,
    pub stage_id: Option<StageId>,
    pub review_id: Option<ReviewId>,
    pub created_at: UnixMillis,
    pub event: Event,
}

#[derive(Debug, thiserror::Error)]
pub enum NotifierError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("non-success response: {status}")]
    Status { status: u16 },
}

#[async_trait]
pub trait Notifier: Send + Sync + 'static {
    async fn notify(&self, payload: NotificationPayload) -> Result<(), NotifierError>;
}

/// Subscribe to the bus and dispatch every matching event to the
/// supplied `Notifier`. Returns a `JoinHandle` so callers can abort
/// the watcher on shutdown. Delivery failures are logged at
/// `tracing::warn!` and dropped — the bus stream keeps flowing so a
/// flaky webhook does not stall the rest of the system.
///
/// Replay policy: starts from "live now" (`since: None`). The
/// notifier is not a durable consumer — a missed event during a core
/// restart is acceptable, matches SCOPE.md "notifications are a hint,
/// the durable record is the events table". A future Phase will add
/// at-least-once delivery with a persisted cursor.
pub async fn spawn_notifier(
    bus: Arc<EventBus>,
    notifier: Arc<dyn Notifier>,
) -> Result<JoinHandle<()>, RpcError> {
    let mut stream = bus
        .subscribe_since(SubscribeFilter::All, None)
        .await
        .map_err(|e| RpcError::Internal(format!("subscribe: {e}")))?;
    let handle = tokio::spawn(async move {
        while let Some(item) = stream.next().await {
            let env = match item {
                Ok(env) => env,
                Err(e) => {
                    tracing::warn!(error = %e, "notifier stream error");
                    continue;
                }
            };
            let kind = match &env.event {
                Event::JobFailed { .. } => NotificationKind::JobFailed,
                Event::ReviewRequested { .. } => NotificationKind::ReviewRequested,
                _ => continue,
            };
            let payload = NotificationPayload {
                kind,
                cursor: env.cursor,
                job_id: env.job_id,
                stage_id: env.stage_id,
                review_id: review_id_of(&env.event),
                created_at: env.created_at,
                event: env.event,
            };
            if let Err(e) = notifier.notify(payload).await {
                tracing::warn!(error = %e, "notifier delivery failed");
            }
        }
    });
    Ok(handle)
}

fn review_id_of(event: &Event) -> Option<ReviewId> {
    match event {
        Event::ReviewRequested { review_id, .. } => Some(*review_id),
        _ => None,
    }
}
