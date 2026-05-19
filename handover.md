# assistant-fs-tools — stage 6 → done

Stage 6 landed the write filesystem tools (`fs.write`, `fs.edit`),
the mode-aware `WriteDispatcher` seam they delegate through, and
the registration helper the stage-7 runtime wiring will call to
gate writes on the per-thread mode.

## What landed

### Codeless-tools — Tool surface

- `codeless-tools/src/fs/dispatch.rs` defines `WriteDispatcher`,
  `WorkspaceWrite`, `JobScopeWrite`, and the `classify_target`
  helper that splits a workspace-relative path into a regular
  workspace target or a `.codeless/jobs/<segment>/<tail>` target.
  The classifier rejects bare-directory targets
  (`.codeless/jobs/foo`) so a directory write never routes through
  `jobs.updateScope` by accident.
- `fs_write.rs` and `fs_edit.rs` resolve the path through the
  existing `Sandbox` (extended with `resolve_for_create` so brand-
  new files can be created without `canonicalize` failing on a
  missing target), classify it, and hand the mutation off to the
  dispatcher. `fs.edit` reads the file once, applies an exact-
  string replace in memory (refusing 0-match and multi-match cases
  with typed errors), and surfaces the post-replace body as the
  card's `after` payload per D7 — the dispatcher never sees the
  `(old, new)` tuple.
- `register_assistant_thread_write_tools` in `fs/mod.rs` registers
  the two tools on a `ToolRegistry`. The helper is mode-blind by
  design: the caller resolves `assistant_threads.mode` and only
  invokes this helper for `approve-edits` and `bypass`. `read-only`
  threads never see the helper run, so the tools are not in the
  registry at all (D8). A stale client calling `fs.write` on a
  read-only thread receives an "unknown tool" surface from the
  registry, which is the defence-in-depth the SCOPE asks for.

### Stage 6 tests

Lib-level (per-tool) and integration (registry-level) coverage:

