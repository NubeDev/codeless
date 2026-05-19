# BUNDLE-DESIGN — `.codeless-job` bundle layout, manifest, denylist, size caps

Stage 2 deliverable for the `job-export` job. The authoritative
narrative lives in
[`DOCS/SCOPE-JOB-EXPORT.md`](../../../DOCS/SCOPE-JOB-EXPORT.md); this
file is the **frozen design lock** for the implementation stages 4–7
to build against and for the stage-3 REVIEW gate to approve.

If this file contradicts `DOCS/SCOPE-JOB-EXPORT.md`, that doc wins —
fix this file rather than diverge.

Precondition status (carried forward from stage 1): JOB-WORKFLOW (B)
is **still not merged**. This design is locked against the post-(B)
world per the scope doc. Implementation stages remain blocked until
(B) lands; the design itself does not.

## 1. Bundle file shape

- **Container.** Single file, gzipped tar (`tar` + `gzip` defaults,
  no `xz`, no zip). Magic bytes are the gzip header; the importer
  sniffs `1f 8b` before opening.
- **Extension.** `.codeless-job`. Pickers in the UI filter on this
  suffix; the importer accepts any path but logs a warning if the
  suffix differs.
- **Filename convention** (recommended, not enforced):
  `<job_name>-<source_workspace>-<exported_at_yyyymmdd>.codeless-job`.
  The exporter writes whatever `output_path` the caller passes; the
  UI fills in this default in the save dialog.
- **Encoding.** UTF-8 throughout. All JSON / JSONL is UTF-8 with LF
  line endings. No BOM. The exporter rejects any source string that
  fails `String::from_utf8` (none should — SQLite TEXT is already
  UTF-8) and surfaces the offending column / row id.
- **Determinism.** Within one export, ordering is fixed: entries are
  written in the order listed in §2 below; JSONL rows are sorted by
  the natural key (`ordinal` then `id`); the `manifest.json` keys are
  emitted in the order shown in §3. Two exports of the same Job at
  the same point-in-time produce byte-identical tar contents (the
  gzip header's mtime is zeroed for reproducibility; outer file mtime
  comes from the filesystem and is not part of the bundle contract).

## 2. Directory layout (locked)

Frozen exactly as `DOCS/SCOPE-JOB-EXPORT.md` §"What's in the bundle"
draws it. The importer rejects any entry whose normalised path falls
outside this set.

```
<root>/                              ← tar root; one top-level dir per bundle
├── manifest.json                    ← required, see §3
├── README.md                        ← required, human-readable cover note
├── template.yaml                    ← required, the current Job spec
├── handover.md                      ← optional; present iff content.has_handover
├── notes/                           ← optional dir; present iff content.note_count > 0
│   └── <filename>.md                ← original filename verbatim, no path components
└── runs/                            ← required; present even when run_count == 0
    └── NNNN/                        ← zero-padded ordinal, width 4, lower bound 0001
        ├── run.json                 ← required, the Run row + frozen snapshots
        ├── stages.jsonl             ← required (may be empty)
        ├── tasks.jsonl              ← required (may be empty)
        ├── todos.jsonl              ← required (may be empty)
        ├── events.jsonl             ← required (may be empty)
        ├── reviews.jsonl            ← required (may be empty)
        └── artifacts/               ← optional; present iff manifest.content.includes_artifacts
            └── <sha256>.bin
```

Rules the importer enforces (one bad entry fails the whole import):

1. Path is relative, contains no `..` segment, no leading `/`, no
   drive letter, no NUL byte.
2. Path matches the regex set above. Anything else (a stray
   `.DS_Store`, a top-level `secrets.env`, a `runs/0001/extra.json`)
   is rejected with `BundleEntryUnexpected { path }`.
3. Entry type is `regular file` or `directory`. Symlinks, hardlinks,
   character / block / fifo devices, sparse files are rejected.
4. The top-level dir name is whatever the exporter chose
   (typically `<job_name>/`); the importer strips one leading
   component before applying rules 1–3.
