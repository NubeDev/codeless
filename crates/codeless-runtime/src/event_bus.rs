use std::pin::Pin;

use codeless_rpc::{EventStream, RpcError};
use codeless_types::{Event, EventCursor, EventEnvelope, JobId, StageId, TaskId, UnixMillis};
use futures_core::Stream;
use sqlx::{Row, SqlitePool};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

/// Two-stage event publication: every `publish` writes a row to the
/// `events` table first (which assigns the monotonic cursor via
/// `INTEGER PRIMARY KEY AUTOINCREMENT`), then broadcasts the
/// resulting envelope to live subscribers. Persisting first is what
/// lets `subscribe(since)` replay the gap between a client's last
/// seen cursor and the current live tail without dropping events at
/// the boundary.
///
/// `broadcast` remains the live fan-out primitive: many short-lived
/// subscribers, a single writer, slow subscribers are allowed to lag
/// (we surface lag as a stream error rather than back-pressuring the
/// publisher — a single slow client cannot stall the entire runtime).
pub struct EventBus {
    pool: SqlitePool,
    sender: broadcast::Sender<EventEnvelope>,
}

impl EventBus {
    /// Capacity is the lag tolerance per subscriber — events older
    /// than `capacity` since the subscriber's last poll are dropped
    /// from the broadcast tail and the subscriber surfaces
    /// `RpcError::Internal("event lag: …")`. The persisted log in
    /// SQLite is the recovery path: a lagged subscriber re-subscribes
    /// with the last cursor it did see and the replay catches it
    /// back up.
    pub fn new(pool: SqlitePool, capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender, pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Persist + broadcast in one shot. The returned cursor is the
    /// authoritative monotonic position — subscribers can hand it
    /// back as `since` later and the replay will resume from
    /// `cursor + 1`. A failed insert is propagated; the broadcast is
    /// not attempted.
    pub async fn publish(
        &self,
        job_id: Option<JobId>,
        stage_id: Option<StageId>,
        task_id: Option<TaskId>,
        event: Event,
        now: UnixMillis,
    ) -> sqlx::Result<EventCursor> {
        let (type_label, payload) = split_event_json(&event)?;
        let row = sqlx::query(
            "INSERT INTO events (job_id, stage_id, task_id, type, payload, created_at) \
             VALUES (?,?,?,?,?,?) RETURNING cursor",
        )
        .bind(job_id.map(|id| id.to_string()))
        .bind(stage_id.map(|id| id.to_string()))
        .bind(task_id.map(|id| id.to_string()))
        .bind(&type_label)
        .bind(&payload)
        .bind(now.0)
        .fetch_one(&self.pool)
        .await?;
        let cursor = EventCursor(row.try_get::<i64, _>("cursor")?);

        let env = EventEnvelope {
            cursor,
            job_id,
            stage_id,
            task_id,
            created_at: now,
            event,
        };
        // No live subscribers is a normal state, not an error.
        let _ = self.sender.send(env);
        Ok(cursor)
    }

    /// `filter` is applied in the returned stream; the broadcast
    /// channel itself is unfiltered. With single-tenant traffic the
    /// simplicity wins over per-topic channels.
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
    pub(crate) fn matches(&self, env: &EventEnvelope) -> bool {
        match self {
            Self::All => true,
            Self::Job(target) => env.job_id == Some(*target),
        }
    }
}

/// Split an `Event` into the `(type_label, payload_json)` tuple that
/// matches the column shape of the `events` table — `type` carries
/// the wire-stable kebab-case discriminator, `payload` carries the
/// rest of the variant fields. Going through `serde_json::Value`
/// rather than a per-variant match keeps this one function the only
/// place that needs to know about the `#[serde(tag = "type")]`
/// representation of `Event`.
fn split_event_json(event: &Event) -> sqlx::Result<(String, String)> {
    let mut value = serde_json::to_value(event).map_err(serde_err)?;
    let type_label = value
        .as_object_mut()
        .and_then(|o| o.remove("type"))
        .and_then(|t| match t {
            serde_json::Value::String(s) => Some(s),
            _ => None,
        })
        .ok_or_else(|| sqlx::Error::Decode("event missing type discriminator".into()))?;
    let payload = serde_json::to_string(&value).map_err(serde_err)?;
    Ok((type_label, payload))
}

fn serde_err(e: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Decode(format!("json: {e}").into())
}
