# SCOPE-JOB-EXPORT — exporting and importing a Job (with history)

Status: not started
Owner: ap@nube-io.com
Created: 2026-05-19

## Summary

Today a Job is married to the repo + workspace it was born in. Its
template lives at `.codeless/jobs/<name>.yaml`, its handover and log
live at `runs/<name>/`, and its Run / Stage / Task / Event / Review
rows live in that workspace's `codeless.db`. You cannot copy a Job
to a teammate's machine, drop it into another workspace, or seed a
fresh Job from a Job that ran well last week — short of hand-curating
files and giving up on the run history entirely.

This doc specifies a **Job bundle** — a single archive that captures
everything needed to reuse a Job in another workspace — plus the
`export_job` / `import_job` RPCs, the UI affordances, and the
semantics of "import then hit run."

> **Sister docs.** [`JOB-MODEL.md`](./JOB-MODEL.md) defines what lives
> on disk in the user's repo (`template.yaml`, `handover.md`,
> `log.md`). [`JOB-WORKFLOW.md`](./JOB-WORKFLOW.md) (B) defines the
> Job/Run split this doc assumes. [`WORKSPACE-ATTACH.md`](./WORKSPACE-ATTACH.md)
> defines the per-workspace boundary that scoping is anchored to.
> Where this doc contradicts any of them, **those win** — fix this
> file rather than diverge.

## Goals

1. **Export.** From any Job page (browser, desktop, CLI) the user
   can produce a single file — a `.codeless-job` bundle — that
   contains:
   - the current `template.yaml`,
   - the current `handover.md` and every `notes/*.md`,
   - every Run row with its frozen `template_snapshot` and
     `handover_snapshot`,
   - every Stage, Task, Todo, Event, Review row keyed under those
     Runs,
   - a manifest describing schema version, exporting codeless
     version, source repo identity (URL + commit SHA, not the local
     path), and any per-workspace settings the Job depended on.
2. **Import** into a different workspace by picking the bundle and a
   destination workspace. The importer:
   - writes the `.codeless/jobs/<name>.yaml` to the destination
     repo (committed),
   - creates a new Job row in the destination workspace's SQLite,
   - rehydrates every Run / Stage / Task / Todo / Event / Review with
     **new IDs** but preserved ordinals, timestamps, statuses, and
     bodies — so the history is browsable in the UI exactly as it
     was on the source side,
   - imports `handover.md` and `notes/*.md` into the destination
     worktree path the next Run will use.
3. **Run.** After import, `[run]` works with no extra steps — the
   imported template is the spec the next Run reads; the imported
   handover seeds the prompt prefix; prior runs show up in the runs
   list as read-only history.
4. **Per workspace.** Every export and every import is scoped to one
   destination workspace (per WORKSPACE-ATTACH). Cross-workspace
   moves are explicit: export → switch workspace → import.

## Non-goals

- Cross-version migration. The bundle declares a `schema_version`;
  an importer that does not understand it refuses with a clear
  error. We will not silently translate older bundles.
- Sharing **secrets**. Bundles never carry API keys, bearer tokens,
  or the contents of `$XDG_DATA_HOME/codeless/`. The destination
  workspace's own secrets apply on the next Run.
- Sharing **worktree contents**. The Job ran against a specific
  commit of a specific repo; the bundle records the source repo URL
  and SHA, but the destination workspace must have that repo
  attached and on a compatible commit. Re-running on a wildly
  divergent tree is the user's choice, not the bundle's promise.
- Multi-Job bundles. One Job per file. A Plan (per JOB-WORKFLOW
  §"Job chaining") gets its own bundle format if and when it ships.
- Re-executing the history. Import rehydrates rows for browsing; it
  does **not** replay events through the runtime. The next Run
  starts at stage 1 (or at `resume_from` if the user picks a stage)
  exactly as a normal re-run would.
- Live federation. A bundle is a file moved by the user. No "follow
  this Job on another server" sync. That belongs in Phase 7.

## What's in the bundle

A `.codeless-job` file is a gzipped tar archive with a fixed layout:

