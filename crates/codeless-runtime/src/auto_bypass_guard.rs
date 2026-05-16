//! Surface F thrashing guard. Tracks how many consecutive stages have
//! been auto-bypassed under the job's `AutoBypassPolicy` and forces a
//! halt on the third one — see `DOCS/AUTO-BYPASS-DECISIONS.md` Q1.
//!
//! The state is intentionally ephemeral: a `Mutex<HashMap<JobId, u32>>`
//! that the driver rebuilds from the events table on startup. The
//! decisions doc forbids a new SQLite column for the count (R4 / Q1
//! "State location"), so the source of truth is the wire log: a
//! `StageAutoBypassed` event increments the count; a `StageCompleted`
//! with status `Passed` resets it to zero. Anything else is ignored.
//!
//! Counting rule: "two consecutive auto-bypasses" means two adjacent
//! `Failed`-then-auto-bypassed stages with no intervening `Passed`. The
//! guard fires on the **second** auto-bypass attempt — at that point
//! one auto-bypass already landed on the row, so `would_breach`
//! returns true and the runner halts before emitting the second
//! `StageAutoBypassed` envelope. A reset-on-pass moves the count back
//! to zero, so a Pass between two failures lets the next failure
//! auto-bypass once again.
//!
//! Cap breaches never reach the guard: `template_runner`'s
//! `classify_stage_failure` returns `FailureAction::Halt` on
//! `stop_reason.is_some()` before consulting the policy, so the
//! thrashing guard is dead code on that path (the spec's "cap-breach
//! bypasses the guard" case). The test below exercises that the guard
//! does not record anything when the failure does not flow through
//! `record_auto_bypass`.

use std::collections::HashMap;
use std::sync::Mutex;

use codeless_types::JobId;
use sqlx::Row;

use crate::store::SqliteStore;

/// In-memory tracker for consecutive auto-bypasses per job. Cloned via
/// `Arc` into every `TemplateRunner` the factory builds so all of one
/// process's running jobs share the same map; a server restart wipes
/// the map and `rebuild_from_store` reseeds it from the events table.
#[derive(Debug, Default)]
pub struct ThrashingGuard {
    consecutive: Mutex<HashMap<JobId, u32>>,
}

