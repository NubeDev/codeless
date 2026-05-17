-- PS5 (DOCS/PLUGIN-SUBSTRATE.md item 5): the persona / thread-kind data
-- model. Two changes land together because they are read together at
-- agent-call time:
--
--   1. `personas` grows three substrate-doc columns the runner needs to
--      compose a turn from a persona alone: `allowed_tools` (the MCP
--      tool cap derived server-side per PS3), `default_model_family`
--      (a codeless-side family alias the runner resolves to a concrete
--      provider model -- never a provider model id directly, see the
--      manifest contract in item 6), and `default_attachments_policy`
--      (how thread uploads are surfaced into the prompt). The existing
--      `instructions` column already plays the role of `system_prompt`
--      and is reused verbatim -- adding a fourth column with the same
--      meaning would split the source of truth.
--
--      `allowed_tools` is stored as a JSON array string so the
--      application-side decoder shares one path with
--      `allowed_subagents` / `default_snippets`. `default_model_family`
--      is nullable because "no preference, fall through to the runner
--      default" is a real value distinct from "smart" / "fast" /
--      "reasoning". `default_attachments_policy` defaults to
--      `inline-thread-scoped` (the substrate-doc example) so existing
--      rows have a defined policy without a backfill of every column.
--
--   2. `assistant_threads.persona_id` is added NOT NULL with an FK to
--      `personas(id)` ON DELETE RESTRICT. The substrate doc pins the
--      persona to the thread lifetime -- the runner must be able to
--      reproduce the agent posture for the lifetime of the thread, so
--      deleting the persona while threads point at it is refused at the
--      schema level. SQLite cannot ADD COLUMN ... NOT NULL REFERENCES
--      without a sensible default, and ADD COLUMN cannot inline a CHECK
--      that depends on another row, so this stage rebuilds the table.
--      Existing rows (pre-PS5) back-fill to `builtin:general`.
--
-- Two built-in personas (`builtin:general`, `builtin:coding`) are
-- seeded as the substrate-doc-mandated defaults. The five legacy
-- builtins (`coder`, `architect`, `reviewer`, `security`, `designer`)
-- stay on the table -- the job-side persona picker still references
-- them -- and back-fill the new columns from the seeded defaults via
-- the column-level DEFAULT clauses.

ALTER TABLE personas ADD COLUMN allowed_tools TEXT NOT NULL DEFAULT '[]';
ALTER TABLE personas ADD COLUMN default_model_family TEXT;
ALTER TABLE personas ADD COLUMN default_attachments_policy TEXT NOT NULL
    DEFAULT 'inline-thread-scoped';

-- `builtin:general` -- the existing Assistant default persona. No
-- MCP tool grants at all: the substrate-doc seam in PS3 is now backed
-- by a real list, and the general persona is intentionally the
-- narrowest of the two so the default conversation cannot reach into
-- a plugin's tools without a deliberate persona switch.
INSERT INTO personas
    (id, name, description, icon, instructions, use_for_jobs,
     default_model, allowed_subagents, default_snippets, built_in,
     allowed_tools, default_model_family, default_attachments_policy,
     created_at, updated_at)
VALUES
    ('builtin:general',
     'General',
     'Default Assistant persona. Conversational, no tool grants.',
     'spark',
     'You are the codeless Assistant. Answer the user clearly and concisely.
- Prefer direct answers over speculation; ask for the missing piece when the question is under-specified.
- You have no tool access in this persona; if the user asks for an action that needs a tool, say so and suggest switching to a persona that grants it.',
     0,
     NULL,
     '[]',
     '[]',
     1,
     '[]',
     'smart',
     'inline-thread-scoped',
     0,
     0),
    ('builtin:coding',
     'Coding',
     'Job-runner-style coding assistant. Read + edit + shell.',
     'coder',
     'You are a coding assistant pair-programming inside the user''s repo.
- Read files before editing them. Match existing patterns and naming.
- Prefer the smallest correct change. Don''t refactor adjacent code unprompted.
- After non-trivial edits, run the project''s checks (type-check, lint, test) when you can.
- Keep responses tight: short prose, code blocks with language fences.',
     0,
     NULL,
     '["explore","code-review","security","general"]',
     '[]',
     1,
     '["fs.*","shell.*","attachments.read"]',
     'smart',
     'inline-thread-scoped',
     0,
     0);

-- Rebuild `assistant_threads` so `persona_id` is NOT NULL and declares
-- the FK to personas. Column order is meaningful: every store-side
-- `SELECT *` decoder reads by name, but `migrations.rs::
-- assistant_tables_match_stage_5_schema` asserts the column order
-- verbatim, so keep `persona_id` slotted before the timestamps.
CREATE TABLE assistant_threads_new (
    id         TEXT PRIMARY KEY,
    title      TEXT NOT NULL,
    persona_id TEXT NOT NULL REFERENCES personas(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO assistant_threads_new (id, title, persona_id, created_at, updated_at)
SELECT id, title, 'builtin:general', created_at, updated_at FROM assistant_threads;

DROP TABLE assistant_threads;
ALTER TABLE assistant_threads_new RENAME TO assistant_threads;
CREATE INDEX assistant_threads_updated_idx
    ON assistant_threads(updated_at DESC);
