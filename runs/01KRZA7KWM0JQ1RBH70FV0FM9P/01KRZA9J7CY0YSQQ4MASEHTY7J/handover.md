## Done

- Filled in the "Decisions" section of `.codeless/jobs/assistant-fs-tools/SCOPE.md` with eight resolutions: D1–D3 confirm the load-bearing decisions called out in the job goal (mode names `read-only` / `approve-edits` / `bypass`; `mode TEXT NOT NULL DEFAULT 'read-only'` column on `assistant_threads`; `.codeless/jobs/<name>/` always routes through `jobs.updateScope`, including in `bypass`). D4–D8 resolve the five numbered open questions per their stated biases (canonicalising sandbox, 5 MiB read cap, 200-match search cap with truncation flag, action-card payload reuses the existing `jobs.updateScope` diff card, `read-only` hides write tools from the planner registry with server-side rejection as defence-in-depth).
- Added an "Implications for later stages" coda mapping each decision to the stage that consumes it (stage 3 → D1/D2, stage 4 → D4/D5/D6, stage 6 → D3/D7, stage 7 → D1).
- Committed as `363ee95 stage 1: resolve open questions in SCOPE.md` on branch `codeless/assistant-fs-tools`.

## Next

- WORKFLOW.md sets a REVIEW gate after stage 1: human sign-off on the decisions before any code lands. The next session should wait for that gate to clear, then start stage 3 (`assistant_threads.mode` column + migration + `assistant.setThreadMode` RPC + round-trip tests).
- Stage 2 in `template.yaml` is the REVIEW gate itself (no code).

## What you need to know

- Stage 1 is prose-only per WORKFLOW.md ("Stage 1 is **prose-only**. … No code."). The commit touches only `SCOPE.md`; no cargo/pnpm runs were required and none were done.
- Decision D1 fixes the wire vocabulary at exactly three strings — stage 3's `AssistantThreadMode` enum must accept only those three and stage 3's migration must not add a `CHECK` constraint (D2 explains why).
- Decision D3 places the job-scope routing check at the `fs.write` / `fs.edit` *dispatch* boundary in `codeless-runtime`, not inside the `Tool` impl in `codeless-tools`. This is load-bearing for stage 6: the routing decision needs the thread row, the workspace root, and the resolved path together, which the runtime sees and the tool crate does not. Stage 6's test plan must include a `bypass`-mode regression test for the `.codeless/jobs/<name>/` route — that test is the reason D3 exists.
- Decision D8 means stage 4's tool-registry construction reads the thread's `mode` column. Stage 4 must wire that read; the registry is built per tool-call boundary, not once at thread creation.
- The `bin/mani` tooling referenced in WORKFLOW.md is not present in this isolated worktree (the harness creates the worktree without symlinks to workspace-root tools). Stage 1 committed with plain `git`; later stages may need the same fallback, or the harness will need to bind-mount `mani` into the worktree.
- `handover.md` at the repo root is stale (it describes the previous `workspace-scoping` job's stage 6). It was not touched in this stage; the WORKFLOW "closing trio" item that updates `handover.md` belongs to stages that change code, and the current stage is prose-only. A future stage 3 author may want to overwrite it.

## Open questions

- (none) — all eight SCOPE.md open questions are resolved in the "Decisions" section. Any reversal must amend that section in the same commit as the motivating code change.
