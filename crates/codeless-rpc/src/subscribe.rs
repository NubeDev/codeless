use std::pin::Pin;

use codeless_types::{EventCursor, EventEnvelope, JobId};
use futures_core::Stream;
use serde::{Deserialize, Serialize};

/// Filter passed to `RpcServer::subscribe`. Variants stay coarse on
/// purpose — per-stage / per-task filtering happens client-side from
/// the same event stream so the server doesn't need to multiplex N
/// fine-grained channels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "kebab-case")]
pub enum EventFilter {
    /// Every event the runtime emits — used by the global event log view.
    All,
    /// Only events tagged with this job (and its stages and tasks).
    Job { job_id: JobId },
}

/// Resume cursor passed alongside `EventFilter`. `None` means "live
/// only — drop replay"; `Some(cursor)` means "replay everything strictly
/// after this cursor, then go live" (SCOPE.md "Catch-up cursor").
pub type Since = Option<EventCursor>;

/// What `subscribe` returns. `Send` so transports can move it across
/// task boundaries (axum SSE handler, tauri channel, …). The stream
/// ends only when the caller drops it or the runtime shuts down.
pub type EventStream =
    Pin<Box<dyn Stream<Item = Result<EventEnvelope, crate::RpcError>> + Send + 'static>>;