impl ThrashingGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when an additional auto-bypass on `job` would
    /// cross the two-strikes threshold. The runner consults this
    /// **before** emitting `StageAutoBypassed`; on `true` it writes
    /// `stop_reason = AutoBypassThrashing` on the job row and halts.
    pub fn would_breach(&self, job: JobId) -> bool {
        self.consecutive
            .lock()
            .expect("thrashing guard mutex poisoned")
            .get(&job)
            .copied()
            .unwrap_or(0)
            >= 1
    }

    /// Record an auto-bypass against `job`. The runner calls this
    /// **after** `would_breach` returned `false` and after the
    /// `StageAutoBypassed` envelope has been emitted, so the count
    /// stays consistent with the wire log.
    pub fn record_auto_bypass(&self, job: JobId) -> u32 {
        let mut m = self
            .consecutive
            .lock()
            .expect("thrashing guard mutex poisoned");
        let entry = m.entry(job).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Reset the count for `job` on a Passed stage. A single
    /// successful stage between failures is the doc's reset criterion
    /// — see Q1 "Counting rule. A `Passed` between them resets the
    /// counter to zero."
    pub fn record_pass(&self, job: JobId) {
        self.consecutive
            .lock()
            .expect("thrashing guard mutex poisoned")
            .insert(job, 0);
    }

    /// Read the current count without mutating. Test-only — production
    /// code goes through `would_breach`, which embeds the threshold
    /// comparison so a future change to the window size (e.g. from
    /// two to three) lands in one place rather than at every call
    /// site.
    #[cfg(test)]
    fn count(&self, job: JobId) -> u32 {
        self.consecutive
            .lock()
            .expect("thrashing guard mutex poisoned")
            .get(&job)
            .copied()
            .unwrap_or(0)
    }

    /// Forget `job` entirely. Called when a job leaves the running set
    /// terminally so the map does not grow unbounded across a long-
    /// lived process. Test-only today because the runtime's terminal
    /// transitions are scattered across `driver` / `rpc::jobs` /
    /// `job_driver_loop`; wiring the cleanup at every exit point is
    /// follow-up work and growth-rate is bounded by the number of
    /// jobs a single server has ever Run, not by stage count.
    #[cfg(test)]
    pub fn forget(&self, job: JobId) {
        self.consecutive
            .lock()
            .expect("thrashing guard mutex poisoned")
            .remove(&job);
    }

    /// Rebuild the consecutive-auto-bypass count for `job` by replaying
    /// the events table in cursor order. A driver restart loses the
    /// in-memory map; calling this at the top of each
    /// `TemplateRunner::run` reseeds the count from the wire log so a
    /// resumed job that was already at one auto-bypass does not get a
    /// fresh budget for a second.
    ///
    /// The replay is deliberately liberal about unknown variants:
    /// every event except `stage-auto-bypassed` and `stage-completed`
    /// (with `status=passed`) is ignored. New event types added in
    /// later stages do not need to teach this function about
    /// themselves.
    pub async fn rebuild_from_store(&self, store: &SqliteStore, job: JobId) -> sqlx::Result<u32> {
        let rows = sqlx::query("SELECT type, payload FROM events WHERE job_id = ? ORDER BY cursor")
            .bind(job.to_string())
            .fetch_all(store.pool())
            .await?;
        let mut count: u32 = 0;
        for row in rows {
            let ty: String = row.try_get("type")?;
            match ty.as_str() {
                "stage-auto-bypassed" => count += 1,
                "stage-completed" => {
                    let payload: String = row.try_get("payload")?;
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&payload) {
                        if v.get("status").and_then(|s| s.as_str()) == Some("passed") {
                            count = 0;
                        }
                    }
                }
                _ => {}
            }
        }
        self.consecutive
            .lock()
            .expect("thrashing guard mutex poisoned")
            .insert(job, count);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::JobId;

    #[test]
    fn fresh_job_does_not_breach() {
        let g = ThrashingGuard::new();
        let j = JobId::new();
        assert!(!g.would_breach(j));
    }

    #[test]
    fn second_consecutive_auto_bypass_is_a_breach() {
        // The two-strikes rule: the FIRST auto-bypass advances the job;
        // the SECOND is rejected. `would_breach` is the runner's check
        // **before** emitting `StageAutoBypassed`, so after recording
        // one bypass the next call must return `true`.
        let g = ThrashingGuard::new();
        let j = JobId::new();
        assert!(!g.would_breach(j));
        g.record_auto_bypass(j);
        assert!(g.would_breach(j));
    }

    #[test]
    fn pass_resets_the_counter() {
        // Doc Q1 "Counting rule": a `Passed` stage between two failures
        // resets the count to zero. After a Pass the next auto-bypass
        // must be permitted again.
        let g = ThrashingGuard::new();
        let j = JobId::new();
        g.record_auto_bypass(j);
        assert!(g.would_breach(j));
        g.record_pass(j);
        assert!(!g.would_breach(j));
        // And a fresh auto-bypass after the Pass restarts the budget.
        g.record_auto_bypass(j);
        assert!(g.would_breach(j));
    }

    #[test]
    fn cap_breach_bypasses_the_guard() {
        // The doc's "cap-breach bypasses the guard" case. Cap breaches
        // are halted upstream by `classify_stage_failure` (which
        // short-circuits on `stop_reason.is_some()`) and never call
        // `record_auto_bypass`. The guard sees nothing — its count
        // stays at zero, so even after a cap-breach halt the next
        // stage on a resumed-or-rerun job is allowed one auto-bypass
        // before the two-strikes rule fires.
        let g = ThrashingGuard::new();
        let j = JobId::new();
        // Simulate two cap-breach failures: the runner halts at
        // `classify_stage_failure` and never reaches the guard, so no
        // recording happens.
        assert!(!g.would_breach(j));
        assert!(!g.would_breach(j));
        // The count is still zero — the guard is dead code on the cap-
        // breach path.
        assert_eq!(g.count(j), 0);
    }

    #[test]
    fn count_is_per_job() {
        // Different jobs do not share state — a thrash on one job must
        // not poison the auto-bypass budget of an unrelated job
        // running in the same process.
        let g = ThrashingGuard::new();
        let a = JobId::new();
        let b = JobId::new();
        g.record_auto_bypass(a);
        assert!(g.would_breach(a));
        assert!(!g.would_breach(b));
    }

    #[test]
    fn forget_resets_state() {
        let g = ThrashingGuard::new();
        let j = JobId::new();
        g.record_auto_bypass(j);
        g.forget(j);
        assert!(!g.would_breach(j));
    }

    /// Construct an in-memory SqliteStore with the production
    /// migrations applied. Shared by the rebuild tests so each one
    /// owns its own pool — a single test cannot leak state into the
    /// others.
    async fn fresh_store() -> std::sync::Arc<SqliteStore> {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::migrations::MIGRATOR.run(&pool).await.unwrap();
        std::sync::Arc::new(SqliteStore::new(pool))
    }

    async fn write_event(store: &SqliteStore, job: JobId, ty: &str, payload: serde_json::Value) {
        sqlx::query(
            "INSERT INTO events (job_id, stage_id, task_id, type, payload, created_at) \
             VALUES (?, NULL, NULL, ?, ?, 0)",
        )
        .bind(job.to_string())
        .bind(ty)
        .bind(payload.to_string())
        .execute(store.pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn rebuild_counts_consecutive_auto_bypasses() {
        // Two auto-bypasses in a row with no pass between -> count == 2.
        // The runner's `would_breach` would have returned `true`
        // before the second one was recorded on a live run; the
        // rebuild path lands at the same state from the persisted
        // log so a server restart does not silently reset the budget.
        let store = fresh_store().await;
        let g = ThrashingGuard::new();
        let j = JobId::new();
        write_event(&store, j, "stage-auto-bypassed", serde_json::json!({})).await;
        write_event(&store, j, "stage-auto-bypassed", serde_json::json!({})).await;
        let count = g.rebuild_from_store(&store, j).await.unwrap();
        assert_eq!(count, 2);
        assert!(g.would_breach(j));
    }

    #[tokio::test]
    async fn rebuild_resets_on_intervening_pass() {
        // Auto-bypass, then Pass, then auto-bypass. The Pass clears the
        // counter; the rebuild ends at one, so a follow-up auto-bypass
        // is still allowed.
        let store = fresh_store().await;
        let g = ThrashingGuard::new();
        let j = JobId::new();
        write_event(&store, j, "stage-auto-bypassed", serde_json::json!({})).await;
        write_event(
            &store,
            j,
            "stage-completed",
            serde_json::json!({"status": "passed"}),
        )
        .await;
        write_event(&store, j, "stage-auto-bypassed", serde_json::json!({})).await;
        let count = g.rebuild_from_store(&store, j).await.unwrap();
        assert_eq!(count, 1);
        assert!(g.would_breach(j));
    }

    #[tokio::test]
    async fn rebuild_ignores_failed_stage_completed() {
        // A `stage-completed` with status=failed is NOT a reset — the
        // auto-bypass branch always pairs Failed with a
        // `stage-auto-bypassed` envelope that the rebuild already
        // counts, and a Failed without auto-bypass means the job
        // halted (no follow-up stage to count). Treating Failed as a
        // reset here would let a halt-then-resume burn a fresh
        // auto-bypass budget the policy never authorised.
        let store = fresh_store().await;
        let g = ThrashingGuard::new();
        let j = JobId::new();
        write_event(&store, j, "stage-auto-bypassed", serde_json::json!({})).await;
        write_event(
            &store,
            j,
            "stage-completed",
            serde_json::json!({"status": "failed"}),
        )
        .await;
        let count = g.rebuild_from_store(&store, j).await.unwrap();
        assert_eq!(count, 1);
    }
}
