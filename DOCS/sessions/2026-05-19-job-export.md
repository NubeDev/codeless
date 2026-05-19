# 2026-05-19 — job-export

Job [`job-export`](../../.codeless/jobs/job-export/SCOPE.md) implements
the `.codeless-job` bundle (E1 of
[`DOCS/SCOPE-JOB-EXPORT.md`](../SCOPE-JOB-EXPORT.md)): export one Job
with its full per-Run history, import into another workspace, hit
run.

## Stage 1 — survey + precondition check  `[!]` halted

Stage 1 is "survey current Job/Run schema and confirm JOB-WORKFLOW
(B) precondition status; list every table column the bundle must
serialize."

The job's own
[`WORKFLOW.md`](../../.codeless/jobs/job-export/WORKFLOW.md#precondition-check-stage-1)
declares this as a hard halt:

> Stage 1's first action: confirm JOB-WORKFLOW (B) — the Job/Run
> split — is merged. Grep for the `runs` table migration; if it does
> not exist, halt the stage with `[!]` in the session doc and write
> a handover explaining that this job is blocked on (B). Do not try
> to scaffold around it; the bundle layout depends on immutable Run
> rows.

### Precondition check — result: **NOT MERGED**

`ls crates/codeless-runtime/migrations/` lists 27 migrations (0001
through 0028, no 0018). None of them creates a `runs` table. A grep
across the migration set for `runs`, `template_snapshot`, and
`handover_snapshot` returns only stray prose comments — no DDL:

```
$ grep -i "runs\|template_snapshot\|handover_snapshot" crates/codeless-runtime/migrations/*.sql
0009_job_system_prompt.sql:-- the row so reruns and resumes reproduce the prompt the user picked.
0011_personas.sql:     'General-purpose coding assistant. Writes, edits, and runs.',
0016_precheck_override_once.sql:-- the runner just before the pre-check runs. Distinct from
0026_supervisor_goals.sql:-- table load-bearing — the "if it runs >1h, stop it" example only
```

The `jobs` table still carries the per-attempt columns that
[JOB-WORKFLOW.md (B)](../JOB-WORKFLOW.md#b-full-step--split-job-and-run)
hoists onto `runs`: `runner`, `branch`, `worktree_path`,
`cost_cap_cents`, `wall_clock_cap_ms`, `cost_cents`, `started_at`,
`ended_at`, `template_yaml`, `prompt`. There is no immutable
per-attempt record, no `handover_md` column on jobs, no
`(job_id, ordinal)` uniqueness.

The codebase is on phase **(A)** of JOB-WORKFLOW. The `runs/NNNN/`
layout this bundle is built around therefore cannot be cleanly
constructed — there is exactly one implicit run per Job, with its
state co-mingled into the mutable `jobs` row. Exporting it as a
"frozen Run with `template_snapshot` / `handover_snapshot`" would
require either (i) fabricating snapshots at export time (which is
not what (B) promises and breaks resume-from-stage semantics in
imported history) or (ii) inventing a different bundle shape that
the importer in a post-(B) world will then have to translate. Both
violate `SCOPE.md`'s precondition and `WORKFLOW.md`'s "do not
scaffold around it."

This job is therefore **blocked on JOB-WORKFLOW (B)** and stage 1
halts with `[!]`. No source files were edited.

## Survey — current table columns the bundle would need

Captured here so the operator can compare what (B) ships against
what the bundle serializer will need to read. Sourced from
`crates/codeless-runtime/migrations/*.sql` (0001 initial plus every
`ALTER TABLE` since).

### `jobs` — today (pre-(B)), per `0001_initial.sql` + later ALTERs

| column | type | introduced |
| --- | --- | --- |
| `id` | TEXT PRIMARY KEY | 0001 |
| `repo_id` | TEXT NOT NULL → `repos(id)` | 0001 |
| `status` | TEXT NOT NULL | 0001 |
| `stop_reason` | TEXT | 0001 |
| `template_yaml` | TEXT | 0001 — moves to `runs.template_snapshot` under (B) |
| `prompt` | TEXT | 0001 |
| `runner` | TEXT NOT NULL | 0001 — moves to `runs` under (B) |
| `branch` | TEXT NOT NULL | 0001 — moves to `runs` |
| `worktree_path` | TEXT | 0001 — moves to `runs` |
| `cost_cap_cents` | INTEGER NOT NULL | 0001 — moves to `runs` |
| `wall_clock_cap_ms` | INTEGER NOT NULL | 0001 — moves to `runs` |
| `cost_cents` | INTEGER NOT NULL DEFAULT 0 | 0001 — moves to `runs` |
| `started_at` | INTEGER | 0001 — moves to `runs` |
| `ended_at` | INTEGER | 0001 — moves to `runs` |
| `created_at` | INTEGER NOT NULL | 0001 |
| `model` | TEXT | 0002 |
| `permission_mode` | TEXT | 0002 |
| `effort` | TEXT | 0002 |
| `workspace_mode` | TEXT NOT NULL DEFAULT 'in-repo' | 0004 |
| `system_prompt` | TEXT | 0009 |
| `persona_id` | TEXT → `personas(id)` | 0010 / 0012 (FK promoted in 0012) |
| `auto_bypass_policy` | TEXT | 0014 |
| `pending_operator_comment` | TEXT | 0015 |
| `precheck_override_once` | (per `0016_precheck_override_once.sql`) | 0016 |

Plus the table was recreated in `0012_stage_persona_id_and_jobs_persona_fk.sql`
to promote `persona_id` to an FK. Migration 0018 is intentionally
absent (a numbering skip; verified by `ls`).

Under (B), the bundle's `runs/NNNN/run.json` carries the
attempt-state columns; the bundle's `template.yaml` + `handover.md`
come from the mutable Job row (`jobs.template_yaml`,
`jobs.handover_md` once (B) adds it). The denylist in `SCOPE.md`
(`*token*`, `*secret*`, `*api_key*`) does **not** match any current
column name, but the serializer should still apply it
prospectively so a later migration that adds e.g. `runner_api_key`
is caught by the schema, not by code review.

### `stages` — per `0001_initial.sql` + later ALTERs

| column | type | introduced |
| --- | --- | --- |
| `id` | TEXT PRIMARY KEY | 0001 |
| `job_id` | TEXT NOT NULL → `jobs(id)` | 0001 — becomes `run_id` under (B) |
| `ordinal` | INTEGER NOT NULL | 0001 |
| `name` | TEXT NOT NULL | 0001 |
| `status` | TEXT NOT NULL | 0001 |
| `verify_cmd` | TEXT | 0001 |
| `started_at` | INTEGER | 0001 |
| `ended_at` | INTEGER | 0001 |
| `session_id` | TEXT | 0003 |
| `goal` | TEXT | 0005 |
| `acceptance` | TEXT | 0005 |
| `last_activity_at` | INTEGER | 0006 |
| `archived` | INTEGER NOT NULL DEFAULT 0 | 0006 |
| `persona_id` | TEXT → `personas(id)` | 0012 |
| `bypassed_at` | INTEGER | 0013 |
| `bypassed_reason` | TEXT | 0013 |
| `failure_class` | TEXT | 0017 |
| `failure_detail` | TEXT | 0017 |

### `tasks` — per `0001_initial.sql`

| column | type |
| --- | --- |
| `id` | TEXT PRIMARY KEY |
| `stage_id` | TEXT NOT NULL → `stages(id)` |
| `ordinal` | INTEGER NOT NULL |
| `status` | TEXT NOT NULL |
| `depends_on` | TEXT NOT NULL DEFAULT '[]' |
| `lease_holder` | TEXT (`<pid>:<startup-nonce>`) |
| `lease_expires_at` | INTEGER |
| `cost_cents` | INTEGER NOT NULL DEFAULT 0 |
| `input_tokens` | INTEGER NOT NULL DEFAULT 0 |
| `output_tokens` | INTEGER NOT NULL DEFAULT 0 |
| `started_at` | INTEGER |
| `ended_at` | INTEGER |

Bundle note: `lease_holder` and `lease_expires_at` are runtime state,
not history. The serializer should drop them on export (no point
shipping a stale lease on a `<pid>` that doesn't exist on the
destination) and the importer should write NULL for both.

### `todos` — per `0021_todos.sql` + `0023_todo_failure_detail.sql`

| column | type |
| --- | --- |
| `id` | TEXT PRIMARY KEY |
| `task_id` | TEXT NOT NULL → `tasks(id)` |
| `ordinal` | INTEGER NOT NULL |
| `title` | TEXT NOT NULL |
| `status` | TEXT NOT NULL (`pending`/`in-progress`/`done`/`skipped`/`failed`) |
| `kind` | TEXT NOT NULL (`runner`/`planner`/`checks`/`docs`/`git`) |
| `created_at` | INTEGER NOT NULL |
| `started_at` | INTEGER |
| `ended_at` | INTEGER |
| `failure_detail` | TEXT (0023) |

UNIQUE `(task_id, ordinal)`.

### `reviews` — per `0001_initial.sql`

| column | type |
| --- | --- |
| `id` | TEXT PRIMARY KEY |
| `stage_id` | TEXT NOT NULL → `stages(id)` |
| `status` | TEXT NOT NULL |
| `comment` | TEXT |
| `requested_at` | INTEGER NOT NULL |
| `resolved_at` | INTEGER |

### `events` — per `0001_initial.sql`

| column | type |
| --- | --- |
| `cursor` | INTEGER PRIMARY KEY AUTOINCREMENT (source-workspace monotonic — confirms open Q 4 in `SCOPE.md`) |
| `job_id` | TEXT (nullable) — becomes `run_id` under (B) per JOB-WORKFLOW migration script |
| `stage_id` | TEXT |
| `task_id` | TEXT |
| `type` | TEXT NOT NULL |
| `payload` | TEXT NOT NULL (JSON) |
| `created_at` | INTEGER NOT NULL |

The bundle's per-event `original_id` (per `SCOPE-JOB-EXPORT.md` open
question 6) maps to the source `cursor`. The importer rewrites
`cursor` in the destination's sequence and preserves `created_at`,
`type`, `payload`, and the `(stage_id, task_id)` triplet — but
needs (B)'s migration to be in flight so it can write `run_id`.

### Tables explicitly **out of the bundle**

- `repos`, `attached_workspaces` — bundle records `repo_url` +
  `repo_commit` in the manifest; the destination's repo row is
  authoritative.
- `pty_sessions` — runtime state on the source host; not history.
- `assistant_*` (0008, 0019, 0020, 0028) — the in-app Assistant
  thread / messages / attachments belong to the user, not the Job.
  Scope doc carries no mention of including them.
- `personas` (0011) — `jobs.persona_id` and `stages.persona_id` are
  exported as opaque IDs; the destination must already have the
  same persona row or import will surface a warning. Out of scope
  for E1.
- `scheduled_pause_points` (0022) — Job-scoped pause schedule;
  arguably should be exported. Logged as a follow-up question for
  stage 2.
- `chat_messages` (0024) / `chat_bindings` (0025) /
  `supervisor_goals` (0026) / `chat_adapters` (0027) /
  `runner_config` (0027) — workspace-level surfaces, not Job
  history.

## What stage 2 must do once (B) lands

1. Re-survey the schema: (B) will introduce `runs`, rename
   `stages.job_id` → `stages.run_id`, and re-key events. The column
   inventory above becomes stale.
2. Lock the manifest JSON shape and bundle directory layout against
   `DOCS/SCOPE-JOB-EXPORT.md` §"What's in the bundle".
3. Resolve `SCOPE.md` open questions 2 (size cap defaults),
   3 (`output_path` jailing), 4 (confirm `events.cursor` monotonicity
   under (B)), 5 (handover lands on `jobs.handover_md`).
4. Lock the secrets denylist regex set and the column-name walk.
5. Write the stage-3 REVIEW packet per `WORKFLOW.md`.

## Verify

No source diff. `cargo` checks not run (no code touched). Per
`WORKFLOW.md` closing trio for a read-only stage: handover + session
doc committed only.

## Stage 2 — design lock  `[x]`

Stage 2 ("design bundle layout and manifest.json schema_version 1
against DOCS/SCOPE-JOB-EXPORT.md; lock the secrets denylist and
per-entry size caps") is paper-only and can land independently of
the (B) precondition that gates implementation stages 4–7.

### Re-verification of the (B) blocker

Re-grepped this stage: `crates/codeless-runtime/migrations/` still
has no `runs` migration; `jobs` still owns the per-attempt columns;
no `handover_md` on `jobs`. Stage 1's halt stands; the design below
targets the post-(B) world per `DOCS/SCOPE-JOB-EXPORT.md`.

### Deliverable

[`.codeless/jobs/job-export/BUNDLE-DESIGN.md`](../../.codeless/jobs/job-export/BUNDLE-DESIGN.md)
— the frozen design lock that stage 3 REVIEW reads and stages 4–7
build against. Covers in ten sections:

1. Bundle file shape (gzipped tar, `.codeless-job`, UTF-8,
   deterministic ordering, mtime-zeroed gzip header).
2. Directory layout — every allowed path, every rejected entry
   shape, ordinal-dir zero-padding, top-level dir stripping.
3. `manifest.json` schema_version 1 — every key typed, required,
   `deny_unknown_fields`, the importer's first read.
4. Per-Run `run.json` field set (with `worktree_path_source`
   rename so destination can't accidentally reuse it) and the
   five JSONL streams' sort orders + drop rules (`lease_holder`
   etc. dropped on export).
5. Secrets denylist (case-insensitive substring regex set:
   `token`, `secret`, `api[_-]?key`, `pass(word|wd)`,
   `private[_-]?key`, `credential`, `bearer`, `auth[_-]?header`).
   Confirmed: zero current columns match.
6. Size caps — 200 MiB per bundle, 10 MiB per entry, plus per-kind
   sub-caps (64 KiB manifest / README, 1 MiB template / handover /
   note / run.json / JSONL line, 1024 runs/bundle, 500k
   events/run); constants live in `limits.rs`; importer enforces
   streaming.
7. Open question resolutions — OQ-1 through OQ-5 from the scope
   doc plus OQ-D (event payload scanning: out for E1) and OQ-E
   (`scheduled_pause_points`: out for E1).
8. Refuse-to-export preconditions (non-terminal Run, empty repo
   URL, output path outside workspace `fs_root_canonical`, any cap
   exceeded).
9. README cover note outline.
10. What stages 3–7 inherit + what's deliberately not locked
    (tar/gzip lib choice, exact error variant names).

### Locked open questions, one-liner each

- OQ-2 (size caps): values per §5 of the design doc.
- OQ-3 (output path): jailed under workspace `fs_root_canonical`
  via canonicalised prefix check.
- OQ-4 (events.cursor monotonicity): confirmed by SQLite
  AUTOINCREMENT; (B) re-keys `job_id → run_id` only.
- OQ-5 (handover lands on `jobs.handover_md`): confirmed; first
  new Run snapshots like any other.
- OQ-D (payload scanning): out for E1; README warns.
- OQ-E (`scheduled_pause_points`): out for E1; logged for E2.

### Verify

No source diff. New file:
`.codeless/jobs/job-export/BUNDLE-DESIGN.md`. `cargo` checks not
run (no code touched). Commit: handover + session doc + design doc
only.

## Stage 4 — walker + serializer  `[!]` halted on (B)

Stage 4 is "implement codeless-runtime job_export walker plus
serializer that emits manifest, JSONL streams, and the gzipped tar
to a server-side path."

This is the first **implementation** stage. Per the job's own
[`WORKFLOW.md`](../../.codeless/jobs/job-export/WORKFLOW.md#precondition-check-stage-1)
and stage 1's halt finding, implementation stages 4–7 cannot land
until JOB-WORKFLOW (B) — the Job/Run split — is merged. The bundle
layout (`runs/NNNN/run.json` with `template_snapshot` /
`handover_snapshot`, per-Run JSONL streams keyed by an immutable
`runs.id`, `events.run_id` re-key) is built directly on the schema
(B) ships.

### Re-verification this stage

Repeated the stage-1 grep:

```
$ ls crates/codeless-runtime/migrations/
0001…0028 (no 0018), no migration adds a `runs` table.

$ grep -li "CREATE TABLE.*runs\b\|template_snapshot\|handover_snapshot\|handover_md" \
    crates/codeless-runtime/migrations/*.sql
(no matches)
```

`jobs` still owns `template_yaml`, `runner`, `branch`,
`worktree_path`, `cost_cap_cents`, `wall_clock_cap_ms`, `cost_cents`,
`started_at`, `ended_at`. There is no `runs` table, no
`runs.template_snapshot`, no `runs.handover_snapshot`, no
`jobs.handover_md`, no `events.run_id`. The state of the world has
not changed since stage 1.

### Why this stage cannot scaffold around (B)

Three concrete reasons, each enough on its own:

1. **`runs/NNNN/run.json` has no source row.** The locked schema
   (BUNDLE-DESIGN §3 "Per-Run `run.json` shape") requires `id`,
   `ordinal`, `template_snapshot`, `handover_snapshot`,
   `resumed_from_stage`, `created_at` — none of which exist on the
   current `jobs` row. Synthesising one Run row per Job at export
   time would mean inventing `template_snapshot` and
   `handover_snapshot` content that the post-(B) world will not
   produce the same way (it snapshots at Run-start; we have no
   Run-start). The importer in a post-(B) world would then have to
   distinguish "real (B)-era bundles" from "stage-4 scaffolded
   bundles" — the exact translation layer `WORKFLOW.md` forbids.

2. **`events.jsonl` cannot key by Run.** Today `events.job_id` is
   nullable; under (B) it becomes `events.run_id`. The bundle's
   per-Run `events.jsonl` stream is "every event for this Run." Pre-(B)
   the runtime has no `run_id` to filter on, so the stream would
   collapse to "every event for this Job" inside the first (and
   only) run directory — which is a different shape than the locked
   design. Stage 5's importer is built to read the locked shape.

3. **WORKFLOW.md explicit rule.** Quoting it verbatim: *"Do not try
   to scaffold around it; the bundle layout depends on immutable
   Run rows."* This is the contract the user wrote when they
   scaffolded this job; stage 4 must honour it.

R4 in the repo `CLAUDE.md` ("no half-finished implementations […]
mark it `[!]` in the active session doc and halt") makes the call
unambiguous: halt with `[!]`, do not commit a partial implementation
with a TODO. No `job_export/` module is created this stage.

### What stage 4 will do once (B) lands

For the next agent that picks this up after (B) merges:

1. Re-survey the post-(B) schema: confirm the `runs` table column
   set matches `DOCS/JOB-WORKFLOW.md` §(B); confirm
   `events.run_id` re-key; confirm `jobs.handover_md` exists.
2. Create `crates/codeless-runtime/src/job_export/` with one file
   per concept per R3:
   - `mod.rs` — re-exports + the `export_job` entrypoint signature
     matching the wire type in `codeless-rpc`.
   - `limits.rs` — every `pub const` from BUNDLE-DESIGN §5 with a
     `#[cfg(test)]` override hook.
   - `denylist.rs` — the case-insensitive substring regex set from
     BUNDLE-DESIGN §4, plus a `column_allowed(name: &str) -> bool`
     helper used by every JSONL serializer.
   - `manifest.rs` — the `Manifest`, `Exporter`, `Source`,
     `Content` structs (serde + `deny_unknown_fields` on the import
     side; emit-order locked per BUNDLE-DESIGN §3).
   - `walker.rs` — the SQLite walk: one query per table, ordered by
     the keys BUNDLE-DESIGN §3 locks (`stages.ordinal`,
     `(stage_ordinal, task_ordinal)`, `(task_id, ordinal)`,
     `events.cursor`, `(reviews.requested_at, id)`). Streams rows
     out as iterators; never collects whole tables in memory.
   - `serializer.rs` — wraps the walker iterators into the
     `manifest.json`, `template.yaml`, `handover.md`, `notes/*.md`,
     `runs/NNNN/run.json`, and five JSONL files. Owns the
     `worktree_path` → `worktree_path_source` rename and the
     `lease_holder` / `lease_expires_at` drop.
   - `tar_writer.rs` — gzipped tar emit with mtime-zeroed gzip
     header and deterministic entry order per BUNDLE-DESIGN §1.
     Enforces `MAX_BUNDLE_BYTES` and `MAX_ENTRY_BYTES` as it writes
     (refuses to finalize once exceeded — does not write a truncated
     bundle).
   - Each file ships unit tests in the same commit per R5. Tests use
     tiny `#[cfg(test)]` cap overrides from `limits.rs`.
3. Wire the OQ-3 jail check on `output_path` at the entrypoint:
   canonicalise, prefix-check against the workspace's
   `fs_root_canonical`, refuse with
   `OutputPathOutsideWorkspace { fs_root, output_path }`.
4. Wire the refuse-to-export preconditions from BUNDLE-DESIGN §7:
   non-terminal Run, empty `repos.url`, output path jail, cap
   exceed.
5. `cargo test --workspace` green, `cargo clippy --workspace
   --all-targets -- -D warnings` green, `cargo fmt --check` green.
6. Hand over to stage 5 (importer).

### Verify

No source diff. `cargo` checks not run (no code touched). Commit:
session doc update + handover only, mirroring stage 1's halt commit
shape.

## Stage 5 — importer  `[!]` halted on (B) + stage-4 chain

Stage 5 is "implement importer with tar-streaming, path-traversal
guards, batched SQLite inserts in one transaction, and
ImportConflictPolicy (Refuse default)."

This is the second implementation stage. It depends transitively on
JOB-WORKFLOW (B) *and* directly on the stage-4 walker/serializer
module that has not yet been written. Both preconditions are still
unmet this session.

### Re-verification this stage

```
$ ls crates/codeless-runtime/migrations/ | wc -l
27   # 0001..0028 with 0018 skipped — unchanged

$ rg -l 'CREATE TABLE\s+runs\b|template_snapshot|handover_snapshot|handover_md' \
      crates/codeless-runtime/migrations/
(no matches)

$ ls crates/codeless-runtime/src/job_export 2>&1
ls: cannot access 'crates/codeless-runtime/src/job_export': No such file or directory
```

There is no `runs` table, no `events.run_id`, no `jobs.handover_md`,
and no `codeless-runtime/src/job_export/` module. Stage 4 (the
walker + serializer) shipped a halt commit on its own pass and has
not been re-run since (B) is still not in flight). Git log between
stage-4-halt and now:

```
$ git log --oneline 27dca3d..HEAD
92e844c stage 3: implement codeless-runtime job_export walker plus serializer ...
```

That `stage 3:` commit (`92e844c`) is **session-doc/handover-only**
(`git show --stat` lists one file, the prior handover). No source
landed; the loop runner appears to have re-fired stage 4 under the
wrong title. The walker, serializer, manifest structs, denylist
helper, tar writer, and `export_job` entrypoint described in the
stage-2 design lock (`BUNDLE-DESIGN.md`) and the stage-4 plan above
still do not exist on disk.

### Why this stage cannot land

The importer reads the bundle that stage 4 emits and writes into the
post-(B) schema. Three concrete blockers, any one fatal:

1. **No JSONL contract to parse.** The locked `run.json` /
   `stages.jsonl` / `tasks.jsonl` / `todos.jsonl` / `events.jsonl` /
   `reviews.jsonl` shapes live in `BUNDLE-DESIGN.md` as a paper
   spec. Stage 4 was supposed to commit them as serde structs in
   `crates/codeless-runtime/src/job_export/manifest.rs` +
   `serializer.rs`. The importer's first responsibility is to
   deserialize those exact structs with `deny_unknown_fields`; with
   no struct definitions in tree, writing the importer either (a)
   duplicates the spec inline — and bakes a *second* source of
   truth that drifts from the eventual serializer, violating R4 —
   or (b) emits empty parser stubs that read nothing, which is the
   "half-finished implementation with TODO" R4 explicitly forbids.
2. **No destination table to insert into.** `ImportJobArgs` expects
   to write Run rows with `template_snapshot` / `handover_snapshot`
   and event rows keyed by `run_id`. The destination schema today
   has neither. A batched insert transaction over `runs` /
   `events.run_id` cannot be written against a schema that lacks
   those targets; faking it against the pre-(B) `jobs` row would
   bind the importer to a column set (B) is about to delete.
3. **No `inspect_job_bundle` companion.** BUNDLE-DESIGN §6 and
   `SCOPE-JOB-EXPORT.md` §"RPC surface" require manifest validation
   (size caps, schema_version check) to land before any row writes.
   That validation reuses the `Manifest` struct from stage 4. Same
   missing dependency.

`WORKFLOW.md` §"Precondition check (stage 1)" — *"Do not try to
scaffold around it; the bundle layout depends on immutable Run
rows."* — and repo `CLAUDE.md` R4 — *"If you cannot complete a
stage, mark it `[!]` in the active session doc and halt. Do not
commit a partial implementation with a TODO."* — together force the
halt. No `importer.rs` / `tar_safety.rs` files were created this
stage.

### What stage 5 will do once stage 4 has landed (post-(B))

For the agent that picks this up after the walker + serializer are
on disk:

1. Re-verify: (a) `runs` table exists with the BUNDLE-DESIGN §3
   column set; (b) `events.run_id` is wired; (c) `jobs.handover_md`
   exists; (d) `crates/codeless-runtime/src/job_export/{manifest,
   walker, serializer, tar_writer, limits, denylist}.rs` all exist
   and `cargo test --workspace -p codeless-runtime` is green.
2. Add to `crates/codeless-runtime/src/job_export/`:
   - `tar_safety.rs` — the path-traversal guard used by every tar
     entry. Rejects: absolute paths (`Path::is_absolute`), any
     component equal to `..` or `.`, any non-file-non-dir entry kind
     (symlink / hardlink / device / fifo), entries whose
     `tar::Entry::path()` resolves outside the expected fixed set
     `{manifest.json, README.md, template.yaml, handover.md,
     notes/<basename>.md, runs/<4-digit-ordinal>/run.json,
     runs/<4-digit-ordinal>/stages.jsonl, …reviews.jsonl,
     runs/<4-digit-ordinal>/artifacts/<basename>}`. Pattern is a
     strict allow-list, not a deny-list. Unit-tested per R5 with
     hostile fixtures: absolute-path entry, `..` parent escape,
     symlink, long-name overflow, NUL byte, mixed-case dupes on
     case-insensitive FS.
   - `inspect.rs` — the `inspect_job_bundle` entrypoint: open the
     gzip stream, read `manifest.json` *only* (skip past it without
     decoding subsequent entries), validate `schema_version == 1`,
     enforce `MAX_BUNDLE_BYTES` against the file size on disk before
     opening, enforce `MAX_MANIFEST_BYTES` while reading. Returns
     `Manifest` + the on-disk byte count + computed warnings (repo
     SHA mismatch deferred to importer where workspace context is
     known). Never writes to SQLite, never reads past the manifest
     entry. Unit-tested with: valid manifest, wrong schema_version,
     oversize manifest, truncated gzip, manifest not first entry.
   - `importer.rs` — the `import_job` entrypoint per
     `SCOPE-JOB-EXPORT.md` §"RPC surface". Flow:
     a. Run `inspect_job_bundle` to validate the manifest and enforce
        bundle-level caps.
     b. Resolve the destination workspace by `WorkspaceId`; refuse if
        not attached.
     c. Resolve `imported_name` from `rename_to ??
        manifest.source.job_name`. Look up an existing Job by
        `(workspace_id, name)` and apply `ImportConflictPolicy`. For
        E1 only `Refuse` is wired — `Suffix` and `Replace` return
        `ImportError::UnsupportedConflictPolicy { policy }`. Default
        in the RPC arg type is `Refuse` (per E1 scope: "single
        workspace, Refuse default").
     d. Stream-decode the tar (`tar::Archive::entries`) — never
        materialise the whole bundle in RAM, never `Archive::unpack`.
        For each entry: `tar_safety::validate` → match the expected
        path → buffer up to the per-kind cap from `limits.rs` →
        deserialize the per-line JSON into the stage-4 struct.
        Enforce `MAX_ENTRY_BYTES`, the per-kind sub-caps, the
        `MAX_RUNS_PER_BUNDLE` / `MAX_EVENTS_PER_RUN` counters.
     e. Open one SQLite transaction
        (`sqlx::SqliteConnection::begin`). Insert in dependency
        order: Job row → Runs → Stages → Tasks → Todos → Reviews →
        Events. Use `INSERT ... ` against fresh ULIDs generated by
        `ulid::Ulid::new()` (R6 — never reuse the source IDs). Keep
        a `HashMap<SourceId, DestId>` per table to rewrite foreign
        keys as later rows insert. Batched: `INSERT INTO events ...
        VALUES (?, ?, ..), (?, ?, ..), ...` chunked at 500 rows per
        statement to stay under SQLite's 999-parameter default
        without bumping `PRAGMA max_compound_select`.
     f. Write the user-facing files (`.codeless/jobs/<name>.yaml`,
        `handover.md`, `notes/*.md`) under the workspace's
        `fs_root_canonical`. Re-use the tar-safety jail logic so a
        bundle that names `notes/../../etc/passwd` can't escape.
     g. Drop `lease_holder` / `lease_expires_at` (write NULL).
        Rewrite `worktree_path` in `run.json` — already renamed to
        `worktree_path_source` by stage 4 — to NULL on the
        destination side; the next Run will allocate its own.
     h. Commit the transaction; on any failure inside the closure,
        SQLx rolls back and the importer surfaces an `ImportError`
        with the offending entry path. Files written to the worktree
        before the failure are cleaned up by the rollback closure.
     i. Build `ImportJobResult { job_id, imported_name, run_count,
        warnings }`. Warnings include: source repo SHA mismatch
        (compared against the destination's currently-attached repo
        HEAD), excluded artifacts referenced by reviews, note
        filename collisions (suffixed `-imported`).
   - Wire `ImportError` variants in `codeless-rpc` per the scope doc
     (`JobNameExists { existing_job_id }`, `SchemaVersionMismatch`,
     `BundleTooLarge`, `EntryTooLarge`, `PathTraversal { entry }`,
     `WorkspaceNotAttached`, `UnsupportedConflictPolicy`).
3. Tests live with the code per R5:
   - `tar_safety` unit tests (hostile fixtures, above).
   - `inspect` unit tests (above).
   - `importer` unit tests: refuse-on-name-collision, schema_version
     mismatch, oversize bundle, malformed JSONL line, FK rewrite
     correctness across Stage → Task → Todo → Event chain.
   - Round-trip integration test reserved for stage 7 (per
     WORKFLOW.md); stage 5 ships unit + small-bundle integration.
4. Crate boundary check (R1): `tar`, `flate2`, `sqlx`, `ulid` all
   stay in `codeless-runtime`. Nothing leaks into `codeless-rpc` /
   `-types` / `-client`. The wire types from
   `codeless-rpc::methods` (`ImportJobArgs`, `ImportJobResult`,
   `ImportConflictPolicy`, `ImportWarning`) are reused verbatim;
   the runtime owns the implementation.
5. `cargo test --workspace` green, `cargo clippy --workspace
   --all-targets -- -D warnings` green, `cargo fmt --check` green.
   Commit via `./bin/mani --config mani.yaml run commit/push
   --projects codeless` from the workspace root.
6. Hand over to stage 6 (RPC wiring).

### Verify

No source diff this stage. `cargo` checks not run (no code touched).
Commit: session doc update + handover only, mirroring stages 1, 4's
halt commit shape. The `stage 3: …` commit between this and the
prior halt is doc-only and does not change the precondition state.

## Stage 6 — RPC wiring  `[!]` halted on (B) + stage-4/5 chain

Stage 6 is "wire `export_job`, `import_job`, `inspect_job_bundle`
RPCs in `codeless-rpc` and `codeless-runtime`; refuse export of any
Job with a non-terminal Run."

### Re-verification this stage

```
$ ls crates/codeless-runtime/migrations/ | wc -l
27   # 0001..0028 with 0018 skipped — unchanged

$ ls crates/codeless-runtime/src/job_export 2>&1
ls: cannot access 'crates/codeless-runtime/src/job_export': No such file or directory

$ grep -n "ExportJob\|ImportJob\|InspectJobBundle\|export_job\|import_job\|inspect_job_bundle" \
       crates/codeless-rpc/src/methods.rs
(no matches)

$ git log --oneline ef9499f..HEAD
a579c7e stage 4: implement importer with tar-streaming, path-traversal guards, ...
```

That `stage 4:` commit (`a579c7e`) between the prior halt and now
is — like `92e844c` (`stage 3:`) before it — session-doc /
handover-only. `git show --stat a579c7e` lists one file (a
handover); no `importer.rs` / `tar_safety.rs` / `inspect.rs` exist.
The state of the world has not changed since stages 1, 4, and 5
halted: no `runs` table, no `events.run_id`, no `jobs.handover_md`,
no `crates/codeless-runtime/src/job_export/` module, no wire types
in `codeless-rpc`.

### Why this stage cannot land

Stage 6 is the seam that exposes the export/import/inspect surface
to UI and CLI. Every piece of it consumes something that does not
yet exist on disk:

1. **No runtime entrypoints to dispatch into.** The stage-2 design
   commit named three runtime fns (`export_job`, `import_job`,
   `inspect_job_bundle`) living under
   `crates/codeless-runtime/src/job_export/`. The directory does
   not exist; the fns are not declared. `RpcServer` cannot route a
   method to a fn that has no body, and `WORKFLOW.md` §"Anti-
   patterns" forbids "empty parser stubs" (R4 — "no half-finished
   implementations").
2. **No wire types in `codeless-rpc`.** A grep across
   `codeless-rpc/src/` returns zero hits for the
   `ExportJobArgs` / `ExportJobResult` / `ImportJobArgs` /
   `ImportJobResult` / `InspectJobBundleArgs` /
   `InspectJobBundleResult` / `ImportConflictPolicy` /
   `ImportWarning` / `ImportError` names that `BUNDLE-DESIGN.md`
   §§3, 6 and `SCOPE-JOB-EXPORT.md` §"RPC surface" lock. Stage 2's
   commit (`2840f81`) is session-doc-only as well — the wire types
   were never landed. Adding them this stage would (a) duplicate
   the BUNDLE-DESIGN spec into a second source of truth that drifts
   from the eventual stage-4 serializer, and (b) emit `specta`
   bindings into the TS bundle that point at runtime methods which
   don't exist — UI work (stages 9–10) cannot use them, and any
   client that calls them gets `MethodNotFound` at runtime.
3. **The "refuse export of any Job with a non-terminal Run"
   precondition has no concept of a Run to check.** This stage's
   own scope sentence requires inspecting `runs.status` for a
   terminal value before allowing export. Pre-(B) there is no
   `runs` table — there's one implicit Run per Job collapsed into
   `jobs.status`. Faking the check against `jobs.status` (a) writes
   a check that the post-(B) world will silently delete, and (b)
   bundles the Job's current mutable in-flight state as "history,"
   which is the exact failure mode BUNDLE-DESIGN.md §7 and
   SCOPE-JOB-EXPORT.md §"Refuse-to-export" forbid.
4. **`RpcServer` dispatch needs the importer to surface
   `ImportError` variants.** Stage 5's plan lists the variant set
   (`JobNameExists`, `SchemaVersionMismatch`, `BundleTooLarge`,
   `EntryTooLarge`, `PathTraversal`, `WorkspaceNotAttached`,
   `UnsupportedConflictPolicy`). With no importer module those
   variants don't exist; routing the RPC to a `todo!()` body or a
   single catch-all error is the R4-forbidden TODO-shaped commit.

`WORKFLOW.md` §"Sequencing" — *"Stages 4–7 are the server-side
core: walker, importer, RPCs, tests. Each ends with `cargo test
--workspace` green plus `cargo clippy --workspace --all-targets --
-D warnings` green."* — makes the ordering explicit: RPCs come
after the walker and importer they wire. Skipping ahead would
either land green-but-empty methods (R4 violation) or land methods
whose green tests assert behaviour we've decided is wrong under
(B) (a `WORKFLOW.md` §"Anti-patterns specific to this job"
violation: "no silent column drops on import" — same family of
mistake).

### What stage 6 will do once stages 4 + 5 + (B) are in place

For the agent that picks this up after the preconditions clear:

1. Re-verify: (a) `runs` table + `events.run_id` + `jobs.handover_md`
   are live in migrations; (b) `crates/codeless-runtime/src/job_export/`
   contains `manifest.rs`, `walker.rs`, `serializer.rs`,
   `tar_writer.rs`, `limits.rs`, `denylist.rs`, `tar_safety.rs`,
   `inspect.rs`, `importer.rs` per the stage 4 + 5 plans, with
   `cargo test --workspace -p codeless-runtime` green.
2. Add to `crates/codeless-rpc/src/methods.rs` (one type per file
   if size justifies it under R3; otherwise grouped behind
   `// region:` markers consistent with the existing file):
   - `ExportJobArgs { job_id: JobId, output_path: PathBuf,
     include_artifacts: bool }` — `output_path` is server-side
     (the browser shell reads it via `fs.read_file` per
     WORKFLOW.md §"Anti-patterns": *"No new transport for the file
     bytes."*).
   - `ExportJobResult { bundle_path: PathBuf, bundle_bytes: u64,
     manifest: Manifest, warnings: Vec<ExportWarning> }`.
   - `ExportError` enum with `NonTerminalRun { run_id, status }`,
     `OutputPathOutsideWorkspace { fs_root, output_path }`,
     `EmptyRepoUrl`, `BundleTooLarge { limit, actual }`,
     `EntryTooLarge { entry, limit, actual }`, `RepoNotAttached`,
     plus the `IoError(String)` / `DbError(String)` catch-alls
     consistent with the rest of `codeless-rpc::error`.
   - `ImportJobArgs { workspace_id, bundle_path,
     conflict_policy: ImportConflictPolicy, rename_to:
     Option<String> }` with `ImportConflictPolicy { Refuse,
     Suffix, Replace }` and `Default` returning `Refuse` per
     SCOPE-JOB-EXPORT.md §"E1 scope".
   - `ImportJobResult { job_id, imported_name, run_count,
     warnings: Vec<ImportWarning> }`.
   - `ImportError` per stage 5's plan.
   - `InspectJobBundleArgs { bundle_path }` + `InspectJobBundleResult
     { manifest, bundle_bytes, warnings }` (read-only — never writes
     SQLite).
   - All structs derive `serde::Serialize`, `serde::Deserialize`,
     `Clone`, `Debug`, `specta::Type`. `deny_unknown_fields` on
     every `Deserialize`. Regenerate the TS bundle in the same
     commit so stages 9–10 can import.
3. Add three trait methods to `RpcServer` in `codeless-rpc/src/server.rs`:
   `export_job(&self, ExportJobArgs) -> Result<ExportJobResult,
   ExportError>`, `import_job(&self, ImportJobArgs) ->
   Result<ImportJobResult, ImportError>`, `inspect_job_bundle(&self,
   InspectJobBundleArgs) -> Result<InspectJobBundleResult,
   InspectError>`. Add matching mock impls to `MockRpc` (or
   whatever the project's `RpcClient` test double is named — check
   `codeless-rpc/src/server.rs` at re-fire time).
4. In `codeless-runtime`, add `impl RpcServer for Runtime` (or
   extend the existing impl) with thin wrappers that:
   - For `export_job`: open a read txn, look up
     `runs WHERE job_id = ? ORDER BY ordinal DESC LIMIT 1`,
     refuse if `status NOT IN ('succeeded','failed','cancelled',
     'aborted')` — exact terminal set comes from the post-(B)
     `runs.status` enum; check `JOB-WORKFLOW.md` §(B) at re-fire
     time. Then delegate to
     `crates::codeless_runtime::job_export::export_job(...)` and
     map errors. The "refuse non-terminal" check lives **here** in
     the RPC seam, not in the walker, because the walker's
     contract is "given a Job + Run set, emit bytes"; gating
     belongs at the entrypoint per R4 (no helpers introduced
     speculatively in the walker for a single caller).
   - For `import_job`: resolve `WorkspaceId`, then delegate to
     `crates::codeless_runtime::job_export::import_job(...)` from
     stage 5.
   - For `inspect_job_bundle`: delegate to
     `crates::codeless_runtime::job_export::inspect(...)`. Never
     touches SQLite.
5. Wire `ExportError` / `ImportError` / `InspectError` into
   `codeless-rpc/src/error.rs`'s `RpcError` envelope so SSE clients
   see structured codes (existing pattern: see how
   `template::ValidationError` is wrapped — match it).
6. Tests live with the code per R5:
   - `codeless-rpc/tests/*.rs`: serde round-trip for every arg /
     result / error variant; `deny_unknown_fields` rejection
     fixtures.
   - `codeless-runtime/tests/job_export_rpc.rs`: per-RPC test —
     `export_job` against a fixture Job with one terminal Run
     succeeds, against a Job with a non-terminal Run returns
     `NonTerminalRun`; `inspect_job_bundle` against the produced
     bundle returns the same `Manifest`; `import_job` round-trip
     deferred to stage 7 (per `template.yaml`).
7. `cargo test --workspace` green; `cargo clippy --workspace
   --all-targets -- -D warnings` green; `cargo fmt --check` green.
   Commit via `./bin/mani --config mani.yaml run commit/push
   --projects codeless` from the workspace root once mani is
   reachable; otherwise raw `git` matching the halt-commit shape.
8. Hand over to stage 7 (round-trip property test + tar-safety
   units).

### Verify

No source diff this stage. `cargo` checks not run (no code
touched). Commit: session doc update + handover only, mirroring
the halt-commit shape of stages 1, 4, and 5.

### Open questions specific to this halt

- The loop runner has now fired five halt-shaped stages (1, 4, 5,
  and two duplicate-titled "3" / "4" doc-only commits) against the
  same unresolved (B) precondition. Operator may want to gate
  stage firing on `[ -d crates/codeless-runtime/src/job_export ]`
  or on the migrations grep until (B) lands; each fire burns a
  session for no work.
- Stages 7 (round-trip tests), 8 (REVIEW), and 9–10 (UI) are all
  transitively blocked on the same chain. Until (B) and stages 4 +
  5 land, this job has zero forward motion.

## Stage 7 — round-trip property test + tar-safety units  `[!]` halted on same chain

Stage 7 is "add round-trip property test (export then import into
scratch workspace, assert row body equality and ordering) and
tar-safety unit tests."

### Re-verification

- `ls crates/codeless-runtime/src/job_export` → does not exist.
- `ls crates/codeless-runtime/migrations/ | grep -i run` → returns
  only `0002_job_runner_overrides.sql`. No `runs` table migration.
- `grep -rl "ExportJob\|ImportJob\|InspectJobBundle"
  crates/codeless-rpc/src` → zero matches.

The world has not moved since the stage 6 halt at `8b90389`. The
four preceding halts (stages 1, 4, 5, 6) all stopped on the same
unresolved JOB-WORKFLOW (B) precondition plus the missing
stage-4/5/6 source chain.

### Why this stage cannot land

Stage 7 writes tests against four things that do not exist:

1. **`export_job` runtime fn** — would be the input to the
   round-trip property test (export from fixture workspace, get
   bytes). Stage 4 was supposed to produce it; no
   `walker.rs` / `serializer.rs` / `manifest.rs` /
   `tar_writer.rs` on disk.
2. **`import_job` runtime fn** — would consume the bundle into a
   scratch SQLite + worktree. Stage 5 was supposed to produce it;
   no `importer.rs` on disk.
3. **`tar_safety` module** — the unit-under-test for the
   tar-safety units (absolute path / `..` segment / symlink /
   off-layout entry / oversize entry rejections). No
   `tar_safety.rs` on disk.
4. **Wire types in `codeless-rpc`** — the property test's
   pre/post comparison reads row bodies via the same shape the
   importer writes; without `ExportJobResult` /
   `ImportJobResult`, the assertion surface is undefined.

Beyond the missing modules, the row-body equality assertion the
stage calls for compares `runs` and `events.run_id` rows on both
sides of the round trip. Without (B), neither side has a `runs`
table; the assertion has no column set to compare. Writing the
test against the pre-(B) `jobs` shape produces a test that the
post-(B) world deletes and re-authors — the same trap repo
`CLAUDE.md` R4 forbids.

Per `WORKFLOW.md` §"Precondition check" and repo `CLAUDE.md` R4
("no half-finished implementations" — and no half-finished tests
either), halt with `[!]`. No source files were edited.

### What lands when stage 7 is re-fired

Re-fire happens only after (B) merges and stages 4 + 5 + 6 land
real source. At that point the stage adds, in one commit:

1. `crates/codeless-runtime/tests/job_export_roundtrip.rs` — a
   `proptest!` block that generates a fixture Job + 1–4 Runs with
   varying Stage / Task / Todo / Event / Review counts, calls
   `export_job` to a `tempfile::TempDir`, opens a fresh in-memory
   SQLite + scratch `WorkspaceId`, calls `import_job`, then
   asserts row-body equality on the immutable columns of every
   exported table plus ordering on `(run.ordinal, stage.ordinal,
   task.ordinal, todo.ordinal)` and `events.cursor` (re-keyed on
   destination but ordered identically to source). Drop the
   runtime-state columns the serializer skips
   (`tasks.lease_holder`, `tasks.lease_expires_at`).
2. Unit tests beside `crates/codeless-runtime/src/job_export/tar_safety.rs`
   — one rejection test per guard listed in
   `.codeless/jobs/job-export/SCOPE.md` §"Importer guards":
   absolute path (`/etc/passwd`), `..` segment, symlink entry,
   entry outside the `manifest.json` / `template.yaml` /
   `handover.md` / `notes/` / `runs/NNNN/` layout, per-entry size
   cap exceeded, total uncompressed size cap exceeded. Each test
   constructs a minimal tar in-memory via `tar::Builder`, feeds it
   through the safety check, and asserts the exact
   `ImportError::TarSafety { kind, path }` variant.
3. Closing trio: `cargo test --workspace` green, `cargo clippy
   --workspace --all-targets -- -D warnings` green, `cargo fmt
   --check` green. Commit via `./bin/mani --config mani.yaml run
   commit/push --projects codeless` from the workspace root.

### Verify

No source diff this stage. `cargo` checks not run (no code
touched). Commit: session doc update + handover only, matching
the halt-commit shape of stages 1, 4, 5, and 6.

### Open questions specific to this halt

- Five consecutive stages (1, 4, 5, 6, 7) have now halted on the
  same chain. The loop's "burn a session per stage" cost is now
  fixed overhead until the operator gates firing on `[ -d
  crates/codeless-runtime/src/job_export ]` or on the (B)
  migrations grep. Reiterating from prior halts; the cost
  compounds.
- Stage 8 (mid-job REVIEW) is the next gate. Firing it against
  the current tree gives the user nothing to review — no
  manifest output, no test transcript, no tar-safety results —
  and will itself halt.
