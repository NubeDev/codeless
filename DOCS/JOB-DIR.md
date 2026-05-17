# JOB-DIR — a job is a directory of editable docs, not a single YAML

> **Sister docs.** [`JOB-MODEL.md`](./JOB-MODEL.md) is the user-facing
> framework — three files, fixed format. [`JOB-WORKFLOW.md`](./JOB-WORKFLOW.md)
> covers the *iterate* half of the loop (edit the template, edit the
> handover, drop a note, re-run). This doc extends both by promoting
> a job from "one YAML file" to "one directory the user fills with
> scope, workflow, and any other markdown they want the agent to read."
>
> Where this doc disagrees with `JOB-MODEL.md` or `SCOPE.md`, **those
> win** — open an issue and update this file rather than diverge.

## The problem

`JOB-MODEL.md` ships a single file per job:

```
acme/repo/
└ .codeless/jobs/<name>.yaml      ← name, goal, stages
```

`goal:` is one line. There is nowhere to put:

- A **scope** document — what we're trying to do, what's out of scope,
  constraints, deliverables.
- A **workflow** document — how the agent should drive the work,
  verify steps, what to commit when.
- Any other markdown the user wants the agent to see — design notes,
  links, scratch.

Today's iterate-loop (JOB-WORKFLOW.md) plugs a partial hole:
`runs/<job_id>/notes/*.md` lets the user drop ad-hoc context, but
those files live inside a per-run worktree, are tied to *one specific
run*, and don't survive across runs of the same job. They are
*per-run feedback*, not *job-level intent*.

What the user has been asking for is the missing surface: a place to
write down what a job is **for** that lives next to the spec, in the
source repo, version-controlled, editable in the UI.

## The framework — one directory per job, several files inside

```
acme/repo/
├ .codeless/
│  ├ config.yaml                    ← repo defaults (unchanged)
│  └ jobs/
│     └ <name>/                     ← new: directory per job
│        ├ template.yaml            ← stages, runner, caps
│        ├ SCOPE.md                 ← the brief; editable, agent-read
│        ├ WORKFLOW.md              ← how to drive the work; agent-read
│        └ *.md                     ← any other docs, all agent-read
└ runs/
   └ <name>/
      ├ handover.md                 ← contract between sessions (unchanged)
      └ log.md                      ← session audit (unchanged)
```

Everything inside `.codeless/jobs/<name>/` is **committed in the user's
source repo**. The user authors it; the agent reads it before each
stage; `git log` records every edit.

### `template.yaml` — the spec

Same three fields as today's flat YAML:

```yaml
name: user-profile
goal: Add GET /api/users/:id endpoint and a profile page.
stages:
  - add User model and migration
  - add GET /api/users/:id handler with tests
  - REVIEW api shape before frontend uses it
  - add getUser(id) on the frontend RPC client
  - build UserProfilePage component
  - wire the /users/:id route
```

Migration from the flat layout is automatic: the first edit through
the new file surface promotes
`.codeless/jobs/<name>.yaml` → `.codeless/jobs/<name>/template.yaml`
and commits both changes (one commit each so `git log` records the
migration explicitly).

### `SCOPE.md` — the brief

User-authored prose. What the job is *for*, what success looks like,
what's out of scope, constraints, deliverables, links. There is no
required schema — it's a markdown file. The agent reads it before
every stage.

Example:

```markdown
# Scope

Add the GET /api/users/:id endpoint and a profile page that uses it.

## In scope

- Backend handler with tests.
- Frontend RPC client method.
- Profile page component + route.

## Out of scope

- Avatar field — backend has no avatar_url column. Don't add one.
- Auth changes — assume the session middleware already authorises.

## Constraints

- React 19, react-router-dom v6 (use `<Route>`, not `<Switch>`).
- `display_name`, not `name`, on the user object.
```

`SCOPE.md` is treated as first-class by the runtime: the prompt builder
emits it under a `## Scope` heading at the top of the docs section.