```
<name>.codeless-job/                  ← extracted, the tar contains:
├── manifest.json                     ← required, see below
├── template.yaml                     ← the current spec
├── handover.md                       ← current handover (may be absent)
├── notes/
│   └── <filename>.md                 ← every note, ordered by filename
├── runs/
│   ├── 0001/                         ← per-Run snapshot, ordinal-named
│   │   ├── run.json                  ← Run row + frozen snapshots
│   │   ├── stages.jsonl              ← one Stage row per line
│   │   ├── tasks.jsonl
│   │   ├── todos.jsonl
│   │   ├── events.jsonl              ← the full event stream for this Run
│   │   ├── reviews.jsonl
│   │   └── artifacts/                ← optional, see "What's not in"
│   ├── 0002/
│   └── ...
└── README.md                         ← human-readable cover note
```

`manifest.json`:

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

### What's *not* in the bundle, deliberately

- **The worktree.** Worktrees are derived state; the destination
  recreates one on the next Run from the destination repo's tree.
- **`codeless.db` rows that aren't this Job's.** Export walks one
  Job + its Runs and nothing else.
- **Secrets / env / API keys.** Period.
- **`log.md`.** It is the audit trail for the *source* worktree;
  it is reconstructible from the imported events if anyone asks for
  it. Keeping it out avoids a parallel source of truth post-import.
- **Artifacts > N MB by default.** A REVIEW that uploaded a large
  diff or test-failure artifact gets a reference + SHA but not the
  blob, unless the user passes `include_artifacts: true` on export.

### Compatibility — what `schema_version` covers

Schema version is bumped when **any** of the following change in a
way that an older importer cannot ignore:

- Wire types in `codeless-types` that appear in the JSONL streams
  (Run, Stage, Task, Todo, Event, Review).
- The manifest shape.
- The bundle directory layout.

Bumping is not free. Each bump means a migration shim in the
importer for the previous N versions. P1 ships `schema_version: 1`
and refuses anything else. P2 onward gains best-effort backward
compatibility for one prior version.

## RPC surface

```rust
// codeless-rpc / methods.rs

#[derive(Serialize, Deserialize, specta::Type)]
pub struct ExportJobArgs {
    pub job_id: JobId,
    /// Destination path on the *server*. The UI streams the bytes
    /// from this path via `fs.read_file` after the call returns, so
    /// browser shells don't need a server-pushed download channel.
    pub output_path: String,
    pub include_artifacts: bool,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub struct ExportJobResult {
    pub output_path: String,
    pub bytes_written: u64,
    pub run_count: u32,
    pub event_count: u32,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub struct ImportJobArgs {
    pub workspace_id: WorkspaceId,
    /// Path to the bundle on the server.
    pub bundle_path: String,
    /// Optional rename. Defaults to manifest.source.job_name.
    pub rename_to: Option<String>,
    /// What to do if a Job with the same name exists in the
    /// destination workspace.
    pub on_conflict: ImportConflictPolicy,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub enum ImportConflictPolicy {
    /// Refuse and surface the existing Job's id.
    Refuse,
    /// Import under a new name with a numeric suffix.
    Suffix,
    /// Replace the existing Job (drops its rows, keeps its worktree
    /// intact on disk for the user to inspect).
    Replace,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub struct ImportJobResult {
    pub job_id: JobId,
    pub imported_name: String,
    pub run_count: u32,
    pub warnings: Vec<ImportWarning>,
}
```

`ImportWarning` is for non-fatal mismatches — destination repo SHA
differs from the bundle's `source.repo_commit`, a referenced
artifact was excluded, a note filename collided with an existing one
(suffixed `-imported`), etc.

A separate `inspect_job_bundle(bundle_path)` RPC returns the
manifest without touching SQLite, so the UI can show "this bundle is
3 runs from `acme/app@abc1234`, 1.2 MB, exported 2026-05-18" before
the user commits.

## UI

Three surfaces, one new dialog.

**Job page header — `[Export]` button.**
- Click → server-side save dialog (Tauri shell: native picker;
  browser: input field + sensible default in the workspace's local
  paths).
- On success: toast "exported to `<path>`" with `[open folder]` and
  `[copy path]` actions. Browser shell additionally offers
  `[download]` which streams the file via `fs.read_file`.

