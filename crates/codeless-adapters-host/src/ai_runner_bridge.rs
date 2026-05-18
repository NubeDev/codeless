//! Translation layer between the upstream `ai-runner` crate and the
//! codeless event model.
//!
//! `ai-runner` was designed before codeless existed; its [`Event`]
//! shape groups every provider's stream onto a small flat enum keyed
//! by `SessionId`. Codeless events are envelope-and-payload — the
//! envelope carries `(job_id, stage_id, task_id)` so subscribers can
//! filter, and the payload tracks state machine transitions plus AI
//! telemetry. The bridge is the only place that knows both shapes.
//!
//! Direction is one-way: `ai_runner::Event` in, `codeless_types::Event`
//! out, with a caller-supplied publish closure handling the envelope
//! side (timestamp, job/stage IDs, the actual `EventBus` write). The
//! closure is what keeps this crate free of a dependency on
//! `codeless-runtime` — adapters-host stays a leaf in the codeless
//! crate graph even though the runtime is what wires the runner up.
//!
//! A small piece of state lives inside `forward_events`: the
//! [`TodoWriteTracker`]. Claude Code's `TodoWrite` tool call carries
//! the full todo list on every invocation; the tracker diffs the
//! current payload against the prior snapshot so the bridge can emit
//! `TodoAdded` for genuinely new items and `TodoUpdated` /
//! `TodoCompleted` for status flips — instead of re-announcing every
//! row each call. State scope is one forwarder = one runner task, so
//! the tracker lives on the stack of `forward_events` and dies with
//! the run.

use codeless_types::{CostCents, Event, TaskId, TodoId, TodoKind, TodoStatus};
use tokio::sync::mpsc;

/// Tool name Claude Code uses for its built-in plan tool. Matched
/// case-sensitively because the upstream wire is case-stable, and
/// other CLI runners ship different tool names — codex / copilot
/// plumbing slots in here as a new tool-name constant later (out of
/// scope for the first cut; the trio still fires regardless, so an
/// unrecognised tool call simply falls through to the generic
/// `ToolCall` event).
pub const CLAUDE_TODO_WRITE_TOOL: &str = "TodoWrite";

/// Cap for a single todo row's display title. Mirrors the cap the
/// trio emitter uses in `template_runner` so user-visible rows render
/// on one line regardless of how verbose the runner's todo content is.
/// WORKFLOW.md anti-pattern: "Do not store todo titles in the event
/// payload longer than ~200 chars."
const MAX_TODO_TITLE_CHARS: usize = 200;

/// Per-run diff state for Claude Code's `TodoWrite` tool. Holds the
/// most recently observed snapshot of the runner's plan list, keyed
/// by position within the array (Claude Code's contract is that the
/// list is positionally stable — new items append, status flips
/// in-place). One tracker per forwarder; the data dies with the run.
#[derive(Debug, Default)]
pub struct TodoWriteTracker {
    entries: Vec<TrackedTodo>,
}

#[derive(Debug, Clone)]
struct TrackedTodo {
    todo_id: TodoId,
    content: String,
    status: TodoStatus,
}