### `WORKFLOW.md` — how to drive the work

User-authored prose, same idea but for *process*. How the agent
should sequence the stages, what to verify between them, what counts
as "done" for each, what to commit when.

Example:

```markdown
# Workflow

- Before any code change, read `git log -20` and `ls src/` to orient.
- Each stage: write the code, run `cargo test`, commit only if green.
- The REVIEW stage pauses for the user. Write a one-line summary in
  the handover and stop.
- The frontend `verify` step is `pnpm test` — ~40s, not a hang.
```

Like `SCOPE.md`, treated as first-class: emitted under `## Workflow`
in the prompt.

### Other `.md` files

Anything else the user drops in the directory (`design.md`,
`links.md`, `notes.md`). The agent reads them in filename-alphabetical
order, each under a `## <filename>` heading, after the scope and
workflow. The user can drop as many as they like.

`.yaml` and `.yml` files are also readable through the file surface
(e.g. `template.yaml` itself), but only `.md` files feed the prompt.

## The on-disk contract

```
| Path                                         | Who writes      | Committed | Why                                                       |
|----------------------------------------------|-----------------|-----------|-----------------------------------------------------------|
| `<repo>/.codeless/jobs/<name>/template.yaml` | User            | yes       | The spec, same fields as flat-layout `<name>.yaml`.       |
| `<repo>/.codeless/jobs/<name>/SCOPE.md`      | User            | yes       | The brief. Agent reads it every stage.                    |
| `<repo>/.codeless/jobs/<name>/WORKFLOW.md`   | User            | yes       | How the agent should drive the work.                      |
| `<repo>/.codeless/jobs/<name>/*.md`          | User            | yes       | Any other supporting docs. Agent reads them too.          |
| `<worktree>/runs/<job_id>/handover.md`       | Session + user  | yes       | Inter-session contract (unchanged from JOB-MODEL.md).     |
| `<worktree>/runs/<job_id>/log.md`            | Session         | yes       | Audit trail (unchanged from JOB-MODEL.md).                |
| `<worktree>/runs/<job_id>/notes/*.md`        | User            | yes       | Per-run feedback (unchanged from JOB-WORKFLOW.md).        |
```

Two distinct surfaces, two distinct purposes:

- **Job docs** (`.codeless/jobs/<name>/`) — what the job is, persists
  across every run, edits apply to all future runs of this job.
- **Run notes** (`runs/<job_id>/notes/`) — per-run feedback, tied to
  one attempt, the next run of the same job sees the prior run's
  handover + the next user note, but not a different run's notes.

## How the agent reads the docs

The prompt-prefix builder folds the directory's docs into every
stage prompt, in this order:

```
[stage prompt is built from, in order]
  1. # Prior session handover
       ← runs/<job_id>/handover.md, prior session's wrap-up
  2. # Job docs
       ← .codeless/jobs/<name>/SCOPE.md     (under ## Scope)
       ← .codeless/jobs/<name>/WORKFLOW.md  (under ## Workflow)
       ← .codeless/jobs/<name>/*.md         (under ## <filename>, alpha)
  3. # Notes from the user
       ← runs/<job_id>/notes/*.md  (alpha, this run's feedback)
  4. # Job goal
       ← template.yaml `goal:`
  5. # Stage N of M
       ← template.yaml `stages:` entry
  6. # What to do now
       ← orchestrator instruction
```

Order matters. Handover comes first because it's the contract.
Job-level docs come next because they're the persistent intent. Notes
come last because they're "what's different *this time*" — they
should override or refine the docs, not be drowned by them.

If the directory layout isn't present (legacy flat-YAML job), the
`# Job docs` section is empty. The other four sections still flow.

## The RPC surface

Four typed methods own the file surface. All take `job_id`; the
runtime resolves it to `.codeless/jobs/<template.name>/`.

```
| Method              | Purpose                                                              |
|---------------------|----------------------------------------------------------------------|
| `list_job_files`    | Return every file in the directory + a layout marker.                |
| `read_job_file`     | Read one file by basename.                                           |
| `write_job_file`    | Create or overwrite a file. Migrates flat→dir on first write.        |
| `delete_job_file`   | Remove a file. `template.yaml` is reserved — refuse to delete it.    |
```

Plus the existing `update_job_template` (which now writes to
`<name>/template.yaml` and migrates flat→dir transparently).

Every write is committed in the user's repo with a stable commit
subject so `git log` is a real audit trail:

```
update template: <name>
update job-file: <name>/<filename>
delete job-file: <name>/<filename>
migrate template: <name> → directory layout
migrate template: <name> (remove flat YAML)
```

### Filename rules

- Single basename, no path segments. `../escape.md` is rejected.
- No dotfiles. `.env` is rejected.
- `template.yaml` is reserved on `write_job_file` / `delete_job_file`
  — use `update_job_template` for the spec, which carries the YAML
  validation and rename guard.
- Bare names get `.md` appended. `design` → `design.md`. Files that
  already end in `.md`, `.yaml`, or `.yml` keep their extension.

### Layout marker

`list_job_files` returns a `layout` field:

- `"directory"` — `<name>/` exists, modern layout, all features work.
- `"flat"` — `<name>.yaml` exists, no directory. The next write
  through any of `write_job_file` / `update_job_template` migrates
  the job to the directory layout in two commits.
- `"none"` — neither exists yet. First save creates the directory.

The UI surfaces the marker as a one-line hint when the job is in the
flat or none state so the user understands what their first save
will do.

## The UI

A single **Spec** pane in the job page sub-rail replaces today's
Template pane. Inside:

```
┌─────────────────────────────────────────────────────┐
│ files                                       [+ file]│
├─────────────┬───────────────────────────────────────┤
│ template.yaml (spec)                                │
│ SCOPE.md (scope)                                    │
│ WORKFLOW.md (workflow)                              │
│ design.md                                           │
│ links.md                                            │
└─────────────┴───────────────────────────────────────┘
```

- Two-pane layout: file list on the left (~14rem), CodeMirror editor
  on the right.
- Selecting a file fetches its content fresh and renders it in the
  editor with the right language (YAML for `template.yaml`, markdown
  for the rest).
- **Save** commits the file via the right RPC (`update_job_template`
  for the spec, `write_job_file` for everything else).
- **Discard** reverts the buffer to the last-saved content.
- **+ file** opens a dialog with one-click presets for `SCOPE.md` and
  `WORKFLOW.md` (only shown when they're not on disk yet), or any
  custom filename. The body is editable inline before saving.
- **Delete** (× on hover) for any file except `template.yaml`.
- **Open in tab** sends the file's absolute path to the host's editor
  tab so the user gets the full editor experience for long docs.

The legacy-flat-YAML hint sits at the top of the file list when the
job hasn't been migrated yet:

> Legacy flat-YAML layout. Your first save will promote this job to
> a directory so you can add SCOPE / WORKFLOW / docs.

## Migration

Three migration paths, all transparent to the user:

1. **Fresh job, new layout.** `submit_job` with a `template_yaml`
   does not write anything to disk by itself — it stores the YAML in
   the DB. The first `update_job_template` (whether from the UI or
   the planner) creates `<name>/template.yaml`.

2. **Legacy flat-YAML job, first edit through `update_job_template`.**
   The new contents land at `<name>/template.yaml`; the old
   `<name>.yaml` is deleted; both changes are committed in separate
   commits so `git log` records the migration.

3. **Legacy flat-YAML job, first edit through `write_job_file`** (the
   user wants to add `SCOPE.md` without touching the spec). Same
   migration: the existing flat YAML is copied to
   `<name>/template.yaml`, the flat file is removed, then the new
   `.md` file is written. Three commits: migrate-template,
   migrate-template-remove-flat, update-job-file.

No CLI flag, no manual step. The user discovers the new layout by
clicking the "+ file" button.

## What this is *not*

- **Not a file explorer for the whole repo.** The Spec pane only
  shows files under `.codeless/jobs/<name>/`. The existing file
  explorer handles everything else.
- **Not the worktree.** Run-specific files (handover, log, notes)
  stay in the worktree under `runs/<job_id>/`. The Spec pane never
  shows those.
- **Not a wiki.** Each job's directory is a flat list of files; no
  subdirectories, no nested markdown. If a job needs richer
  structure, it's probably two jobs.
- **Not a planner replacement.** The planner (today's hard-coded
  fallback, tomorrow's Rig-backed thing) still proposes `name`,
  `goal`, and `stages`. SCOPE.md and WORKFLOW.md are user-authored
  prose — the planner could pre-populate them with placeholders, but
  it does not own them.

## What's deliberately not here yet

- **Per-stage docs.** SCOPE.md and WORKFLOW.md are job-level; the
  agent reads them every stage. If a stage needs its own doc, the
  user inlines a "## Stage N" section into WORKFLOW.md. A real
  per-stage docs surface lands only if real runs show this isn't
  enough.
- **Templating in markdown.** No `{{variable}}` substitution. If the
  user wants the agent to see the current git ref, they paste it.
- **Cross-job docs.** Each job's directory is its own. A repo-wide
  AGENTS.md or similar lives in
  [`CODELESS.md`](../codeless/CODELESS.md), not in the job tree.
- **PR templates / CI hooks.** The `.codeless/jobs/<name>/` directory
  is for *agent input*, not workflow automation. If you want a PR
  template, put it where `gh pr create` looks for it.

## Why this is the right shape

- **Editable in the UI, version-controlled in the repo.** Two
  things the user needs: a place to write down intent and a way to
  see how it evolved. Markdown files in `git` give both for free.
- **The agent reads what the user wrote.** No translation layer, no
  configuration UI for "what should the agent know"; the rule is
  "every `.md` in your job directory."
- **Migration is free.** Legacy flat-YAML jobs keep working; the
  first edit through the new surface promotes them. Nothing to
  re-bootstrap.
- **Same trust boundary.** Single-tenant, single-user. The files
  land in the user's repo and `git diff` is the audit.

## What this rebuilds

The full feature was implemented in one session and lost to a
mis-targeted `git reset --hard`. The new files survived:

- `crates/codeless-runtime/src/job_dir.rs` — resolver + filename
  sanitiser + `read_docs_for_prompt`. 7 unit tests.
- `crates/codeless-runtime/tests/job_dir_workflow.rs` — 6 integration
  tests covering directory layout creation, SCOPE.md round-trip,
  flat→directory migration, `template.yaml`-reserved enforcement,
  path-traversal rejection.
- `ui/codeless-ui/src/modules/jobs/JobFilesPane.tsx` — the Spec pane
  with file list + inline CodeMirror editor + new-file dialog.

What needs rebuilding:

- The four RPC methods in `codeless-runtime/src/rpc.rs`
  (`list_job_files`, `read_job_file`, `write_job_file`,
  `delete_job_file`) plus the migration logic inside
  `update_job_template`.
- The wire types in `codeless-rpc/src/methods.rs` and the trait
  declarations in `server.rs`.
- The HTTP routes in `codeless-server/src/routes.rs`.
- The `HttpRpcClient` impls.
- The TS surface (`methods.ts`, `index.ts`) and the `MockRpcClient`
  cases.
- The `JobPage.tsx` rewire from `TemplateEditor` to `JobFilesPane`
  with a renamed "Spec" tab.
- The `read_docs_for_prompt` call from `job_driver_loop.rs` so the
  scope/workflow/extras actually reach the prompt.

This doc captures the design verbatim. The next session has a
self-contained spec to rebuild against.
