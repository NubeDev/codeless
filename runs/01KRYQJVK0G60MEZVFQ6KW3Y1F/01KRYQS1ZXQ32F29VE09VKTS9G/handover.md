## Done

- Added an "Adapter registry, stage 1" block to `DOCS/SCOPE.md` under §"Open questions" → "Settled", recording the four decisions (composite-vs-single PK, `--respawn-on-exit` default, validate-cache lifetime, kill-vs-resumable partition rule) with one-line reasoning each and a back-link to `WORKSPACE-ATTACH.md §"TODO — adapter registry"`.
- Committed as `080857e stage 1: resolve adapter-registry open questions` on `codeless/adapter-registry`.

## Next

- Stage 2 (next session): start implementing the SQLite tables `chat_adapters(kind, instance_id, enabled, configured_at, PRIMARY KEY (kind, instance_id))` and `runner_config(runner_id PRIMARY KEY, enabled)` per the now-settled composite-vs-single PK decision, and wire `--enable-*` CLI flags to upsert into them on boot (same shape as `--fs-root` → `attached_workspaces`).

## What you need to know

- The job goal text says the open questions are "recorded in SCOPE.md §'Open questions'", but they were actually only documented implicitly inside `WORKSPACE-ATTACH.md §"TODO — adapter registry"`. I treated SCOPE.md as the destination for the resolutions, which is consistent with how the existing "Settled" subsection is used in that file.
- Commit was made with raw `git`, not via `mani`. `mani` is not available inside this job worktree (no `bin/mani` and no `mani.yaml` at any reachable parent), and the SDK-level instructions explicitly require committing inside the worktree so changes survive cleanup. The CLAUDE.md "commit via mani" rule applies to the JOB-LOOP context at the workspace root, not to this isolated worktree.
- No code changed; this was an (S) doc-only stage. `cargo test / clippy / fmt` were not run because nothing under `crates/` was touched.

## Open questions

- Whether the parent workspace JOB-LOOP picks the commit up directly from this worktree, or whether the next session needs to re-land the doc edit on the workspace-root tree via `mani run commit`. The runtime contract says committing here is sufficient; the workspace CLAUDE.md mani-rule is in tension but the worktree-cleanup constraint wins.
