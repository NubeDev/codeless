-- FK promotion + per-stage persona binding (AGENT-DECISIONS.md D1).
--
-- Two changes land together so the persona id rides every level of
-- the job graph that consumes it:
--
--   1. `jobs.persona_id` was added in 0010 as a free TEXT so the
--      MVP slice could persist the picked persona before SQLite owned
--      personas (0011 closed that gap). This migration rebuilds
--      `jobs` so the column declares `REFERENCES personas(id)
--      ON DELETE SET NULL`. SQLite has no `ADD CONSTRAINT`; the
--      table-rebuild dance is the documented recipe. `SET NULL` and
--      not `RESTRICT` because deleting a persona is a UI-allowed
--      operation (stage 7 `delete_persona`) and the per-row prompt
--      stays available on `jobs.system_prompt` even if the lookup
--      key is severed -- a rerun loses the persona binding but the
--      original prompt still reproduces the run.
--
--   2. `stages.persona_id` is new. A stage that overrides the
--      job-level persona (D1's per-stage `persona:` YAML key, parsed
--      in a later stage) records the resolved id on the row so the
--      per-stage handover can name the persona the stage ran under.
--      Without the override the column stays NULL and the stage
--      inherits the job's persona at read time -- a NULL here does
--      not mean "no persona," it means "use the job's."
--
-- FKs are *declared* here; enforcement is a per-connection PRAGMA
-- the runtime does not set today, so this migration is schema-only
-- and cannot break existing rows. When the runtime turns FKs on,
-- `ON DELETE SET NULL` makes the cleanup automatic.

-- Rebuild `jobs` so `persona_id` declares the FK. Column order matches
-- the existing schema exactly so `migrations.rs::jobs_columns_match_
-- appendix_a` and every `SELECT *`-driven row decoder keep working.
CREATE TABLE jobs_new (
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
    created_at        INTEGER NOT NULL,
    model             TEXT,
    permission_mode   TEXT,
    effort            TEXT,
    workspace_mode    TEXT NOT NULL DEFAULT 'in-repo',
    system_prompt     TEXT,
    persona_id        TEXT REFERENCES personas(id) ON DELETE SET NULL
);

INSERT INTO jobs_new (
    id, repo_id, status, stop_reason, template_yaml, prompt, runner,
    branch, worktree_path, cost_cap_cents, wall_clock_cap_ms,
    cost_cents, started_at, ended_at, created_at, model,
    permission_mode, effort, workspace_mode, system_prompt, persona_id
)
SELECT
    id, repo_id, status, stop_reason, template_yaml, prompt, runner,
    branch, worktree_path, cost_cap_cents, wall_clock_cap_ms,
    cost_cents, started_at, ended_at, created_at, model,
    permission_mode, effort, workspace_mode, system_prompt, persona_id
FROM jobs;

DROP TABLE jobs;
ALTER TABLE jobs_new RENAME TO jobs;
CREATE INDEX jobs_status_idx ON jobs(status);
CREATE INDEX jobs_repo_idx   ON jobs(repo_id, created_at);

-- New per-stage column. SQLite's ALTER TABLE allows a column-level
-- REFERENCES on ADD COLUMN when the default is NULL, which it is by
-- omission here.
ALTER TABLE stages ADD COLUMN persona_id TEXT REFERENCES personas(id) ON DELETE SET NULL;
