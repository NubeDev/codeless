use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};

use codeless_rpc::{EventStream, RpcError};
use codeless_types::{Event, EventCursor, EventEnvelope, JobId, StageId, TaskId, UnixMillis};
use futures_core::Stream;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

/// In-memory event broadcaster. Each `publish` allocates the next
/// monotonic cursor, wraps the `Event` in an `EventEnvelope`, and fans it
/// out to every live subscriber. A persistent backing store (SQLite,
/// per SCOPE.md "Catch-up cursor") will sit alongside this bus once
/// stage 4 lands the migration.
///
/// `broadcast` is the right primitive here: many short-lived
/// subscribers, a single writer, slow subscribers are allowed to lag
/// (we surface lag as a stream error rather than back-pressuring the
/// publisher — a single slow client cannot stall the entire runtime).
pub struct EventBus {
    sender: broadcast::Sender<EventEnvelope>,
    next_cursor: AtomicI64,
}

impl EventBus {
    /// Capacity is the lag tolerance per subscriber — events older than
    /// `capacity` since the subscriber's last poll are dropped and the
    /// subscriber surfaces `RpcError::Internal("event lag: …")`. 1024
    /// is a starting point chosen to absorb a verify-output spike
    /// without forcing every test to drain immediately.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            next_cursor: AtomicI64::new(1),
        }
    }

    pub fn publish(
        &self,
        job_id: Option<JobId>,
        stage_id: Option<StageId>,
        task_id: Option<TaskId>,
        event: Event,
        now: UnixMillis,
    ) -> EventCursor {
        let cursor = EventCursor(self.next_cursor.fetch_add(1, Ordering::Relaxed));
        let env = EventEnvelope {
            cursor,
            job_id,
            stage_id,
            task_id,
            created_at: now,
            event,
        };
        let _ = self.sender.send(env);
        cursor
    }

    /// `filter` is applied in the returned stream; the broadcast channel
    /// itself is unfiltered. With single-tenant traffic the simplicity
    /// wins over per-topic channels.
    pub fn subscribe(&self, filter: SubscribeFilter) -> EventStream {
        let rx = self.sender.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(move |item| match item {
            Ok(env) if filter.matches(&env) => Some(Ok(env)),
            Ok(_) => None,
            Err(e) => Some(Err(RpcError::Internal(format!("event lag: {e}")))),
        });
        let boxed: Pin<Box<dyn Stream<Item = Result<EventEnvelope, RpcError>> + Send>> =
            Box::pin(stream);
        boxed
    }
}

/// Server-side counterpart to `codeless_rpc::EventFilter`. The wire
/// filter lives in `codeless-rpc` (iOS/Android-safe); this is the
/// runtime's local match closure.
#[derive(Debug, Clone, Copy)]
pub enum SubscribeFilter {
    All,
    Job(JobId),
}

impl SubscribeFilter {
    fn matches(&self, env: &EventEnvelope) -> bool {
        match self {
            Self::All => true,
            Self::Job(target) => env.job_id == Some(*target),
        }
    }
}
