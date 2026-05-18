//! Payload-routing `Action` for the scheduler.
//!
//! The scheduler fires a single `Action` per registered schedule, but
//! a host typically wants the same scheduler to back several
//! different behaviours: enqueue a job, post into an assistant
//! thread, hit a webhook. `PayloadDispatcher` is the join point: it
//! reads `payload["kind"]` and routes to a sub-`Action` registered
//! under that key. Unknown kinds fall through to a default handler.
//!
//! Kept in the library (not the host) so tests can construct a
//! dispatcher without pulling in the runtime, and so every host —
//! `codeless-mcp`, `codeless-runtime`, future shells — shares the
//! same routing semantics.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::scheduler::{Action, ScheduleId};

pub struct PayloadDispatcher {
    handlers: HashMap<String, Arc<dyn Action>>,
    default: Arc<dyn Action>,
}

impl PayloadDispatcher {
    pub fn new(default: Arc<dyn Action>) -> Self {
        Self {
            handlers: HashMap::new(),
            default,
        }
    }

    pub fn register(&mut self, kind: impl Into<String>, action: Arc<dyn Action>) {
        self.handlers.insert(kind.into(), action);
    }
}

#[async_trait]
impl Action for PayloadDispatcher {
    async fn fire(&self, id: &ScheduleId, payload: &Value) {
        let kind = payload.get("kind").and_then(Value::as_str);
        let handler = kind
            .and_then(|k| self.handlers.get(k))
            .unwrap_or(&self.default);
        handler.fire(id, payload).await;
    }
}

/// Action that logs the fire via `tracing::info!`. Useful as the
/// default handler in hosts that haven't wired richer behaviours
/// yet — the schedule still fires, the host can see it in logs.
pub struct LogAction;

#[async_trait]
impl Action for LogAction {
    async fn fire(&self, id: &ScheduleId, payload: &Value) {
        tracing::info!(schedule_id = %id.0, payload = %payload, "schedule fired");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Recorder {
        seen: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl Action for Recorder {
        async fn fire(&self, id: &ScheduleId, payload: &Value) {
            self.seen.lock().unwrap().push((id.0.clone(), payload.clone()));
        }
    }

    fn rec() -> Arc<Recorder> {
        Arc::new(Recorder {
            seen: Mutex::new(Vec::new()),
        })
    }

    #[tokio::test]
    async fn routes_on_payload_kind() {
        let job = rec();
        let mail = rec();
        let fallback = rec();
        let mut d = PayloadDispatcher::new(fallback.clone());
        d.register("enqueue_job", job.clone());
        d.register("send_mail", mail.clone());

        d.fire(&ScheduleId::new("a"), &serde_json::json!({"kind":"enqueue_job","job":"x"})).await;
        d.fire(&ScheduleId::new("b"), &serde_json::json!({"kind":"send_mail","to":"x"})).await;
        d.fire(&ScheduleId::new("c"), &serde_json::json!({"kind":"unknown"})).await;
        d.fire(&ScheduleId::new("d"), &serde_json::json!({})).await;

        assert_eq!(job.seen.lock().unwrap().len(), 1);
        assert_eq!(mail.seen.lock().unwrap().len(), 1);
        assert_eq!(fallback.seen.lock().unwrap().len(), 2);
    }
}
