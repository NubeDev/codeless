# Runtime driver recovery — decisions

Records the design calls made while landing the three-bug fix described
in `.codeless/jobs/runtime-driver-recovery/SCOPE.md`. Stage 4 owns the
mid-implementation decisions (retry classifier, `reset_job` source
states, retry event); stage 7 owns the workspace-liveness audit at the
bottom. Future agents touching the driver loop, the worktree adapter,
or the liveness sweep should read this before changing behaviour.

## Stage 4 — mid-implementation decisions

These were resolved while landing stages 2, 3, and 5. Recorded here so
the rationale survives the commit messages.

1. **`reset_job` is rejected from `Running`, `Paused`, `AwaitingReview`
   and `Completed`.** Source states are `Queued | Failed | Stopped`
   only. Matches the existing `delete_job` rule: the user must
   `pause_job` or `stop_job` first. Avoids racing the driver's own
   transitions.
2. **Retry policy is not configurable.** Hardcoded backoff of 30s /
   120s / 600s with a hard cap of three attempts. Per-repo or per-job
   tuning is deferred until a real need surfaces; the manual
   `reset_job` hatch covers the long tail.
3. **Retries are silent — no `JobRetryScheduled` event for MVP.** The
   re-publish goes through the existing `JobQueued` event so the UI's
   queue-position read remains the single source of truth. Revisit if
   a user complains that "Queued forever" is indistinguishable from
   "Queued, will retry in 30s".
4. **Retry classifier:**
   - Retryable: `WorktreeError::Io`, `WorktreeError::GitFailed` with a
     transient exit code, transient sqlx errors (busy / locked).
     `WorktreeError::AlreadyExists` is retryable *only because* the
     stage 2 adoption path means it now rarely surfaces — when it does,
     it indicates a genuine incompatibility and the next attempt will
     hit the same path; the retry exists to absorb the rare race where
     a parallel cleanup is in flight.
   - Non-retryable: runner-not-enabled, template parse errors,
     `WorktreeError::GitFailed` with a malformed-arg style code. These
     transition `Queued → Failed` immediately with the original error
     recorded in `stop_reason`.
5. **Server restart mid-backoff resets the retry counter.** Accepted
   trade-off: the in-memory map in the driver is lost on restart, the
   backlog replay starts the counter at zero, and the 30s settle delay
   on backlog replay prevents an immediate crash-loop refire. A
   persisted `retry_count` column was rejected — the column would only
   matter for a tiny minority of crashes mid-backoff and would
   complicate every future state-machine edge.

## Stage 7 — workspace liveness sweep audit

The wedge post-mortem listed `workspace_liveness.rs` as a suspect for
flipping `Stopped → Queued`. The audit clears it.

**Files audited:** `crates/codeless-runtime/src/workspace_liveness.rs`
in full. Confirmed via `rg`:

- No `INSERT`, `UPDATE`, or `DELETE` statement targeting the `jobs`,
  `stages`, `tasks`, `runs` or `stage_events` tables anywhere in the
  file.
- The only SQL the sweep issues is a read against
  `attached_workspaces.fs_root_canonical` to resolve `fs_root → repo_id`
  for the event payload.
- The only side effects are `EventBus::publish` calls carrying
  `Event::WorkspaceUnhealthy` and `Event::WorkspaceRecovered`. Both are
  workspace-scoped (`repo_id`, `fs_root`, optional `reason`); neither
  variant carries a `job_id`, and the bus does not auto-correlate
  workspace events to job rows.
- The edge-detection state is per-task in-memory
  (`HashMap<PathBuf, bool>`); it does not flush to the database.

**Finding:** the sweep is a pure observer of the filesystem; it cannot
have caused the wedge. The bug was the compound of bugs 1, 2 and 3
captured in SCOPE.md. Whatever flipped the job back to `Queued` came
from the driver's own re-publish path, not from this sweep.

**Pin:** `workspace_liveness::tests::sweep_never_writes_to_jobs_table`
seeds one job row per status the post-mortem flagged
(`Draft|Queued|Running|Stopped|Failed`), drives the sweep through both
the unhealthy and recovered edges, and asserts every row's `status`,
`stop_reason`, `worktree_path`, `started_at` and `ended_at` are
byte-identical before and after. If a future change adds a job-table
write to the sweep — intentionally or otherwise — this test fires
before it ships.

**Implication for future work:** if a "workspace went unhealthy →
running jobs should pause" feature is ever added, it must live in a
new subscriber on `Event::WorkspaceUnhealthy` — not as a side effect
inside `sweep_once`. Keeping the sweep job-table-free is what makes
its test surface tractable and what lets the driver remain the single
writer of job state.