impl TodoWriteTracker {
    /// Empty tracker — no `TodoWrite` calls have been observed yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many positions are currently tracked. Exposed for tests
    /// and for callers that want to log the running width without
    /// poking at private fields.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Convert a single upstream event to its codeless equivalent. This is
/// the no-state path used for variants that are pure 1:1 mappings —
/// most events. The `TodoWrite` tool call is intentionally absent here
/// and routed through [`map_event_with_state`] instead, because the
/// diff-against-prior-snapshot logic is stateful.
///
/// Returns `None` for variants that have no useful counterpart on the
/// codeless wire today:
///
/// - `Connected` is informational scaffolding for ai-runner CLIs that
///   want to surface "child process started"; codeless treats the job
///   lifecycle as authoritative for that signal.
/// - `Error` from an upstream run is converted by `drive_job` into a
///   `JobFailed` (or per-task `TaskCompleted { status: Failed }`)
///   transition, so dropping it here avoids a duplicate failure event.
///   A `tracing::warn!` keeps the upstream message visible in logs.
/// - A `ToolUse` whose name is [`CLAUDE_TODO_WRITE_TOOL`]. We swallow
///   it here so callers that bypass the stateful path do not surface
///   a generic `ToolCall { tool: "TodoWrite" }` to the UI alongside
///   the structured todo events the stateful path emits — the two
///   would be redundant and a `TodoWrite` rendered as a tool call is
///   noise the user has already seen as a checklist update.
pub fn map_event(ev: ai_runner::Event, task_id: TaskId) -> Option<Event> {
    match ev.kind {
        ai_runner::EventKind::Text { content } => Some(Event::AiToken {
            task_id,
            delta: content,
        }),
        ai_runner::EventKind::ToolUse { name, .. } if name == CLAUDE_TODO_WRITE_TOOL => None,
        ai_runner::EventKind::ToolUse { name, input, .. } => Some(Event::ToolCall {
            task_id,
            tool: name,
            args_json: input
                .as_ref()
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                .unwrap_or_default(),
        }),
        ai_runner::EventKind::Done {
            duration_ms: _,
            cost_usd,
            input_tokens,
            output_tokens,
        } => Some(Event::AiMessageComplete {
            task_id,
            input_tokens: i64::from(input_tokens),
            output_tokens: i64::from(output_tokens),
            cost_cents: usd_to_cents(cost_usd),
        }),
        ai_runner::EventKind::Error { message } => {
            tracing::warn!(error = %message, "ai-runner reported run-level error; relying on driver to surface");
            None
        }
        ai_runner::EventKind::Connected { .. } => None,
    }
}

/// Stateful translation. A `TodoWrite` tool call diffs into one or
/// more todo events via [`map_todo_write`]; every other upstream event
/// falls through to [`map_event`] and produces at most one codeless
/// event. Used by [`forward_events`] — direct callers exist primarily
/// for unit tests that want to drive the diff explicitly.
pub fn map_event_with_state(
    ev: ai_runner::Event,
    task_id: TaskId,
    tracker: &mut TodoWriteTracker,
) -> Vec<Event> {
    if let ai_runner::EventKind::ToolUse { name, input, .. } = &ev.kind {
        if name == CLAUDE_TODO_WRITE_TOOL {
            return match input {
                Some(payload) => map_todo_write(payload, task_id, tracker),
                None => {
                    tracing::trace!("TodoWrite ToolUse arrived without `input`; dropping");
                    Vec::new()
                }
            };
        }
    }
    map_event(ev, task_id).into_iter().collect()
}

/// Diff a Claude Code `TodoWrite` payload against `tracker` and return
/// the events that describe the delta. The payload shape is
/// `{ "todos": [ { "content": "...", "status": "pending|in_progress|completed", "activeForm": "..." }, ... ] }`
/// — Claude Code rewrites the entire list on every call, so the diff
/// is what gives us the cheap "only changes" wire shape.
///
/// Identity rule: entries are keyed by position. Claude's contract is
/// to keep positions stable across calls (new items append, status
/// flips in place); a misbehaving call that shuffles content under
/// existing positions surfaces as a stale-title row, not a duplicate
/// `TodoAdded`. The alternative — keying by content — would multiply
/// rows every time the model rewrote a description.
///
/// All emitted todos use [`TodoKind::Runner`]. The closing trio
/// (`Checks` / `Docs` / `Git`) is runtime-injected at stage entry
/// (`template_runner::publish_trio`) and lives in a non-overlapping
/// ordinal range (`u32::MAX - 2 ..= u32::MAX`); runner-authored todos
/// start at `0` and never collide.
///
/// Returns an empty `Vec` when `payload.todos` is missing or not an
/// array — a malformed call is logged at `warn!` rather than panicking.
pub fn map_todo_write(
    payload: &serde_json::Value,
    task_id: TaskId,
    tracker: &mut TodoWriteTracker,
) -> Vec<Event> {
    let Some(items) = payload.get("todos").and_then(|v| v.as_array()) else {
        tracing::warn!("TodoWrite payload missing `todos` array; runner schema may have changed");
        return Vec::new();
    };
    let mut events = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let content = item
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let status_raw = item
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("pending");
        let Some(status) = parse_todo_status(status_raw) else {
            tracing::trace!(
                ?status_raw,
                "TodoWrite entry had unrecognised status; skipping"
            );
            continue;
        };

        if let Some(existing) = tracker.entries.get_mut(idx) {
            let prior = existing.status;
            existing.content = content;
            if prior != status {
                existing.status = status;
                events.push(status_flip_event(existing.todo_id, prior, status));
            }
        } else {
            let todo_id = TodoId::new();
            tracker.entries.push(TrackedTodo {
                todo_id,
                content: content.clone(),
                status,
            });
            events.push(Event::TodoAdded {
                todo_id,
                task_id,
                ordinal: idx as u32,
                title: truncate_title(&content),
                kind: TodoKind::Runner,
            });
            // A row whose initial status is non-pending needs an
            // immediate flip so subscribers see the right glyph
            // without inferring it from the `TodoAdded` payload
            // alone — the recorder writes the row as `Pending` and
            // relies on a subsequent `TodoUpdated` / `TodoCompleted`
            // to advance it. Pending entries are already in the
            // right state, so we skip the flip there.
            if status != TodoStatus::Pending {
                events.push(status_flip_event(todo_id, TodoStatus::Pending, status));
            }
        }
    }
    events
}