5. Ordinal directories under `runs/` are zero-padded to width 4 and
   strictly ascending starting at the lowest source `run.ordinal`
   present. Gaps in the source ordinal sequence are preserved
   verbatim (export does **not** renumber), but every directory
   present must parse as a 4-digit integer.

## 3. `manifest.json` (schema_version 1, locked)

```json
{
  "schema_version": 1,
  "exported_at": "2026-05-19T08:14:22Z",
  "exporter": {
    "codeless_version": "0.x.y",
    "host_os": "linux"
  },
  "source": {
    "workspace_name": "acme-app",
    "repo_url": "git@github.com:acme/app.git",
    "repo_commit": "abc1234...",
    "job_name": "user-profile",
    "job_id": "01J...",
    "run_count": 3
  },
  "content": {
    "has_handover": true,
    "note_count": 2,
    "total_events": 1842,
    "includes_artifacts": false
  }
}
```

Field contract — every key is required, every value's type is fixed.

| key                                  | type                  | notes |
| ---                                  | ---                   | --- |
| `schema_version`                     | integer, exactly `1`  | importer refuses anything else for E1; E3 adds the back-compat shim |
| `exported_at`                        | string, RFC 3339 UTC  | always `Z`-suffixed, second precision |
| `exporter.codeless_version`          | string, semver-ish    | `env!("CARGO_PKG_VERSION")` of the `codeless` binary that wrote the bundle |
| `exporter.host_os`                   | string                | one of `linux`, `macos`, `windows`, `other` (from `std::env::consts::OS`) |
| `source.workspace_name`              | string                | `attached_workspaces.fs_root_display` collapsed to the workspace's display name |
| `source.repo_url`                    | string                | `repos.url` from the source workspace; never the local path |
| `source.repo_commit`                 | string, 40-char hex   | HEAD of the source repo at export time; empty string if HEAD is detached and unresolvable |
| `source.job_name`                    | string                | `jobs.name` (post-(B)) — also the `name:` field inside `template.yaml` |
| `source.job_id`                      | string, ULID          | original source-workspace id, for the imported-from chip |
| `source.run_count`                   | integer ≥ 0           | equals the number of `runs/NNNN/` dirs in the bundle |
| `content.has_handover`               | bool                  | true iff `handover.md` is present and non-empty |
| `content.note_count`                 | integer ≥ 0           | equals the number of entries under `notes/` |
| `content.total_events`               | integer ≥ 0           | sum of `events.jsonl` line counts across every Run |
| `content.includes_artifacts`         | bool                  | true iff any `runs/NNNN/artifacts/` dir is present |

Forward compatibility: an importer reading a future bundle MUST refuse
on `schema_version != 1` rather than ignore unknown keys. Unknown
top-level keys at version 1 are themselves a refusal (caught by serde's
`deny_unknown_fields`).

The manifest is the **only** file the importer reads before sizing /
admission checks. `inspect_job_bundle` extracts and validates this
file and returns the parsed struct without touching SQLite.

### Per-Run `run.json` shape (locked for E1)

`run.json` carries the post-(B) `Run` row plus the frozen snapshots
the destination needs to rehydrate history. Exact field set:

```json
{
  "id": "01J...",
  "ordinal": 1,
  "template_snapshot": "name: user-profile\nstages:\n  - …\n",
  "handover_snapshot": "## Done\n\n- …\n",
  "runner": "claude",
  "branch": "codeless/user-profile",
  "worktree_path_source": "/Users/ap/code/acme/runs/user-profile",
  "status": "completed",
  "stop_reason": null,
  "started_at": 1747641262,
  "ended_at": 1747641890,
  "cost_cap_cents": 500,
  "wall_clock_cap_ms": 1800000,
  "cost_cents": 142,
  "resumed_from_stage": null,
  "created_at": 1747641260
}
```

`worktree_path_source` is renamed from `worktree_path` to make
explicit that the destination MUST NOT reuse it; the importer writes
NULL into the destination's `runs.worktree_path` for imported rows.

### Per-Run JSONL streams

