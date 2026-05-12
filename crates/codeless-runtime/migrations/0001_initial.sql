-- Initial Codeless schema. Source of truth: DOCS/SCOPE.md "Appendix A —
-- Phase 1 SQLite schema sketch". All IDs are ULID stored as TEXT
-- (sortable, URL-safe). Money is INTEGER cents-USD. Timestamps are
-- INTEGER Unix-millis UTC. No down-migration: this is the base; later
-- migrations are forward-only ALTERs.

CREATE TABLE repos (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    clone_url       TEXT NOT NULL,
    default_branch  TEXT NOT NULL,
    local_path      TEXT NOT NULL,
    git_auth        TEXT NOT NULL,
    concurrency_cap INTEGER,
    default_runner  TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE jobs (
    id                TEXT PRIMARY KEY,
    repo_id           TEXT NOT NULL REFERENCES repos(id) ON DELETE RESTRICT,
    status            TEXT NOT NULL,
    stop_reason       TEXT,
    template_yaml     TEXT,
    prompt            TEXT,
    runner            TEXT NOT NULL,
    branch            TEXT NOT NULL,
    worktree_path     TEXT,
    cost_cap_cents    INTEGER NOT NULL,
    wall_clock_cap_ms INTEGER NOT NULL,
    cost_cents        INTEGER NOT NULL DEFAULT 0,
    started_at        INTEGER,
    ended_at          INTEGER,
    created_at        INTEGER NOT NULL
);
CREATE INDEX jobs_status_idx ON jobs(status);
CREATE INDEX jobs_repo_idx   ON jobs(repo_id, created_at);

CREATE TABLE stages (
    id          TEXT PRIMARY KEY,
    job_id      TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    ordinal     INTEGER NOT NULL,
    name        TEXT NOT NULL,
    status      TEXT NOT NULL,
    verify_cmd  TEXT,
    started_at  INTEGER,
    ended_at    INTEGER
);
CREATE INDEX stages_job_idx ON stages(job_id, ordinal);

CREATE TABLE tasks (
    id                TEXT PRIMARY KEY,
    stage_id          TEXT NOT NULL REFERENCES stages(id) ON DELETE CASCADE,
    ordinal           INTEGER NOT NULL,
    status            TEXT NOT NULL,
    -- JSON array of TaskId; empty in linear-execution mode, populated
    -- once topological scheduling lands (SCOPE.md Rule 4).
    depends_on        TEXT NOT NULL DEFAULT '[]',
    -- "<pid>:<startup-nonce>" while leased; NULL when idle. The
    -- startup-nonce is minted once per core-process start so PID-reuse
    -- after a crash can't be mistaken for an alive holder.
    lease_holder      TEXT,
    lease_expires_at  INTEGER,
    cost_cents        INTEGER NOT NULL DEFAULT 0,
    input_tokens      INTEGER NOT NULL DEFAULT 0,
    output_tokens     INTEGER NOT NULL DEFAULT 0,
    started_at        INTEGER,
    ended_at          INTEGER
);
CREATE INDEX tasks_stage_idx        ON tasks(stage_id, ordinal);
CREATE INDEX tasks_lease_expiry_idx ON tasks(lease_expires_at) WHERE status = 'running';

CREATE TABLE reviews (
    id            TEXT PRIMARY KEY,
    stage_id      TEXT NOT NULL REFERENCES stages(id) ON DELETE CASCADE,
    status        TEXT NOT NULL,
    comment       TEXT,
    requested_at  INTEGER NOT NULL,
    resolved_at   INTEGER
);
CREATE INDEX reviews_status_idx ON reviews(status);

CREATE TABLE events (
    cursor      INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id      TEXT,
    stage_id    TEXT,
    task_id     TEXT,
    type        TEXT NOT NULL,
    payload     TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX events_job_cursor_idx ON events(job_id, cursor);
CREATE INDEX events_created_at_idx ON events(created_at);

CREATE TABLE pty_sessions (
    id              TEXT PRIMARY KEY,
    job_id          TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    cwd             TEXT NOT NULL,
    opened_by       TEXT NOT NULL,
    opened_at       INTEGER NOT NULL,
    last_activity   INTEGER NOT NULL,
    closed_at       INTEGER
);
CREATE INDEX pty_idle_idx ON pty_sessions(last_activity) WHERE closed_at IS NULL;
