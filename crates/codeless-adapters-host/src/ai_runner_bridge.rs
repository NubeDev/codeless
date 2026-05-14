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

use codeless_types::{CostCents, Event, TaskId};
use tokio::sync::mpsc;

/// Convert a single upstream event to its codeless equivalent.
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
pub fn map_event(ev: ai_runner::Event, task_id: TaskId) -> Option<Event> {
    match ev.kind {
        ai_runner::EventKind::Text { content } => Some(Event::AiToken {
            task_id,
            delta: content,
        }),
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
pub async fn forward_events<F, Fut, E>(
    mut rx: mpsc::Receiver<ai_runner::Event>,
    task_id: TaskId,
    mut publish: F,
) -> Result<(), E>
where
    F: FnMut(Event) -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
{
    while let Some(ev) = rx.recv().await {
        if let Some(mapped) = map_event(ev, task_id) {
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
        assert!(
            map_event(
                ai_event(EventKind::Error {
                    message: "boom".into()
                }),
                task_id,
            )
            .is_none()
        );
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
}
