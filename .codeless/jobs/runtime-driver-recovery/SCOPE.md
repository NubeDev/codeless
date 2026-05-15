# Scope — runtime-driver-recovery

## Goal

Fix the three-bug interaction that wedged a real job in `Queued`
state on 2026-05-15:

1. **Worktree creation is not idempotent.** `WorktreeManager.create`
   at `crates/codeless-adapters-host/src/worktree.rs` errors
   `AlreadyExists` on path collision with no `git worktree prune`,
   no adoption path, no recovery.
2. **The driver loop has no retry on error.** `spawn_job_driver_loop`
   at `crates/codeless-runtime/src/job_driver_loop.rs` is purely
   event-driven (`JobQueued` / `JobPromoted` / `JobResumed`); when
   `drive_job` returns an error the spawned task dies silently and
   no further dispatch happens.
3. **The state machine has no edge out of `Queued`.** Per
   `crates/codeless-runtime/src/state_machine.rs`, `Queued` can only
   move to `Running` or `Stopped`. `start_job` requires `Draft`,
   `resume_job` requires `Stopped`/`Failed`/`Paused`. A wedged
   `Queued` job is unrecoverable without `delete_job + resubmit`.

The three compound: stage fails → `Stopped` → user resumes → driver
tries to create the worktree → fails because it exists from the
previous run → no retry → job stays `Queued` forever → user has no
RPC to escape with.

After this job lands, the runtime is self-healing for this class of
failure: worktree creation adopts existing trees, the driver retries
with bounded backoff before giving up, and `reset_job` provides a
user-driven escape hatch for the genuinely unrecoverable cases.

## In scope

- **Bug 2 fix (worktree idempotency).** `WorktreeManager.create`:
  prune first; adopt existing dir if it matches; return
  `AlreadyExists` only when the dir exists and is genuinely
  incompatible (different branch, not a worktree). Unit tests per
  branch.
- **Bug 1 fix (driver retry).** `spawn_job_driver_loop`: classify
  `drive_job` errors; re-publish `JobQueued` with backoff
  (30s/120s/600s) for retryable errors; transition `Queued → Failed`
  with `stop_reason` recorded after max retries; non-retryable
  errors fail immediately. Test-only knob to skip the sleep so unit
  tests run fast.
- **Bug 3 fix (`reset_job` RPC).** New RPC method moving
  `Queued | Failed | Stopped → Draft`. Clears `worktree_path` (after
  best-effort `WorktreeManager.remove`), `stop_reason`, `ended_at`.
  Publishes new `Event::JobReset` variant. Three new edges in
  `state_machine::transition_job`: `(Queued, Draft)`, `(Failed,
  Draft)`, `(Stopped, Draft)`.
- **UI affordance.** Reset button in the job page, visible only in
  `Queued`/`Failed`/`Stopped` statuses. Wired through `RpcClient`.
- **Workspace liveness audit.** Confirm `workspace_liveness.rs`
  never writes to the `jobs` table. Add a test fixture for this.
- **Regression test for the wedge scenario** that fails today and
  passes after all three fixes.

## Out of scope

- A general scheduler. The current driver is "good enough until the
  server gets a real scheduler" per its module docstring. This job
  adds bounded retry, not a real queue.
- Changing the event-bus design. The retry mechanism re-publishes
  existing `JobQueued` events; no new event semantics besides
  `JobReset` and a possible `JobRetryScheduled` (decide in stage 4).
- Multi-tenant retry policies, per-user retry limits.
- An auto-`reset_job` daemon. Manual escape only; the driver retry
  + worktree adoption handle the common cases.
- Worktree GC changes (already covered by `gc_worktrees`).
- TODO comments. CLAUDE.md R4. Mark unfinished stages `[!]` and halt.

## Constraints

- **R1 (crate direction).** `std::process::Command` / `tokio::process`
  stays in `codeless-adapters-host`. The retry classifier in
  `codeless-runtime` must classify errors *by type*, not by spawning
  anything. A grep for `process::Command` outside the adapters crate
  must remain zero.
- **R2 (single transport).** The UI Reset button imports `RpcClient`
  only. No `@tauri-apps/api/*`, no direct `fetch`.
- **R3 (one UI framework).** No `JobPage.web.tsx` for the Reset
  button. One job page.
- **R4 (SQLite is source of truth).** Retry state lives in the
  driver's in-memory map; the `jobs` table records only what's
  authoritative (`Queued` → `Failed` after max retries, with
  `stop_reason`). Do not add a `retry_count` column for MVP. If the
  server restarts mid-backoff, the backlog replay starts the
  retry counter fresh; document this in the decisions file as an
  accepted trade-off.
- **R5 (single-tenant trust).** Unchanged. `reset_job` is gated by
  the same bearer token as every other RPC.
- **Backoff cap.** Hard ceiling at 3 retries (30s, 120s, 600s).
  After that, `Queued → Failed` with `stop_reason = "driver:
  retry-exhausted"`. The user uses `reset_job` to try again.
- **Adoption is conservative.** Worktree adoption checks: dir
  exists, contains a `.git` file (worktree marker), `git -C <path>
  rev-parse --abbrev-ref HEAD` returns the requested branch, and
  the dir is in `git worktree list --porcelain`. Any mismatch is
  `AlreadyExists`, not adoption.
- **No `--force` worktree removal as part of adoption.** Adoption
  is non-destructive; `reset_job` does the destructive remove
  separately.
- **Comments per CLAUDE.md R2.** No emojis, no task-status
  comments, no restatements, no banners.

## Resolution required from open design points

Stage 4 (REVIEW) MUST record these decisions in
`DOCS/RUNTIME-DRIVER-RECOVERY-DECISIONS.md`:

1. Should `reset_job` be allowed from `Running`? Lean **no** —
   user must `pause_job` or `stop_job` first. Mirrors the existing
   `delete_job` behaviour.
2. Should the retry policy be configurable per repo or per job?
   Lean **no for MVP** — hardcoded backoff, revisit when a real
   need surfaces.
3. Is there a `JobRetryScheduled` event, or do retries silently
   re-publish `JobQueued`? Lean **explicit event** so the UI can
   show "retry in 30s" instead of "queued forever again."
4. Error classification — what's retryable vs not? Document the
   list. First cut: `WorktreeError::AlreadyExists` (retryable
   only if adoption now succeeds; otherwise non-retryable),
   `WorktreeError::Io` (retryable), `WorktreeError::GitFailed`
   (retryable for transient codes, non-retryable for malformed
   args), runner-not-enabled (non-retryable), template parse
   (non-retryable).
5. What does the server-restart-mid-backoff path look like? Lean
   **counter resets**, plus a 30s "settle" delay on backlog replay
   so a crash loop doesn't immediately re-fire failures.

## Pointers

- Bug 2 site: `codeless/crates/codeless-adapters-host/src/worktree.rs:74-97`
- Bug 1 site: `codeless/crates/codeless-runtime/src/job_driver_loop.rs:80-145`
- Bug 3 sites:
  - `codeless/crates/codeless-rpc/src/methods.rs` (add `ResetJobArgs`)
  - `codeless/crates/codeless-runtime/src/rpc/jobs.rs` (`reset_job` fn)
  - `codeless/crates/codeless-runtime/src/state_machine.rs` (new edges)
  - `codeless/crates/codeless-types/src/event.rs` (`Event::JobReset`)
- Workspace rules: `../CLAUDE.md` (workspace), `./CLAUDE.md` (inner repo)
- The post-mortem of the wedge: this very stage of conversation;
  if extracted, drop into `DOCS/POSTMORTEM-2026-05-15-driver-wedge.md`.
