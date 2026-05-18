# scoped-pause-points — stage 5 → stage 6 (runtime hook)

Stage 5 (persistence) landed. Next stage wires the runtime hook in
the stage/todo state machine to consult the schedule before
advancing.

## What landed in stage 5

- Migration `crates/codeless-runtime/migrations/0022_scheduled_pause_points.sql`.
  Table keyed on `(job_id, ordinal)` per SCOPED-PAUSE-POINTS §4 —
  `ordinal` is the 1-based YAML index, not the resolved stage
  ordinal, so a resync that renumbers stages does not orphan rows.
  Columns: `job_id`, `ordinal`, `point_id`, `target_json`,
  `position`, `reason`, `created_at`. `target_json` carries a
  `PausePointTarget` serialised through the wire-shape serde derive
  so the column round-trips through the same parser the wire uses.
  FK `job_id` → `jobs(id)` `ON DELETE CASCADE`.

- `crates/codeless-runtime/src/store/scheduled_pause_points.rs` —
  store module exposing two methods on `SqliteStore`:
  - `replace_scheduled_pause_points(job_id, &[PausePoint], now)` —
    idempotent rebuild inside one transaction (DELETE then bulk
    INSERT). Empty slice drops the schedule for the job.
  - `list_scheduled_pause_points(job_id)` — YAML-ordered load.
  Seven async tests cover round-trip across every selector variant,
  idempotency on repeated input, row drop on schedule shrink,
  ordinal renumbering on schedule reorder, per-job isolation, and
  the `ON DELETE CASCADE` from `jobs`.

- `resync_template_from_disk` (in `rpc/jobs.rs`) and
  `update_job_template` (in `rpc/job_files.rs`) now call the new
  `rebuild_scheduled_pause_points(rpc, job_id, &parsed)` helper
  before publishing `JobTemplateUpdated`. Resolution failures are
  surfaced as `RpcError::InvalidArgument` with the full punch list
  joined by `; ` so a chat-driven edit that breaks the schedule is
  refused before the row set diverges from the YAML on disk.

- `crates/codeless-runtime/tests/migrations.rs` —
  `scheduled_pause_points` added to the Appendix A allow-list and a
  new test (`scheduled_pause_points_table_keys_on_job_and_ordinal`)
  asserts the column order, the `scheduled_pause_points_job_idx`
  index, and the `ON DELETE CASCADE` foreign key.

## Verify

- `cargo test --workspace` — green.
- `cargo clippy --workspace --all-targets -- -D warnings` — green.
- `cargo fmt --check` — green.

## What stage 6 needs from this

- The schedule is now durable per-job. The runtime hook should read
  it via `SqliteStore::list_scheduled_pause_points(job_id)` at the
  same point in the transition where the trio gate inspects stage
  completion (`stage 1 handover` note).
- Identity is `PausePointId`. The parser mints a fresh one per
  resolve, so the runtime cannot rely on id stability across a
  resync. If stage 6 needs "did this exact point already fire?"
  semantics, add `fired_at` / `superseded_at` columns in a follow-up
  migration — those were called out in stage 1 §4 but deferred so
  the persistence diff stays focused.
- `StopReason::ScopedPausePoint { point_id, label }` is **not yet**
  added to `codeless_types::StopReason`; stage 6 owns that change
  along with the actual `pause_job` call site.

## Open follow-ups (do not act in stage 6 unless explicitly needed)

- `fired_at` / `superseded_at` columns and the question-3 resync
  semantics (silenced no-ops with a note in the resync event payload).
- A `pause_points_updated` event variant on `Event` so the UI can
  refresh the divider chips without re-reading the whole job state.
