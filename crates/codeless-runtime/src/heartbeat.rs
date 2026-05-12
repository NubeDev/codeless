use std::sync::Arc;
use std::time::Duration;

use codeless_types::{TaskId, UnixMillis};
use tokio::task::JoinHandle;

use crate::store::SqliteStore;
use crate::time::now_ms;

/// Spawn a background task that renews a task's lease every `period`
/// until the lease is lost (someone else became the holder) or the
/// returned handle is aborted.
///
/// `ttl` must be larger than `period`; the loop sets each renewed
/// expiry to `now + ttl` so a single missed tick still leaves the
/// reaper a window to step in. `period_ms` < `ttl_ms / 2` is the
/// usual safety margin.
///
/// The task swallows transient DB errors with a tracing event rather
/// than panicking — a heartbeat that briefly fails will retry on the
/// next tick; only "lease lost" (the CAS update affected zero rows)
/// is treated as terminal.
pub fn spawn_heartbeat(
    store: Arc<SqliteStore>,
    task_id: TaskId,
    holder: String,
    period: Duration,
    ttl: Duration,
) -> JoinHandle<()> {
    let ttl_ms = ttl.as_millis().min(i64::MAX as u128) as i64;
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(period).await;
            let new_expires = UnixMillis(now_ms().0.saturating_add(ttl_ms));
            match store.heartbeat_task(task_id, &holder, new_expires).await {
                Ok(true) => continue,
                Ok(false) => {
                    tracing::info!(%task_id, holder, "heartbeat lost lease, exiting");
                    return;
                }
                Err(e) => {
                    tracing::warn!(error = %e, %task_id, "heartbeat db error; retrying");
                }
            }
        }
    })
}