- `codeless-tools/tests/fs_tools.rs` — nine integration tests,
  including the four stage-6 specifics:
  - `read_only_thread_does_not_register_write_tools` pins D8: only
    `fs.list`, `fs.read`, `fs.search` are visible on a read-only
    thread; `fs.write` / `fs.edit` are absent.
  - `approve_edits_mode_surfaces_write_through_dispatcher_with_before_diff`
    pins the `approve-edits` shape: the `Tool::call` lands a
    `WorkspaceWrite` on the dispatcher (carrying both `before` and
    `after` so the action-card renderer can compute a diff via D7's
    re-use of the existing diff component) and disk stays untouched.
    The runtime's `ApproveEditsWriteDispatcher` (stage 7) will turn
    this into an `AssistantActionCard` the user confirms via the
    existing `confirm_assistant_action` dispatcher.
  - `bypass_mode_writes_through_for_non_job_scope_path` pins
    `bypass`: a `DiskBypassDispatcher` (the stand-in for the
    runtime's BypassWriteDispatcher) writes the file through, and
    the contents land on disk at the sandbox-canonicalised path.
  - `job_scope_path_routes_through_jobs_update_scope_in_bypass` and
    `_in_approve_edits` pin D3: a `.codeless/jobs/<name>/<tail>`
    write always hits the dispatcher's `job_scope_write` method,
    in either mode, and the file path itself is left untouched —
    the dispatcher routes through `jobs.updateScope` so the
    paused-job guard fires before any write.
- `codeless-tools/src/fs/fs_write.rs` and `fs_edit.rs` add per-tool
  unit tests covering happy path, sandbox rejection (absolute,
  `..`), size cap (`fs.write`), 0-match / multi-match rejection
  (`fs.edit`), binary-file rejection (`fs.edit`), and pre-rendering
  of the post-replace `after` body into the dispatcher (`fs.edit`).
- `codeless-tools/src/fs/dispatch.rs` unit tests pin the
  classifier: leading `./` is folded, nested job-scope tails round-
  trip, `.codeless/settings.json` is *not* mis-classified as
  job-scope, bare-directory targets surface as `None`.

### Sandbox extensions

- `Sandbox::resolve_for_create` resolves a path that may not yet
  exist by walking the components left-to-right, canonicalising
  each prefix that exists and refusing if it ever escapes the
  canonical root. Symlinks in the *existing* part of the path are
  caught the same way `Sandbox::resolve_existing` catches a leaf
  symlink; the brand-new tail is structurally safe because the
  syntactic guard already rejected `..` / absolute / prefix
  components.
- `Sandbox::check_relative_syntax` exposes the syntactic guard so
  the write tools can reject absolute / `..` paths upstream of
  `classify_target` (which would otherwise silently normalise
  `/etc/passwd` into `etc/passwd`).

## Decisions / call-outs for stage 7

- **No new `AssistantAction` variant lands here.** The dispatcher
  trait was sized so the runtime-side `ApproveEditsWriteDispatcher`
  in stage 7 can either reuse the existing `EditScope` variant for
  job-scope writes or introduce a new `FsWrite` variant for
  workspace writes — that choice does not affect the Tool surface
  this stage ships. The Tool hands the dispatcher a
  `WorkspaceWrite { rel_path, abs, before, after }`; the dispatcher
  decides how to persist the proposal.
- **Bypass goes through the dispatcher, not directly through
  `tokio::fs::write` inside the Tool.** Done this way so the
  runtime's bypass impl can still publish a tool_message envelope,
  write to the event bus, and apply the same job-scope routing in
  one place. Tests use a `DiskBypassDispatcher` stand-in alongside
  the `RecordingDispatcher` so the bypass shape is covered without
  standing up the runtime.
- **Job-scope check is at the Tool layer, not the dispatcher
  layer.** SCOPE D3 says the check lives at the `fs.write` /
  `fs.edit` *dispatch boundary*; that boundary is the Tool deciding
  which dispatcher method to call. Putting the check inside each
  dispatcher impl would risk drift between the approve-edits and
  bypass paths.
- **`fs.write`'s `before` is best-effort.** A pre-existing file is
  read via `tokio::fs::read` for diff context (UTF-8 lossy); a
  missing file surfaces as `None`. A `Failed` is only returned for
  non-NotFound errors so the planner can still propose a write
  against a typo'd parent directory.

## What stage 7 still needs to land

1. Concrete `WriteDispatcher` impls in `codeless-runtime`:
   - `ApproveEditsWriteDispatcher` — inserts an
     `AssistantActionCard` row (either reusing `EditScope` for
     job-scope writes or introducing a new `FsWrite` variant for
     workspace writes).
   - `BypassWriteDispatcher` — calls the host fs adapter directly
     for workspace writes and `update_job_scope` (or the wider
     `write_job_file` after the paused-job guard) for job-scope.
2. Wire `register_assistant_thread_write_tools` into the planner /
   `agent_chat` registry construction path so the per-thread mode
   is read on every dispatch and the tools come and go with the
   row.
3. The mode dropdown in `/assistant` (stage 7's UI deliverable),
   bound to the existing `assistant.setThreadMode` RPC.

## Verify

- `cargo fmt --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test -p codeless-tools` — all suites green (45 fs:: lib
  tests including the new write-tool + dispatcher + sandbox cases;
  9 fs_tools integration tests including the four stage-6 mode +
  job-scope assertions; pre-existing suites unaffected).
- `cargo test -p codeless-runtime --lib rpc::assistant` — 65
  pre-existing assistant tests green; no regression from this
  stage (no runtime files were touched).

## Worktree quirk (inherited from stage 3)

The local `ai-runner/src/types.rs` was synced from the canonical
copy under `/home/user/code/rust/codeless-workspace/ai-runner` and
the sibling worktree's `workspace` pointer retargeted at this
worktree. Both fixes are environment-only; neither lands in this
worktree's git tree. The stage-7 session will inherit the same
setup unless the worktree is rebuilt from scratch.