**Workspaces sidebar — `[Import Job…]` entry on the active
workspace's menu.**
- Click → file picker for a `.codeless-job`.
- Picker → preview dialog showing manifest summary (source repo,
  commit, run count, exported timestamp, exporter version,
  warnings if the local repo's SHA differs).
- Conflict policy selector + optional rename field.
- `[Import]` → calls `import_job`, navigates to the new Job page,
  surfaces warnings as a dismissible banner.

**Imported-Job badge on the Job page.**
- A small `imported from acme/app@abc1234` chip near the title.
- Hovering shows the original `job_id`, exported timestamp, and
  exporter version.
- Linkable to the manifest in a read-only viewer for forensics.

CLI parity (Phase 2 work, mention here for completeness):

```sh
codeless job export <job-name> --out ./user-profile.codeless-job
codeless job import ./user-profile.codeless-job \
    --workspace acme-app \
    [--rename user-profile-v2] \
    [--on-conflict refuse|suffix|replace]
```

## Semantics — "import then hit run"

Post-import, hitting `[run]` on the imported Job behaves exactly
like a fresh run on a newly-created Job:

- `submit_job` reads the **imported** template (now on disk at the
  destination repo's `.codeless/jobs/<name>.yaml`), writes a fresh
  Run row with `ordinal = max(imported_ordinals) + 1`, snapshots
  the current template + handover, cuts a worktree off the
  destination repo at its current HEAD, and starts at stage 1.
- The imported Run rows show in the Run list as **historical
  read-only entries**. Their status pills and timestamps reflect
  the source side at export time; no event stream is replayed.
- Resume-from-stage (per JOB-WORKFLOW (B)) treats the imported Runs
  as eligible source ordinals — the user can "re-run from stage 3
  of imported Run 2" exactly as they could from a local Run.
- The imported `handover.md` and `notes/*.md` flow into the new
  Run's prompt prefix via the existing assembler
  (JOB-WORKFLOW §"How feedback flows…"). No special-casing.

If the destination repo's HEAD differs from
`manifest.source.repo_commit` by more than a configurable threshold
(default: any drift) the run dialog surfaces an inline warning
before the user submits. Imported tests / paths may not exist on
the new tree; the bundle does not promise they will.

## Conflict & rename rules

`on_conflict = Refuse` (default in UI for safety):
- Importer aborts with `JobNameExists { existing_job_id }`. The UI
  re-opens the preview dialog with the rename field focused.

`on_conflict = Suffix`:
- The importer tries `<name>`, `<name>-2`, `<name>-3`, … until free.
- `.codeless/jobs/<chosen>.yaml` is what's written; the bundle's
  internal `name:` field is rewritten to match.

`on_conflict = Replace`:
- The existing Job's rows are soft-deleted (status `superseded`,
  not physically dropped); a `Event::JobReplacedByImport` is emitted
  on the old job's stream.
- The old worktree on disk is untouched — the user can still cd
  into it.
- The new Job inherits the chosen name and the imported history.

Replacement is loud. The UI prompts twice; the CLI requires an
explicit `--on-conflict replace --yes`.

## Security & integrity

- **Path traversal in tar.** The importer rejects entries with
  absolute paths, `..` segments, symlinks, or paths outside the
  expected fixed layout. One bad entry fails the whole import.
- **Size caps.** Default per-bundle cap (configurable per
  workspace, e.g. 200 MB); per-entry cap (e.g. 10 MB).
  Manifests beyond cap fail at `inspect_job_bundle` time, before
  any rows are written.
- **Manifest signature (post-MVP).** P1 ships unsigned bundles
  trusted at face value. P2 adds an optional Ed25519 signature
  block over the manifest + per-file SHA-256 list, and a workspace
  setting "only import bundles signed by <pubkey>". Single-tenant
  MVP doesn't need it; multi-org export does.
- **No secrets.** Re-stated for emphasis. The exporter walks
  Job/Run rows and the repo's `.codeless/` + `runs/` directories;
  nothing under `$XDG_DATA_HOME/codeless/` other than the SQLite
  rows themselves is read, and the row serializer omits any column
  whose name matches a fixed denylist (`*token*`, `*secret*`,
  `*api_key*`).

## Sequencing — (E1) → (E2) → (E3)

Same phased discipline as JOB-WORKFLOW (A) → (B). Land (E1) only
after JOB-WORKFLOW (B) has merged — the Job/Run split is a hard
precondition for exporting "the full stage/tick history" cleanly.

**(E1) — Export + Import core, single workspace.** ~3-4 days.
- `codeless-runtime/src/job_export/` module: walker that pulls
  Job + Runs + Stages + Tasks + Todos + Events + Reviews into
  in-memory structs.
- Serializer: writes `manifest.json`, JSONL streams, file copies
  under a temp dir; tars + gzips into `output_path`.
- Importer: validates manifest, opens tar streaming, writes
  destination repo files, batches SQLite inserts in a single
  transaction.
- `export_job`, `import_job`, `inspect_job_bundle` RPCs.
- UI: `[Export]` button on Job page; `[Import Job…]` on workspace
  menu; preview dialog; toast / banner surfaces.
- Tests: round-trip property test — export a Job, import into a
  scratch workspace, assert every row's body and ordering match.

**(E2) — Conflict policies, artifacts, warnings.** ~2 days.
- `on_conflict` policies plumbed end to end including the soft-
  delete path for `Replace`.
- Artifact inclusion flag, size cap config, per-workspace import
  limits.
- Warning surface in the UI (banner + per-row tooltips on the
  imported Runs list).
- CLI subcommands `job export` / `job import`.

**(E3) — Signed bundles, cross-version shim.** ~1 week, post-MVP.
- Optional Ed25519 signature over manifest + content hashes.
- Workspace setting `import.require_signature_from: [<pubkey>…]`.
- One-version-back importer shim (read `schema_version: 1` from a
  v2 codebase).

## Open questions

1. **Should imported runs be editable or strictly read-only?**
   Default: strictly read-only. The user can re-run from a stage on
   an imported Run (that creates a *new* local Run), but cannot
   edit the historical handover/notes on a past imported Run. The
   mutable `handover.md` on the Job itself is the post-import edit
   surface — same as for native Runs.
2. **What about a Job that was running at export time?**
   Refuse to export. A Job must be in a terminal state on every
   Run (`completed` / `failed` / `stopped` / `cancelled`) before
   it can be exported. Forcing this avoids racing the event stream.
   The UI's `[Export]` button is disabled while any Run is live.
3. **Bundle deduplication.** If the same bundle is imported twice,
   `Refuse` blocks it by name; `Suffix` produces `<name>-2`. We
   do not de-dupe by manifest hash — the user's intent might be
   "import this fresh again." Revisit if it becomes annoying.
4. **What if the destination workspace has no repo attached for the
   source URL?** Import still succeeds — the Job row points at the
   destination workspace's *current active repo*, and the run
   dialog warns about the URL mismatch. The user can attach the
   matching repo via WORKSPACE-ATTACH and re-target the Job
   afterwards.
5. **Per-workspace registry of imported Jobs?** Not in MVP. The
   `imported from …` badge + the manifest viewer cover the
   forensics; a dedicated "Imports" tab is over-engineering until
   we see the user importing dozens.
6. **Event ID collisions.** Events carry monotonic IDs scoped to
   the source workspace. The importer regenerates IDs in the
   destination's sequence; the bundle records the original ID in
   an `original_id` field on each event for cross-reference.
7. **What about Plans (per JOB-WORKFLOW §"Job chaining")?**
   Out of scope here. A Plan bundle is a separate doc once Plans
   have UI (P3). A Plan bundle would reference Job bundles by name
   + manifest hash rather than embedding them.

## Out of scope (re-stated for clarity)

- Sharing bundles via a hosted service. The bundle is a file; how
  it moves between machines is the user's call (scp, Slack, email,
  S3, USB stick).
- Editing a bundle externally. Bundles are not a user-authored
  format — they are exporter output. Round-tripping
  edit-the-tarball is not supported.
- Diffing two bundles in the UI. Useful, not MVP. A future
  `inspect_job_bundle` extension could surface this.
