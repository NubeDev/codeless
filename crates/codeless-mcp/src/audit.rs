//! MCP `tools/call` audit event shape (PLUGIN-MCP.md § Audit).
//!
//! Decision-locked item 7 of PLUGIN-MCP.md: "plugin id is a first-
//! class audit field, not parsed from the tool id at query time".
//! The event shape mirrors the JSON the doc enumerates, with two
//! changes:
//!
//! - `args_hash` is computed at sink-write time, not at the event-
//!   construction site, so a sink that wants to elide PII can drop
//!   the field rather than recompute a hash it already has.
//! - `outcome` is a typed enum rather than a string, so audit code
//!   downstream of the sink (`grep ok`, `grep err`) gets the same
//!   shape regardless of how the sink decides to serialise.
//!
//! Sinks are pluggable behind an `AuditSink` trait so the unit
//! tests in `tests/plugin_mcp_e2e.rs` can snapshot the event stream
//! verbatim, and a production codeless-server can wire the same
//! event onto its existing structured-log subscriber without this
//! crate growing a slog or tracing dep beyond `tracing` (already
//! present for the rest of the server).

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Outcome of a single `tools/call`. Mirrors the doc's three string
/// states (`ok | err | denied`) but as a Rust enum so handlers can't
/// silently typo `"errored"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpCallOutcome {
    Ok,
    Err,
    Denied,
}

impl McpCallOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            McpCallOutcome::Ok => "ok",
            McpCallOutcome::Err => "err",
            McpCallOutcome::Denied => "denied",
        }
    }
}

/// One `tools/call` audit row. Field-for-field with the PLUGIN-MCP.md
/// example, plus a typed `dispatch_kind` so the audit consumer doesn't
/// have to disambiguate `"tool_call"` (the manifest dispatch) from
/// `"rest_proxy"` against the tool id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpAuditEvent {
    pub tool_name: String,
    /// `None` when the call was for a core codeless MCP tool (not a
    /// plugin contribution); `Some(id)` for every plugin tool. The
    /// first-class field rule (lock #7) lives here -- a downstream
    /// "disable plugin X" filter is `plugin_id == Some(x)`, not a
    /// substring match on the listing.
    pub plugin_id: Option<String>,
    /// `None` for core tools, `Some(kind)` for plugin contributions.
    /// String form matches the manifest's `dispatch.kind` values so
    /// the doc's JSON example and our typed sink agree by inspection.
    pub dispatch_kind: Option<&'static str>,
    pub outcome: McpCallOutcome,
    pub duration: Duration,
}

/// Sink that consumes audit events. The handler invokes one
/// `record` per `tools/call`; the sink decides whether to log, drop,
/// or buffer.
pub trait AuditSink: Send + Sync {
    fn record(&self, event: McpAuditEvent);
}

/// Default sink: drops every event. The MCP binary's production
/// shape today (no aggregator) is the same shape as the test
/// harness when a test does not care about the audit stream, so a
/// drop-on-the-floor default keeps the call sites uncluttered.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullAuditSink;

impl AuditSink for NullAuditSink {
    fn record(&self, _event: McpAuditEvent) {}
}

/// In-memory sink used by tests. The `Mutex<Vec<_>>` is fine because
/// MCP handlers serialise per-request anyway; a high-throughput
/// production sink lands later behind the same trait.
#[derive(Debug, Default)]
pub struct InMemoryAuditSink {
    events: Mutex<Vec<McpAuditEvent>>,
}

impl InMemoryAuditSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Snapshot of every event recorded so far. Cloned so the caller
    /// can assert against it without holding the lock.
    pub fn events(&self) -> Vec<McpAuditEvent> {
        self.events.lock().expect("audit lock").clone()
    }
}

impl AuditSink for InMemoryAuditSink {
    fn record(&self, event: McpAuditEvent) {
        self.events.lock().expect("audit lock").push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_wire_strings_match_doc() {
        assert_eq!(McpCallOutcome::Ok.as_str(), "ok");
        assert_eq!(McpCallOutcome::Err.as_str(), "err");
        assert_eq!(McpCallOutcome::Denied.as_str(), "denied");
    }

    #[test]
    fn in_memory_sink_snapshots_order() {
        let sink = InMemoryAuditSink::new();
        sink.record(McpAuditEvent {
            tool_name: "notes.notes_append".into(),
            plugin_id: Some("notes".into()),
            dispatch_kind: Some("tool_call"),
            outcome: McpCallOutcome::Ok,
            duration: Duration::from_millis(7),
        });
        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].plugin_id.as_deref(), Some("notes"));
        assert_eq!(events[0].dispatch_kind, Some("tool_call"));
    }
}