- `stages.jsonl` — one Stage row per line, sorted by `ordinal`. All
  columns from the survey in `DOCS/sessions/2026-05-19-job-export.md`
  except `archived` (always exported as `0` for imported history; the
  destination can re-archive locally).
- `tasks.jsonl` — sorted by `(stage_ordinal, task_ordinal)`.
  `lease_holder` and `lease_expires_at` are **omitted on export**
  (they are runtime state, not history); the importer writes NULL.
- `todos.jsonl` — sorted by `(task_id, ordinal)`.
- `events.jsonl` — strictly ascending source `cursor`. Each line
  carries `original_id` (= the source `cursor`) and every other
  event column (`run_id`, `stage_id`, `task_id`, `type`, `payload`,
  `created_at`). The destination rewrites `cursor` in its own
  monotonic sequence; ordering across the file is preserved.
- `reviews.jsonl` — sorted by `requested_at` then `id`.

Each JSONL line is a single JSON object; no embedded newlines, no
trailing comma, no comment lines. Empty streams are an empty file
(zero bytes), not a missing file.

## 4. Secrets column denylist (locked)

Applied row-by-row to **every** column name walked during export.
A column whose name matches any regex below is dropped from the
serialized JSON (key omitted entirely; the destination writes the
column's default / NULL on import).

The denylist is **case-insensitive** and matches on substring.

| pattern             | rationale |
| ---                 | --- |
| `(?i)token`         | bearer tokens, API tokens, refresh tokens |
| `(?i)secret`        | named secrets, `client_secret`, `webhook_secret` |
| `(?i)api[_-]?key`   | `api_key`, `apikey`, `api-key` |
| `(?i)password`      | `password`, `pw`, `passwd` are caught by the broader form `pass(word|wd)` below |
| `(?i)pass(word|wd)` | catches `password`, `passwd` without false-positiving `bypass*` |
| `(?i)private[_-]?key` | `private_key`, `privatekey`, `private-key` |
| `(?i)credential`    | `credential`, `credentials_json` |
| `(?i)bearer`        | `bearer_token` (also caught by `token`, kept for clarity) |
| `(?i)auth[_-]?header` | `auth_header`, `authheader` |

Current schema (per the stage-1 survey) has **zero** matches against
this denylist. The list is therefore prospective: it catches a future
migration that adds e.g. `runner_api_key` or `webhook_secret` by
schema rather than by code review.

The denylist applies to **column names**, not values. We do not scan
payload JSON for tokens; that is a different problem (see open
question OQ-D below). The `events.payload` column is exported
verbatim and the user is on notice via the README cover note that
event payloads may contain whatever the runner chose to log.

Additionally — re-stated for emphasis, not a regex:

- **No filesystem reads outside the source repo's `.codeless/` +
  `runs/<job>/`.** Specifically, `$XDG_DATA_HOME/codeless/` is read
  only via SQLite; no raw file walk into it.
- **No env var capture.** The serializer never reads `std::env`.

## 5. Size caps (locked for E1)

Defaults are hardcoded constants in `codeless-runtime::job_export`.
There is no per-workspace settings infrastructure to hang these off
yet; SCOPE.md flags adding one as E2 work.

| cap                              | default | enforced at |
| ---                              | ---     | --- |
| `MAX_BUNDLE_BYTES`               | 200 MiB | exporter (refuse to finalize), importer (refuse before any extraction beyond manifest) |
| `MAX_ENTRY_BYTES`                | 10 MiB  | importer per tar entry; exporter refuses to emit one |
| `MAX_MANIFEST_BYTES`             | 64 KiB  | importer reads at most this many bytes for `manifest.json`; exporter refuses to emit larger |
| `MAX_README_BYTES`               | 64 KiB  | importer + exporter |
| `MAX_TEMPLATE_YAML_BYTES`        | 1 MiB   | importer + exporter |
| `MAX_HANDOVER_BYTES`             | 1 MiB   | importer + exporter |
| `MAX_NOTE_BYTES`                 | 1 MiB   | per-file note size; sum still capped by `MAX_BUNDLE_BYTES` |
| `MAX_RUN_JSON_BYTES`             | 1 MiB   | per Run's `run.json` |
| `MAX_JSONL_LINE_BYTES`           | 1 MiB   | any single JSONL row; protects against a runaway `events.payload` |
| `MAX_RUNS_PER_BUNDLE`            | 1024    | sanity bound; refuses a Job with > 1024 Runs without operator override |
| `MAX_EVENTS_PER_RUN`             | 500 000 | sanity bound; refuses a Run with > 500k events; surfaces an actionable error |

Constants live in `codeless-runtime/src/job_export/limits.rs` (stage 4
work). Each has a `pub const` plus a `#[cfg(test)]` override hook so
the round-trip tests in stage 7 can run with tiny caps.

The importer enforces caps **streaming** — it reads the tar one entry
at a time, accumulating per-entry and per-bundle byte counts, and
aborts the moment a cap is exceeded. No "decompress everything to a
temp dir then size-check" path; that would defeat the cap as a DoS
guard.

Cap-exceed errors carry the offending cap name and the observed size,
so the UI can show "this bundle's `events.jsonl` for run 3 exceeds
10 MiB (observed: 12.4 MiB)" rather than a generic "too big."

## 6. Open question resolutions

The stage-1 session doc and `SCOPE.md` flagged five open questions
for stage 2. Locked answers below; the implementation stages should
not re-litigate these.

- **OQ-1 — Is JOB-WORKFLOW (B) merged?** No (re-verified this
  session: no `runs` migration on disk; `jobs` still owns the
  per-attempt columns). Stage 1 stays halted `[!]`. Stage 2's
  design is locked against the post-(B) world; implementation stages
  4–7 remain blocked on (B). No change.

- **OQ-2 — Size cap defaults.** Confirmed at the values in §5
  above. No per-workspace override surface in E1 (E2 work). The
  constants live in `limits.rs` for one-place editing.

- **OQ-3 — `output_path` resolution.** **Jailed under the active
  workspace's `fs_root_canonical`.** Concretely: the runtime
  canonicalises `output_path`, checks it starts with the workspace's
  `fs_root_canonical + std::path::MAIN_SEPARATOR`, and refuses
  otherwise with `OutputPathOutsideWorkspace { fs_root, output_path }`.
  The UI's save dialog pre-fills a path under the workspace; the user
  may edit it but cannot escape the jail. CLI parity (E2) applies the
  same check. Rationale: matches the existing `fs.*` RPC trust
  boundary; avoids the "exporter writes to `/etc/…`" footgun.

- **OQ-4 — `events.cursor` monotonicity under (B).** Confirmed via
  `0001_initial.sql:85` — `cursor INTEGER PRIMARY KEY AUTOINCREMENT`
  is workspace-monotonic by SQLite semantics, regardless of which
  Run a row points at. (B)'s migration only re-keys `events.job_id →
  events.run_id`; the AUTOINCREMENT property is untouched. The
  exporter therefore relies on `cursor ASC` for `events.jsonl`
  ordering; the importer rewrites cursors in the destination's
  sequence and stores the source cursor in `original_id`.

- **OQ-5 — Where the imported handover lands on disk.** On the
  Job row, in `jobs.handover_md` (post-(B) column per
  `DOCS/JOB-WORKFLOW.md` §(B)). The first new Run snapshots it like
  any other (`runs.handover_snapshot`). No special-casing on the
  import path; the importer simply writes the bundle's `handover.md`
  contents into `jobs.handover_md` and lets the existing assembler
  pick it up on `[run]`. The bundle's per-Run `runs/NNNN/run.json`
  carries the **historical** `handover_snapshot` for each imported
  Run separately; the two never alias.

- **OQ-D (new this stage) — Should `events.payload` be scanned for
  secrets?** **No, not in E1.** The denylist is column-name-scoped.
  Payload contents are exported verbatim and the README cover note
  (§8 below) calls this out. Adding payload-content scanning is E2
  scope at the earliest, gated by a clear "what counts as a secret in
  a payload" decision.

- **OQ-E (new this stage) — `scheduled_pause_points`.** The
  stage-1 survey flagged this as "arguably should be in the bundle."
  Decision: **out of scope for E1.** The bundle is one Job's
  template + handover + notes + Run history; pause schedules are a
  forward-looking control surface the destination can re-author.
  Logged as a follow-up for E2.

## 7. Refuse-to-export preconditions

The exporter aborts before writing any bytes if any are true:

1. The Job has any Run whose `status` is not in `{completed, failed,
   stopped, cancelled}` (i.e. a live Run). Surfaces
   `JobNotTerminal { run_id, status }`. UI's `[Export]` button is
   disabled in this state.
2. `repos.url` for the source repo is empty (cannot fill
   `manifest.source.repo_url`). Surfaces `SourceRepoUrlMissing`.
3. `output_path` fails the jail check from OQ-3. Surfaces
   `OutputPathOutsideWorkspace`.
4. The walked content exceeds any §5 cap. Surfaces the same
   per-cap error the importer would (`BundleTooLarge`,
   `EntryTooLarge`, etc.).

## 8. README cover note (locked outline)

The bundle's top-level `README.md` is human-readable and exists so
that a teammate who opens the tar with plain `tar` can orient. It is
not load-bearing for the importer. Exporter generates it from a
template; stage 4 work to wire up. Required sections, in order:

```
# Codeless Job bundle: <job_name>

Exported: <exported_at>
Source workspace: <workspace_name>
Source repo: <repo_url> @ <repo_commit>
Codeless version: <codeless_version>
Schema version: 1

## What's inside

- template.yaml — the Job spec
- handover.md — current handover (may be absent)
- notes/ — Job notes
- runs/NNNN/ — one frozen Run per directory, with run.json + JSONL streams
- manifest.json — machine-readable index (the importer reads this first)

## What's NOT inside

- Worktree contents (the destination cuts a fresh worktree)
- Secrets, env vars, API keys (denylist applied at export)
- Event payloads are exported verbatim — anything the runner logged
  ends up here. Review before sharing externally.

## How to import

Open the destination workspace in Codeless → Workspaces sidebar →
[Import Job…] → pick this file.
```

## 9. What stages 3–7 inherit from this lock

- Stage 3 REVIEW: read §§ 1–8 of this file plus the three RPC arg
  structs in `DOCS/SCOPE-JOB-EXPORT.md` §"RPC surface". Approve or
  request changes before stage 4 lands code.
- Stage 4 walker + serializer: build to §§2, 3, 4. The
  `limits.rs` constants in §5 are the only place the cap numbers
  appear in code.
- Stage 5 importer: enforce every rule in §2 and every cap in §5
  streaming. The refuse-paths in §7 are mirrored on the import side
  for symmetric errors.
- Stage 6 RPCs: `export_job` / `import_job` / `inspect_job_bundle`
  signatures are already locked in `SCOPE-JOB-EXPORT.md`. The
  refuse-to-export preconditions in §7 are part of `export_job`'s
  error surface.
- Stage 7 round-trip test: assert byte equality on the JSONL files
  modulo `cursor` rewrites (compare `original_id` ascending against
  the source `cursor` ascending) and on every row's body. Tar-safety
  tests cover each rejected entry shape in §2 rule list.

## 10. What's deliberately not locked here

- The exact ULID format for `Run.id` and the IDs the importer
  generates. The importer reuses the runtime's standard ULID source;
  the bundle records source IDs in `original_id` (for events) and in
  the per-run `run.json.id` (for the Run row's chip / forensics).
- The tar library / gzip library choice. R1 says it lives in
  `codeless-runtime` or `codeless-adapters-host`; choosing
  `tar` + `flate2` vs. anything else is a stage-4 call so long as
  the constraints in §1 hold.
- The exact wording of error variants. Names listed above are the
  intent; stage 4–6 may rename for `thiserror` ergonomics so long as
  the structured payload (the offending cap, path, etc.) survives.
