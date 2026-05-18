-- Pre-armed supervisor goals (JOB-CHAT.md §C3). One row per "if X then Y"
-- intent the user expressed in the per-Job chat thread; the supervisor
-- inserts a row when it parses the intent and re-arms timers / event
-- watchers from the table on boot so a process restart does not lose
-- the authorisation. Hard rule 4 of JOB-CHAT.md is what makes this
-- table load-bearing — the "if it runs >1h, stop it" example only
-- works end-to-end if the goal survives the supervisor's lifecycle.
--
-- `run_id` is the Run attempt the goal is scoped to. JOB-WORKFLOW (B)
-- (the Job/Run split that introduces a `RunId` newtype) has not landed
-- yet; pre-(B) the supervisor scopes itself per JobId and writes the
-- JobId string here. Once (B) lands the column shape stays the same —
-- the value swaps from a JobId-shaped ULID to a RunId-shaped ULID
-- without a schema change.
--
-- `kind` is the v0.1 closed set from JOB-CHAT.md (C3):
--   * deadline-stop   — fire `stop_job` when a wall-clock deadline
--                       trips. The `condition_json` carries the
--                       absolute `deadline_ms` so a restart re-arms
--                       the same timer rather than restarting it from
--                       boot time.
--   * threshold-stop  — fire `stop_job` when a numeric metric
--                       (cost cents, wall-clock ms, …) crosses a
--                       threshold; the supervisor's metric poll fires
--                       the goal.
--   * event-notify    — post a chat reply when a named Event variant
--                       (e.g. `StageCompleted` for a specific stage)
--                       lands on the bus. No destructive action.
-- Adding a sixth kind requires a doc PR amending JOB-CHAT.md (C3) and
-- a fresh migration; the store rejects unknown kinds at write time so
-- a typo does not silently land an unrecognised row.
--
-- `condition_json` and `action_json` are typed-enum payloads. The
-- store validates both on insert (`GoalCondition` / `GoalAction` in
-- `store::supervisor_goals`) so a malformed row cannot reach disk.
-- The columns stay TEXT (not JSON1's JSONB) so the migration is
-- compatible with the same SQLite builds the rest of the schema
-- relies on; structured access is the runtime's job, not SQL's.
--
-- `authorised_by` references `chat_messages.id` — the user message
-- whose body authorised the goal. The audit trail JOB-CHAT.md Hard
-- rule 4 promises ("the original 'if X then Y' message plus the
-- post-action summary, both rows in chat_messages") is exactly this
-- foreign-key edge.
--
-- `status` walks `armed → fired | cancelled | superseded`. Terminal
-- statuses are mutually exclusive: `fired` is the success path,
-- `cancelled` is the user changed their mind, `superseded` is the
-- Run already ended before the condition could trip. The store's
-- `mark_*` helpers each take the goal out of `armed` exactly once;
-- the `idx_supervisor_goals_armed` partial index keeps the
-- rehydration scan on supervisor boot cheap.
--
-- `fired_at` is NULL until a `mark_*` terminal transition lands; the
-- timestamp matches whichever transition the helper recorded.
CREATE TABLE supervisor_goals (
    id              TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL,
    kind            TEXT NOT NULL,
    condition_json  TEXT NOT NULL,
    action_json     TEXT NOT NULL,
    authorised_by   TEXT NOT NULL REFERENCES chat_messages(id),
    status          TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    fired_at        INTEGER
);

-- Rehydration on supervisor boot is "scan armed rows for the Run, re-
-- arm timers / event watchers, drop stale ones" (JOB-CHAT.md §C3).
-- The partial index narrows the scan to rows that still need
-- rehydration; terminal-status rows are kept for the audit trail but
-- never re-armed.
CREATE INDEX idx_supervisor_goals_armed
    ON supervisor_goals (run_id, created_at)
    WHERE status = 'armed';
