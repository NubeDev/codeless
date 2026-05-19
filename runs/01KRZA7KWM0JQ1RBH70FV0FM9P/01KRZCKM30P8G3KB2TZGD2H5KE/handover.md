## Done

- Implemented `fs.write` and `fs.edit` Tool impls in `codeless-tools/src/fs/`, both delegating mutation to a new mode-aware `WriteDispatcher` trait (`dispatch.rs`).
- `Sandbox` extended with `resolve_for_create` (canonicalises each existing prefix to catch symlink escapes) and `check_relative_syntax` (pure-syntactic guard called upstream of `classify_target`).
- `classify_target` splits a workspace-relative path into a regular `Workspace` target or a `JobScope { segment, tail }` target; bare-directory job-scope paths are rejected so a directory write cannot accidentally route through `jobs.updateScope`.
- `register_assistant_thread_write_tools` registers fs.write/fs.edit on a tool registry; mode gating lives at the caller (read-only never calls it → tools are absent from registry per D8).
- Per-tool unit tests + nine integration tests in `crates/codeless-tools/tests/fs_tools.rs` covering all three modes (read-only excludes tools, approve-edits surfaces dispatcher call with before/after, bypass writes through) plus the `.codeless/jobs/<name>/` special case in both bypass and approve-edits.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p codeless-tools`, `cargo test -p codeless-runtime --lib rpc::assistant` all green.
- Committed as `c70b1a8` on `codeless/assistant-fs-tools`.

## Next

- Stage 7: concrete `WriteDispatcher` impls in `codeless-runtime` (`ApproveEditsWriteDispatcher` inserting a card row + `BypassWriteDispatcher` writing through / routing to `jobs.updateScope`), planner-side wiring of `register_assistant_thread_write_tools`, and the mode dropdown in `/assistant`. Detailed checklist in handover.md.

## What you need to know

- The Tool layer is intentionally runtime-blind. The dispatcher trait sits in codeless-tools so codeless-runtime can supply the concrete impls in stage 7 without altering the Tool surface.
- Job-scope detection (`.codeless/jobs/<segment>/<tail>`) lives at the Tool layer per SCOPE D3 — the Tool picks `workspace_write` vs `job_scope_write`. Putting the check inside dispatchers would risk drift between modes.
- No new `AssistantAction` variant was added. Stage 7 can either reuse `EditScope` for job-scope writes or add an `FsWrite` variant for workspace writes without touching the Tool surface.
- Tests use `DiskBypassDispatcher` and `RecordingDispatcher` stand-ins; these are not the production dispatchers — those land in stage 7.
- The branch is `codeless/assistant-fs-tools`; commit not pushed (no remote push in the stage scope).

## Open questions

- (none)
