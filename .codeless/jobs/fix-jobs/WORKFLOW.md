# Workflow — fix-jobs

How to drive the stages in `template.yaml`. Read this and `SCOPE.md`
before every stage.

## Sequencing

- Stages 1–3 are investigate + write failing test. **No production
  code changes** in these stages — only the test file and SCOPE.md
  updates. Each commits its own small diff.
- Stage 4 is a REVIEW gate. The reviewer must see the failing test and
  the §"Root cause" section filled in *before* any fix lands.
- Stage 5 is the fix. One commit per distinct root cause if the
  investigation surfaced more than one.
- Stage 6 verifies in the running app and records the result.
- Stage 7 is the final REVIEW gate. Do not auto-merge.

## Per-stage discipline

Before any work in a stage:

1. Re-read `SCOPE.md` §"In scope" and §"Constraints". If the stage
   demands something not in scope, stop and surface it — don't expand
   silently.
2. Re-read the relevant `JobDetailStack` and `JobPage` files. The
   "render every job-detail tab simultaneously" contract is the load-
   bearing design constraint; the fix must preserve it.

Before committing:

1. `pnpm -C ui/codeless-ui typecheck` green.
2. `pnpm -C ui/codeless-ui lint` green.
3. Tests added in the stage actually fail on `master` (stage 3) or
   pass after the fix (stage 5). Confirm both, don't assume.
4. SCOPE.md is updated in the same commit when the stage produces a
   `§Reproduction`, `§Root cause`, or `§Manual verification` entry.

Commit + push via **mani** from the workspace root:

```
./bin/mani --config mani.yaml run commit --projects codeless \
  MSG='stage N: <one-line title>'
./bin/mani --config mani.yaml run push --projects codeless
```

No `--force`, no `--no-verify`. If a hook fails, fix the cause.

## REVIEW gates

Two gates: stage 4 (post-investigation, pre-fix) and stage 7 (final).

At each gate, write a handover comment in the job chat with:

- Stage 4: the §"Root cause" text from SCOPE.md, the failing-test
  path, and the `pnpm test` output showing it fails on `master`.
- Stage 7: the diff summary, the §"Manual verification" text, and a
  link to the PR.

Do not proceed past a gate without explicit approval.

## Anti-patterns specific to this job

- **Do not** "fix" the bug by serialising — collapsing to one mounted
  JobPage at a time defeats the design and silently regresses tab-
  switch latency.
- **Do not** key shared singletons by "the last mounted jobId". That
  trades a visible bug for a heisenbug.
- **Do not** add `key={jobId}` to force a remount. Same reason —
  hides the underlying bug, breaks instant-switch.
- **Do not** edit JobPage's render gates or the App.tsx wrapper's
  `invisible` class as the "fix" unless the investigation explicitly
  named one of those as the root cause. Pattern-match-fixing render
  gates without understanding why the bug happens is exactly how
  regressions like this one get introduced.
- **Do not** start fixing in stage 3. Stage 3 only writes the failing
  test. The fix is stage 5, after the REVIEW gate.

## When to halt

- The failing test won't fail on `master` after a real attempt:
  surface this — your reproduction is wrong, not the bug.
- The root cause turns out to be in the Rust backend: stop, document,
  do not expand scope.
- Any constraint in §"Constraints" of SCOPE.md cannot be honoured by
  the fix you've found: stop and surface.
