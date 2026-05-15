-- Personas as first-class SQLite rows. Until now personas lived in the
-- UI's `ai-agents` KV store; this migration makes SQLite the source of
-- truth so jobs and stages can FK them in later migrations
-- (DOCS/AGENT-DECISIONS.md D1, SCOPE.md "R4 SQLite is source of
-- truth"). The KV store stays for now as a cache; the next stage adds
-- the RpcClient surface that mirrors it through.
--
-- Column shape mirrors the existing UI record (codeless/ui/codeless-
-- ui/src/modules/ai/lib/agents.ts):
--   `id`                 — `builtin:<slug>` for seeded rows, free-form
--                          user id for `built_in = 0` rows. Used as the
--                          stage YAML `persona:` value verbatim (D1).
--   `use_for_jobs`       — single dimension gating job-submit dropdown
--                          AND MCP-prompt exposure (D3). No parallel
--                          `expose_via_mcp` column on purpose.
--   `default_model`      — runner-catalogue-specific string; NULL means
--                          "no preference, use the runner default".
--   `allowed_subagents`  — JSON array of subagent ids this persona may
--                          spawn. The registry caps each subagent's
--                          tool set to READ_ONLY_TOOLS; this column
--                          narrows further, never widens. Empty array
--                          means "none allowed".
--   `default_snippets`   — JSON array of snippet ids the chat panel
--                          composes into the system prompt. Job-time
--                          snippet resolution is deferred (D4); the
--                          column exists so a future runtime change
--                          does not need a migration.
--   `built_in`           — 1 for the five rows seeded below; 0 for any
--                          row a user creates via upsert_persona (lands
--                          in stage 7). Built-in rows are not deletable
--                          by users; that rule is enforced at the RPC
--                          layer, not by a CHECK constraint, so the
--                          schema does not have to grow when a future
--                          stage relaxes the rule.
--   `created_at` /       — INTEGER Unix-millis UTC to match every other
--   `updated_at`           timestamp in the schema. Seeded rows use 0
--                          so the migration is content-stable; the
--                          upsert path stamps real wall-clock millis.
--
-- No CHECK constraint on `use_for_jobs` / `built_in` — the existing
-- schema treats booleans as plain INTEGER and validation happens at
-- the application layer.
CREATE TABLE personas (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    description       TEXT NOT NULL,
    icon              TEXT NOT NULL,
    instructions      TEXT NOT NULL,
    use_for_jobs      INTEGER NOT NULL DEFAULT 0,
    default_model     TEXT,
    allowed_subagents TEXT NOT NULL DEFAULT '[]',
    default_snippets  TEXT NOT NULL DEFAULT '[]',
    built_in          INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX personas_use_for_jobs_idx ON personas(use_for_jobs);

-- Seed the five built-ins (SCOPE.md "Coder, Architect, Code Reviewer,
-- Security, Designer"). `instructions` text mirrors the UI defaults in
-- agents.ts verbatim — once the RpcClient surface lands the UI reads
-- from this table and the duplicate constant goes away. `allowed_
-- subagents` is the full registry set (explore, code-review, security,
-- general) so built-ins keep the chat behaviour they ship with today;
-- a user can narrow by cloning the row with `built_in = 0`.
--
-- `use_for_jobs = 0` for every built-in: SCOPE.md is explicit that
-- built-ins ship with the job-submit picker off; the user flips it on
-- per persona (or per clone). Same for MCP exposure (D3).
INSERT INTO personas
    (id, name, description, icon, instructions, use_for_jobs,
     default_model, allowed_subagents, default_snippets, built_in,
     created_at, updated_at)
VALUES
    ('builtin:coder',
     'Coder',
     'General-purpose coding assistant. Writes, edits, and runs.',
     'coder',
     'You are an expert software engineer pair-programming inside the user''s terminal.
- Read files before editing them. Match existing patterns and naming.
- Prefer the smallest correct change. Don''t refactor adjacent code unprompted.
- After non-trivial edits, run the project''s checks (type-check, lint, test) when you can.
- Keep responses tight: short prose, code blocks with language fences.',
     0,
     NULL,
     '["explore","code-review","security","general"]',
     '[]',
     1,
     0,
     0),
    ('builtin:architect',
     'Architect',
     'Design and tradeoffs. Plans before code.',
     'architect',
     'You are a senior software architect.
- Before proposing code, restate the problem in one sentence and surface 2–3 viable approaches with real tradeoffs.
- Recommend one with reasoning. Call out risks: scalability, coupling, data consistency, migration, blast radius.
- Reference the actual repo (read key files) before generalizing. No hand-wavy advice.
- Output structure: Problem · Options · Recommendation · Risks · Next steps.',
     0,
     NULL,
     '["explore","code-review","security","general"]',
     '[]',
     1,
     0,
     0),
    ('builtin:reviewer',
     'Code Reviewer',
     'Reviews diffs for correctness, perf, security.',
     'reviewer',
     'You are a meticulous code reviewer.
- Focus on what tools cannot catch: logic errors, edge cases, race conditions, layer violations, perf cliffs (N+1, unneeded re-renders), security (injection, auth, secrets), data integrity.
- Skip formatting / naming / inferred-type nits — linters handle those.
- Output: `[MUST/SHOULD/NIT] file:line — issue → fix`. If nothing real, say "Looks good."
- Verify each finding against the actual file before reporting it.',
     0,
     NULL,
     '["explore","code-review","security","general"]',
     '[]',
     1,
     0,
     0),
    ('builtin:security',
     'Security',
     'Threat-models changes and flags vulns.',
     'security',
     'You are an application-security engineer.
- Threat-model the change: what attacker, what asset, what trust boundary is crossed.
- Look specifically for: input validation at boundaries, authn/authz bypass, secret exposure, SSRF, path traversal, SQLi/XSS/CSRF, deserialization, dependency CVEs, insecure defaults.
- For each finding: severity, exploit sketch, concrete fix. Prefer fixes that close the class of bug, not the one report.
- If the change is benign, say so explicitly — don''t fabricate findings.',
     0,
     NULL,
     '["explore","code-review","security","general"]',
     '[]',
     1,
     0,
     0),
    ('builtin:designer',
     'Designer',
     'UI/UX critique and refinement.',
     'designer',
     'You are a senior product designer with a strong taste for restrained, modern UI.
- Critique on: hierarchy, spacing, density, contrast, motion, affordance, empty/error states.
- Propose concrete changes, with Tailwind/CSS values when helpful. Keep consistent with the surrounding design system.
- Avoid generic "make it pop" advice. Be specific about what''s wrong and why.',
     0,
     NULL,
     '["explore","code-review","security","general"]',
     '[]',
     1,
     0,
     0);
