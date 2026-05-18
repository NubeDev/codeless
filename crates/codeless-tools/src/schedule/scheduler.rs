//! In-memory scheduler that fires registered `Action`s when their
//! `Schedule` next matches.
//!
//! Storage and tasks are in-process. Persistence is the host's
//! responsibility: mirror `create` / `cancel` into a durable store
//! and re-hydrate on restart. Keeping persistence out of this crate
//! lets the same scheduler back the LLM tool surface, an in-runtime
//! cron, and tests with `MemoryStore`-style harnesses.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::spec::Schedule;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScheduleId(pub String);

impl ScheduleId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Callback invoked when a schedule fires.
///
/// `Action` instead of a bare closure because the same handler may
/// run many times (recurring schedules) and across many ids;
/// boxing once at registration is cheaper than reboxing per fire.
#[async_trait]
pub trait Action: Send + Sync + 'static {
    async fn fire(&self, id: &ScheduleId, payload: &Value);
}

pub type ActionFn = Arc<
    dyn Fn(ScheduleId, Value) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
>;

#[async_trait]
impl Action for ActionFn {
    async fn fire(&self, id: &ScheduleId, payload: &Value) {
        (self)(id.clone(), payload.clone()).await;
    }
}

struct Entry {
    schedule: Schedule,
    cancel: CancellationToken,
}

pub struct Scheduler {
    entries: Arc<Mutex<HashMap<ScheduleId, Entry>>>,
    action: Arc<dyn Action>,
}

impl Scheduler {
    pub fn new(action: Arc<dyn Action>) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            action,
        }
    }

    /// Register a schedule. If an entry already exists under `id`,
    /// the previous one is cancelled and replaced — matches the
    /// expected behaviour for "update this schedule" without forcing
    /// callers to delete-then-create.
    pub async fn create(
        &self,
        id: ScheduleId,
        schedule: Schedule,
        payload: Value,
    ) -> Result<(), SchedulerError> {
        let now = Utc::now();
        if schedule.next_fire_after(now).is_none() {
            return Err(SchedulerError::NoFutureFire);
        }
        let cancel = CancellationToken::new();
        let entry = Entry {
            schedule: schedule.clone(),
            cancel: cancel.clone(),
        };

        {
            let mut guard = self.entries.lock().await;
            if let Some(prev) = guard.insert(id.clone(), entry) {
                prev.cancel.cancel();
            }
        }

        self.spawn_task(id, schedule, payload, cancel);
        Ok(())
    }

    pub async fn cancel(&self, id: &ScheduleId) -> bool {
        if let Some(entry) = self.entries.lock().await.remove(id) {
            entry.cancel.cancel();
            true
        } else {
            false
        }
    }

    pub async fn list(&self) -> Vec<(ScheduleId, Schedule)> {
        self.entries
            .lock()
            .await
            .iter()
            .map(|(id, e)| (id.clone(), e.schedule.clone()))
            .collect()
    }

    fn spawn_task(
        &self,
        id: ScheduleId,
        schedule: Schedule,
        payload: Value,
        cancel: CancellationToken,
    ) {
        let entries = Arc::clone(&self.entries);
        let action = Arc::clone(&self.action);
        let is_one_shot = matches!(schedule, Schedule::OneShot { .. });

        tokio::spawn(async move {
            loop {
                let now = Utc::now();
                let Some(next) = schedule.next_fire_after(now) else {
                    entries.lock().await.remove(&id);
                    return;
                };

                let delta = (next - now).to_std().unwrap_or_default();

                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(delta) => {}
                }

                // Tokio sleep granularity can wake us a hair early;
                // re-check the clock so we never fire before `next`.
                let now_after = Utc::now();
                if now_after < next {
                    let extra = (next - now_after).to_std().unwrap_or_default();
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(extra) => {}
                    }
                }

                if cancel.is_cancelled() {
                    return;
                }

                let action_for_fire = Arc::clone(&action);
                let id_for_fire = id.clone();
                let payload_for_fire = payload.clone();
                tokio::spawn(async move {
                    action_for_fire.fire(&id_for_fire, &payload_for_fire).await;
                });

                if is_one_shot {
                    entries.lock().await.remove(&id);
                    return;
                }
            }
        });
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SchedulerError {
    #[error("schedule has no future fire instant")]
    NoFutureFire,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::spec::{ScheduleTz, TimeOfDay, Weekday};
    use chrono::Duration;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountAction(Arc<AtomicUsize>);

    #[async_trait]
    impl Action for CountAction {
        async fn fire(&self, _id: &ScheduleId, _payload: &Value) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn one_shot_in_near_future_fires_once() {
        let count = Arc::new(AtomicUsize::new(0));
        let sched = Scheduler::new(Arc::new(CountAction(Arc::clone(&count))));
        let at = Utc::now() + Duration::milliseconds(80);
        sched
            .create(
                ScheduleId::new("oneshot"),
                Schedule::OneShot { at },
                Value::Null,
            )
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(sched.list().await.is_empty());
    }

    #[tokio::test]
    async fn past_one_shot_rejected() {
        let sched = Scheduler::new(Arc::new(CountAction(Arc::new(AtomicUsize::new(0)))));
        let at = Utc::now() - Duration::seconds(1);
        let err = sched
            .create(
                ScheduleId::new("past"),
                Schedule::OneShot { at },
                Value::Null,
            )
            .await
            .unwrap_err();
        assert_eq!(err, SchedulerError::NoFutureFire);
    }

    #[tokio::test]
    async fn cancel_prevents_fire() {
        let count = Arc::new(AtomicUsize::new(0));
        let sched = Scheduler::new(Arc::new(CountAction(Arc::clone(&count))));
        let id = ScheduleId::new("c");
        sched
            .create(
                id.clone(),
                Schedule::OneShot {
                    at: Utc::now() + Duration::milliseconds(200),
                },
                Value::Null,
            )
            .await
            .unwrap();
        assert!(sched.cancel(&id).await);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn weekly_with_no_matching_slots_is_rejected() {
        let sched = Scheduler::new(Arc::new(CountAction(Arc::new(AtomicUsize::new(0)))));
        let err = sched
            .create(
                ScheduleId::new("empty"),
                Schedule::Weekly {
                    days: vec![Weekday::Mon],
                    times: vec![],
                    tz: ScheduleTz::Utc,
                },
                Value::Null,
            )
            .await
            .unwrap_err();
        assert_eq!(err, SchedulerError::NoFutureFire);
        let _: TimeOfDay = TimeOfDay::new(0, 0).unwrap();
    }
}