fn parse_todo_status(raw: &str) -> Option<TodoStatus> {
    // Claude Code's vocabulary is `pending | in_progress | completed`.
    // Map onto codeless's larger enum: `Skipped` and `Failed` are
    // runtime-side states (Git-trio no-diff case, verify failure) and
    // never appear in a runner's `TodoWrite` plan.
    match raw {
        "pending" => Some(TodoStatus::Pending),
        "in_progress" => Some(TodoStatus::InProgress),
        "completed" => Some(TodoStatus::Done),
        _ => None,
    }
}

fn status_flip_event(todo_id: TodoId, prior: TodoStatus, next: TodoStatus) -> Event {
    let _ = prior;
    if is_terminal(next) {
        Event::TodoCompleted {
            todo_id,
            status: next,
            failure_detail: None,
        }
    } else {
        Event::TodoUpdated {
            todo_id,
            status: next,
        }
    }
}

fn is_terminal(s: TodoStatus) -> bool {
    matches!(
        s,
        TodoStatus::Done | TodoStatus::Skipped | TodoStatus::Failed
    )
}

/// Trim a runner-supplied todo content string down to the UI's
/// single-line cap. Mirrors `template_runner::truncate_title` so the
/// runner path and the runtime path agree on the length budget.
fn truncate_title(s: &str) -> String {
    if s.chars().count() <= MAX_TODO_TITLE_CHARS {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(MAX_TODO_TITLE_CHARS - 1).collect();
        out.push('…');
        out
    }
}

/// Drain `rx` of upstream events and forward each translated event to
/// `publish`. The function returns `Ok(())` when the channel closes
/// cleanly (the runner finished and dropped its sender) and propagates
/// the first `Err(_)` from `publish` otherwise.
///
/// The publish closure owns the codeless side — typically it captures
/// an `Arc<EventBus>` plus the envelope `(job_id, stage_id, task_id)`
/// for this run and calls `bus.publish(...)`. Keeping the bus out of
/// this module is what holds the runtime → adapters-host edge in the
/// dependency graph; the alternative would force adapters-host to
/// depend on codeless-runtime and pull process-spawning into the
/// runtime's transitive deps.
///
/// A `TodoWriteTracker` is held on this future's stack: the runner's
/// `TodoWrite` calls diff against the prior snapshot so the bridge
/// emits only the deltas. State scope matches forwarder scope —
/// state dies with the run, no cross-run leakage.
pub async fn forward_events<F, Fut, E>(
    mut rx: mpsc::Receiver<ai_runner::Event>,
    task_id: TaskId,
    mut publish: F,
) -> Result<(), E>
where
    F: FnMut(Event) -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
{
    let mut tracker = TodoWriteTracker::new();
    while let Some(ev) = rx.recv().await {
        for mapped in map_event_with_state(ev, task_id, &mut tracker) {
            publish(mapped).await?;
        }
    }
    Ok(())
}

