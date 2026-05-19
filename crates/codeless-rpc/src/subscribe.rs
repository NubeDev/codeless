use std::pin::Pin;

use codeless_types::{EventCursor, EventEnvelope, JobId, RepoId};
use futures_core::Stream;
use serde::{Deserialize, Serialize};

/// Filter passed to `RpcServer::subscribe`. Variants stay coarse on
/// purpose — per-stage / per-task filtering happens client-side from
/// the same event stream so the server doesn't need to multiplex N
/// fine-grained channels.
///
/// The workspace-scoped surfaces (file explorer, jobs list, live event
/// stream) drive a `Repo { repo_id }` subscription so two browser tabs
/// pointed at two attached workspaces never cross-talk. `Library`
/// covers the cross-workspace rails — assistant threads, the
/// workspaces sidebar — that read events with no owning repo (see
/// `DOCS/EVENT-PUBLISH-AUDIT.md` for the per-call-site classification).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "scope", rename_all = "kebab-case")]
pub enum EventFilter {
    /// Every event the runtime emits. Retained only for the global
    /// event log view (and the operator-side `codeless tail`); new
    /// clients drive `Repo { repo_id }` or `Library` so the wire stays
    /// workspace-scoped end-to-end. Treat as deprecated for any
    /// per-tab subscription.
    All,
    /// Only events tagged with this job (and its stages and tasks).
    Job { job_id: JobId },
    /// Every event whose envelope (or payload) resolves to this repo.
    /// At fan-out the runtime joins `events.job_id` against
    /// `jobs.repo_id`, and additionally matches library-payload
    /// events whose `Event` body carries an explicit `repo_id`
    /// (`RepoAdded`, `RepoRemoved`, `RepoUpdated`,
    /// `WorkspaceUnhealthy`, `WorkspaceRecovered`, `JobQueued`).
    Repo { repo_id: RepoId },
    /// Events with no owning repo — library-scope payloads plus the
    /// assistant / unbound-chat surfaces whose envelope `job_id` is a
    /// synthetic id that does not resolve through `jobs`. The
    /// cross-workspace rails (assistant rail, workspaces sidebar)
    /// subscribe here so they see touches no matter which workspace
    /// is in the foreground.
    Library,
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
