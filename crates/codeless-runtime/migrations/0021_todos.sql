-- User-visible sub-steps within a task. Backs the `Stages` overview's
-- third nesting level (Stage > Tick > Todo) so a long-running task
-- shows continuous progress instead of looking stalled between
-- `task-started` and `task-completed`. See DOCS/SCOPE.md `Todo` row
-- and DOCS/JOB-UI.md "Todo rows (nested under a tick)".
--
-- `kind` carries the origin of the row so the UI and the
-- stage-completion gate can distinguish runner-authored items from
-- the runtime-injected closing trio. Wire labels match
-- `TodoKind` (`runner`, `planner`, `checks`, `docs`, `git`).
--
-- `status` wire labels match `TodoStatus` (`pending`, `in-progress`,
-- `done`, `skipped`, `failed`). The stage-completion gate refuses to
-- fire `StageCompleted` until every trio row (`kind IN ('checks',
-- 'docs', 'git')`) on the stage's terminal task is in `done` or
-- `skipped`.
--
-- `(task_id, ordinal)` is unique so the StageRecorder can upsert by
-- position without a follow-up read; ordinals come from the emitter
-- and the trio is written with the three highest ordinals seen for
-- the task.

CREATE TABLE todos (
    id           TEXT PRIMARY KEY,
    task_id      TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL,
    title        TEXT NOT NULL,
    status       TEXT NOT NULL,
    kind         TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    started_at   INTEGER,
    ended_at     INTEGER,
    UNIQUE(task_id, ordinal)
);
CREATE INDEX todos_task_idx ON todos(task_id, ordinal);
CREATE INDEX todos_trio_idx ON todos(task_id) WHERE kind IN ('checks', 'docs', 'git');