/// Round a USD float into integer cents. Inputs come from provider
/// usage reports which already round to a fixed number of decimal
/// places; the bridge clamps negatives to zero rather than silently
/// flipping a sign, since a negative usage value would be a bug
/// upstream rather than a meaningful refund.
fn usd_to_cents(usd: f64) -> CostCents {
    if !usd.is_finite() || usd <= 0.0 {
        return CostCents::ZERO;
    }
    CostCents((usd * 100.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_runner::{Event as AiEvent, EventKind, SessionId};
    use codeless_types::id::TaskId;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn ai_event(kind: EventKind) -> AiEvent {
        AiEvent {
            session_id: SessionId::from("sess"),
            provider: "claude".into(),
            kind,
        }
    }

    fn todo_write_event(payload: serde_json::Value) -> AiEvent {
        ai_event(EventKind::ToolUse {
            id: Some("tu_1".into()),
            name: CLAUDE_TODO_WRITE_TOOL.into(),
            input: Some(payload),
        })
    }

    #[test]
    fn text_maps_to_ai_token() {
        let task_id = TaskId::new();
        let mapped = map_event(
            ai_event(EventKind::Text {
                content: "hello".into(),
            }),
            task_id,
        );
        assert!(matches!(
            mapped,
            Some(Event::AiToken { task_id: t, ref delta }) if t == task_id && delta == "hello"
        ));
    }

    #[test]
    fn tool_use_serializes_input() {
        let task_id = TaskId::new();
        let mapped = map_event(
            ai_event(EventKind::ToolUse {
                id: Some("tu_1".into()),
                name: "edit_file".into(),
                input: Some(json!({"path": "src/main.rs"})),
            }),
            task_id,
        );
        match mapped {
            Some(Event::ToolCall {
                task_id: t,
                tool,
                args_json,
            }) => {
                assert_eq!(t, task_id);
                assert_eq!(tool, "edit_file");
                let v: serde_json::Value = serde_json::from_str(&args_json).unwrap();
                assert_eq!(v, json!({"path": "src/main.rs"}));
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn done_carries_cost_in_cents() {
        let task_id = TaskId::new();
        let mapped = map_event(
            ai_event(EventKind::Done {
                duration_ms: 12,
                cost_usd: 0.0345,
                input_tokens: 100,
                output_tokens: 250,
            }),
            task_id,
        );
        match mapped {
            Some(Event::AiMessageComplete {
                task_id: t,
                input_tokens,
                output_tokens,
                cost_cents,
            }) => {
                assert_eq!(t, task_id);
                assert_eq!(input_tokens, 100);
                assert_eq!(output_tokens, 250);
                assert_eq!(cost_cents, CostCents(3));
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn connected_and_error_drop() {
        let task_id = TaskId::new();
        assert!(map_event(ai_event(EventKind::Connected { model: None }), task_id).is_none());
        assert!(map_event(
            ai_event(EventKind::Error {
                message: "boom".into()
            }),
            task_id,
        )
        .is_none());
    }

    #[test]
    fn todo_write_tool_use_is_suppressed_in_stateless_path() {
        // `map_event` must not surface a generic `ToolCall { tool: "TodoWrite" }`
        // alongside the structured todo events the stateful path emits —
        // otherwise every TodoWrite invocation would appear twice on the
        // UI (once as a tool call, once as a row update).
        let task_id = TaskId::new();
        let mapped = map_event(
            todo_write_event(json!({"todos": [{"content": "x", "status": "pending"}]})),
            task_id,
        );
        assert!(mapped.is_none(), "TodoWrite must not produce a ToolCall");
    }

    #[test]
    fn first_todo_write_emits_added_per_item() {
        let task_id = TaskId::new();
        let mut tracker = TodoWriteTracker::new();
        let events = map_event_with_state(
            todo_write_event(json!({
                "todos": [
                    {"content": "scan repo", "status": "pending", "activeForm": "Scanning repo"},
                    {"content": "draft fix", "status": "pending", "activeForm": "Drafting fix"},
                ]
            })),
            task_id,
            &mut tracker,
        );
        assert_eq!(events.len(), 2);
        match &events[0] {
            Event::TodoAdded {
                task_id: t,
                ordinal,
                title,
                kind,
                ..
            } => {
                assert_eq!(*t, task_id);
                assert_eq!(*ordinal, 0);
                assert_eq!(title, "scan repo");
                assert_eq!(*kind, TodoKind::Runner);
            }
            other => panic!("expected TodoAdded, got {other:?}"),
        }
        match &events[1] {
            Event::TodoAdded { ordinal, title, .. } => {
                assert_eq!(*ordinal, 1);
                assert_eq!(title, "draft fix");
            }
            other => panic!("expected TodoAdded, got {other:?}"),
        }
        assert_eq!(tracker.len(), 2);
    }

    #[test]
    fn status_flip_to_in_progress_emits_updated() {
        let task_id = TaskId::new();
        let mut tracker = TodoWriteTracker::new();
        // First call: register the row.
        let _ = map_event_with_state(
            todo_write_event(json!({
                "todos": [{"content": "do thing", "status": "pending"}]
            })),
            task_id,
            &mut tracker,
        );
        // Second call: same content, status flipped to in_progress.
        let events = map_event_with_state(
            todo_write_event(json!({
                "todos": [{"content": "do thing", "status": "in_progress"}]
            })),
            task_id,
            &mut tracker,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::TodoUpdated { status, .. } => assert_eq!(*status, TodoStatus::InProgress),
            other => panic!("expected TodoUpdated, got {other:?}"),
        }
    }

    #[test]
    fn status_flip_to_completed_emits_todo_completed() {
        let task_id = TaskId::new();
        let mut tracker = TodoWriteTracker::new();
        let _ = map_event_with_state(
            todo_write_event(json!({
                "todos": [{"content": "do thing", "status": "in_progress"}]
            })),
            task_id,
            &mut tracker,
        );
        // The initial in_progress entry emits a TodoAdded + a follow-up
        // TodoUpdated; clear the buffer by ignoring those, then drive
        // the terminal transition.
        let events = map_event_with_state(
            todo_write_event(json!({
                "todos": [{"content": "do thing", "status": "completed"}]
            })),
            task_id,
            &mut tracker,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::TodoCompleted { status, .. } => assert_eq!(*status, TodoStatus::Done),
            other => panic!("expected TodoCompleted, got {other:?}"),
        }
    }

    #[test]
    fn appended_items_get_new_todo_added() {
        let task_id = TaskId::new();
        let mut tracker = TodoWriteTracker::new();
        let _ = map_event_with_state(
            todo_write_event(json!({
                "todos": [{"content": "a", "status": "pending"}]
            })),
            task_id,
            &mut tracker,
        );
        let events = map_event_with_state(
            todo_write_event(json!({
                "todos": [
                    {"content": "a", "status": "pending"},
                    {"content": "b", "status": "pending"},
                ]
            })),
            task_id,
            &mut tracker,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::TodoAdded { ordinal, title, .. } => {
                assert_eq!(*ordinal, 1);
                assert_eq!(title, "b");
            }
            other => panic!("expected TodoAdded, got {other:?}"),
        }
        assert_eq!(tracker.len(), 2);
    }

    #[test]
    fn non_pending_initial_status_emits_added_then_flip() {
        // A row whose first observation is already `in_progress` needs
        // both `TodoAdded` (so the row exists in the store) and a
        // `TodoUpdated` (so the recorder advances it past `Pending`).
        let task_id = TaskId::new();
        let mut tracker = TodoWriteTracker::new();
        let events = map_event_with_state(
            todo_write_event(json!({
                "todos": [{"content": "going", "status": "in_progress"}]
            })),
            task_id,
            &mut tracker,
        );
        assert_eq!(events.len(), 2);
        let added_id = match &events[0] {
            Event::TodoAdded { todo_id, .. } => *todo_id,
            other => panic!("expected TodoAdded, got {other:?}"),
        };
        match &events[1] {
            Event::TodoUpdated { todo_id, status } => {
                assert_eq!(*todo_id, added_id);
                assert_eq!(*status, TodoStatus::InProgress);
            }
            other => panic!("expected TodoUpdated, got {other:?}"),
        }
    }

    #[test]
    fn malformed_payload_returns_no_events() {
        let task_id = TaskId::new();
        let mut tracker = TodoWriteTracker::new();
        // Missing `todos` key.
        assert!(map_todo_write(&json!({}), task_id, &mut tracker).is_empty());
        // `todos` not an array.
        assert!(map_todo_write(&json!({"todos": "nope"}), task_id, &mut tracker).is_empty());
        assert!(tracker.is_empty());
    }

    #[test]
    fn long_titles_truncate() {
        let task_id = TaskId::new();
        let mut tracker = TodoWriteTracker::new();
        let long = "x".repeat(MAX_TODO_TITLE_CHARS + 50);
        let events = map_event_with_state(
            todo_write_event(json!({
                "todos": [{"content": long, "status": "pending"}]
            })),
            task_id,
            &mut tracker,
        );
        match &events[0] {
            Event::TodoAdded { title, .. } => {
                assert_eq!(title.chars().count(), MAX_TODO_TITLE_CHARS);
                assert!(title.ends_with('…'));
            }
            other => panic!("expected TodoAdded, got {other:?}"),
        }
    }

    #[test]
    fn no_change_emits_nothing() {
        let task_id = TaskId::new();
        let mut tracker = TodoWriteTracker::new();
        let _ = map_event_with_state(
            todo_write_event(json!({
                "todos": [{"content": "a", "status": "pending"}]
            })),
            task_id,
            &mut tracker,
        );
        let events = map_event_with_state(
            todo_write_event(json!({
                "todos": [{"content": "a", "status": "pending"}]
            })),
            task_id,
            &mut tracker,
        );
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn forward_collects_in_order() {
        let task_id = TaskId::new();
        let (tx, rx) = mpsc::channel(16);

        tx.send(ai_event(EventKind::Text {
            content: "a".into(),
        }))
        .await
        .unwrap();
        tx.send(ai_event(EventKind::Connected { model: None }))
            .await
            .unwrap();
        tx.send(ai_event(EventKind::Text {
            content: "b".into(),
        }))
        .await
        .unwrap();
        tx.send(ai_event(EventKind::Done {
            duration_ms: 1,
            cost_usd: 0.01,
            input_tokens: 1,
            output_tokens: 2,
        }))
        .await
        .unwrap();
        drop(tx);

        let collected: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&collected);
        let result: Result<(), std::convert::Infallible> = forward_events(rx, task_id, move |ev| {
            let sink = Arc::clone(&sink);
            async move {
                sink.lock().unwrap().push(ev);
                Ok(())
            }
        })
        .await;
        result.unwrap();

        let got = collected.lock().unwrap().clone();
        assert_eq!(got.len(), 3, "Connected should be dropped");
        assert!(matches!(got[0], Event::AiToken { ref delta, .. } if delta == "a"));
        assert!(matches!(got[1], Event::AiToken { ref delta, .. } if delta == "b"));
        assert!(matches!(got[2], Event::AiMessageComplete { .. }));
    }

    #[tokio::test]
    async fn forward_diffs_todo_writes_across_calls() {
        // A two-call TodoWrite sequence through the forwarder: the
        // first call should add both rows; the second should emit
        // exactly one TodoCompleted for the row that flipped.
        let task_id = TaskId::new();
        let (tx, rx) = mpsc::channel(16);

        tx.send(todo_write_event(json!({
            "todos": [
                {"content": "first", "status": "pending"},
                {"content": "second", "status": "pending"},
            ]
        })))
        .await
        .unwrap();
        tx.send(todo_write_event(json!({
            "todos": [
                {"content": "first", "status": "completed"},
                {"content": "second", "status": "pending"},
            ]
        })))
        .await
        .unwrap();
        drop(tx);

        let collected: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&collected);
        let result: Result<(), std::convert::Infallible> = forward_events(rx, task_id, move |ev| {
            let sink = Arc::clone(&sink);
            async move {
                sink.lock().unwrap().push(ev);
                Ok(())
            }
        })
        .await;
        result.unwrap();

        let got = collected.lock().unwrap().clone();
        assert_eq!(got.len(), 3, "two adds + one completed; got {got:?}");
        assert!(matches!(got[0], Event::TodoAdded { ordinal: 0, .. }));
        assert!(matches!(got[1], Event::TodoAdded { ordinal: 1, .. }));
        match &got[2] {
            Event::TodoCompleted { status, .. } => assert_eq!(*status, TodoStatus::Done),
            other => panic!("expected TodoCompleted, got {other:?}"),
        }
    }
}
