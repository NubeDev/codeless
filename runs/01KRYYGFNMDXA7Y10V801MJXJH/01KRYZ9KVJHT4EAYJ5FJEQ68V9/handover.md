## Done

- Authored §"Known issues — Worktree root is not in the fs jail" in `crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md` immediately before §"Open questions", with `Status: fixed in 168cb7f` and a "Where it bites" bullet listing the two registration sites: `codeless-cli/src/serve.rs:413-422` (pre-existing) and `codeless-tauri-desktop/src/boot.rs:161-169` (added in 168cb7f).
- Committed as `f0e9f8b` on branch `codeless/worktree-fs-jail` with message starting `stage 4 — close out:`.

## Next

- (none) — final stage of the worktree-fs-jail job.

## What you need to know

- The section did not exist on this branch (stages 1 and 2 flagged this in their handovers and in `.codeless/jobs/worktree-fs-jail/SCOPE.md`), so stage 4 authored it in its post-fix form rather than flipping an existing `Status: open` line. The section is retained as history per the stage instruction "do not delete the section".
- Commit SHA recorded in the doc is `168cb7f` (the stage-2 commit that landed the actual desktop-host fix). The CLI half was already on the branch before this job started.
- Used raw `git commit` (no `bin/mani` reachable from this isolated worktree); markdown-only change so workspace `cargo test`/`clippy`/`fmt` gates were skipped.

## Open questions

- (none)
