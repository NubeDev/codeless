# Workflow — worktree-fs-jail

How to drive the stages in `template.yaml`. Read this before every
stage, alongside `SCOPE.md` and the authoritative bug doc at
[`crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md`](../../../crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md)
§"Known issues — Worktree root is not in the fs jail".

## Sequencing

- Stage 1 is read-only: reproduce, locate the exact lines, resolve
  the three open questions. No production code, no commits beyond
  the SCOPE.md handover edits.
- Stage 2 lands the fix in both hosts in **one commit** — the same
  two-line shape applies to both, and the diff is more honest as
  one change than as two. Tests for the fix land in stage 3, not
  bundled here.
- Stage 3 lands the regression tests. The unit test and the
  integration test are one commit; they prove the same thing at
  different layers.
- Stage 4 closes BROWSER-LAUNCHER.md and records the actual line
  ranges. No code beyond the doc edit.

There are no REVIEW gates. The fix is mechanical and the doc is
already peer-reviewed.

## Per-stage discipline

Before writing any code in a stage:

1. Re-read `SCOPE.md` §"In scope" and §"Out of scope". The biggest
   risk on this job is scope creep — `HostFs` and the surrounding
   boot code are tempting to refactor. Do not.
2. Re-read the relevant section of `BROWSER-LAUNCHER.md`. It names
   the exact files, the exact line ranges, and the exact fix.
3. `cargo check --workspace` before any edit so the baseline is
   known-clean.

Before committing a stage:

1. `cargo fmt --all` clean.
2. `cargo clippy --workspace --all-targets -- -D warnings` green.
3. `cargo test --workspace` green.
4. The stage's new tests (stage 3) actually fail without the fix —
   verify by temporarily reverting the stage-2 commit on a scratch
   branch, confirming the tests fail, then restoring. Record the
   confirmation in the handover.

Commit + push via **mani** from the workspace root:

```
./bin/mani --config mani.yaml run commit --projects codeless \
  MSG='stage N: <one-line title>'
./bin/mani --config mani.yaml run push --projects codeless
```

No `--force`, no `--no-verify`. If a hook fails, fix the cause.

## Closing trio — the last three todos of every stage

Every stage's todo checklist ends with the same three items, in
order. The user watches these tick over in the `Stages` overview;
they are how the user confirms a long-running stage actually
landed instead of just looking like it did. Do **not** rename or
reorder them.

1. `checks` — run the stage's `verify:` list (or the cargo trio
   above). Every step must pass. On failure: stop, fix, re-run;
   do not advance to `docs`.
2. `docs` — update `handover.md` for the next stage and the active
   session doc, in the same worktree, so the fresh agent that
   opens the next stage has the context it needs.
3. `git` — stage the changes, commit with `stage N: <title from
   template.yaml>`, push to `codeless/worktree-fs-jail`. The
   history mirrors the template stages one-for-one.

A stage is not "done" until all three are green and the push
succeeds. If `checks` or `git` fails, fix the cause and retry — do
not mark the stage `[x]`, do not advance, and never `--force` or
`--no-verify`.

## Anti-patterns specific to this job

- **Do not** refactor `HostFs`, `is_path_allowed`, or the
  allowed-roots data structure. The fix is two `add_root` calls;
  anything beyond that is a separate job.
- **Do not** add a new RPC, field on `ServerInfo`, or CLI flag
  exposing the worktree root. The doc is explicit: it is internal.
- **Do not** generalise the fix into a `register_internal_roots()`
  helper. Three similar lines is better than a premature
  abstraction; there are exactly two call sites and they both fit
  on one line each.
- **Do not** add a TODO for "consider unifying worktree-root and
  attached_workspaces handling". If unification matters, write a
  separate job; do not leave breadcrumbs in production code.
- **Do not** weaken the integration test by mocking `HostFs`. The
  test exists precisely to catch the bug at the boot-time
  composition layer; a mock at that layer defeats the purpose.
- **Do not** delete the `BROWSER-LAUNCHER.md` §"Known issues"
  entry in stage 4. Flip the status line and add the line-range
  note; the section is history for the next reader and earns its
  keep even after the fix lands.
- **Do not** start work on any other BROWSER-LAUNCHER milestone
  while this job is open. The dogfood path in that doc is
  decoupled from this fix on purpose; bundling them is how this
  lands at 5x cost.

## When to halt

- The reproduction in stage 1 does not match the doc's symptom
  exactly (different error message, different file, different
  call site). Surface in chat; the doc may have drifted and the
  fix shape may need to change.
- `HostFs::add_root` turns out to require the path to be a real
  directory at call time, and the `create_dir_all` ordering in
  one of the two hosts does not guarantee that. Surface and
  resolve before stage 2.
- The integration test harness in neither `codeless-cli` nor
  `codeless-tauri-desktop` is suitable, and a new harness would
  be a non-trivial build. Surface in chat; the unit test in
  stage 3 may have to carry the load alone, with the integration
  test deferred to a follow-up.
- Any R1 violation (a `process::Command` import sneaking into a
  mobile-safe crate, an `add_root` call landing in
  `codeless-runtime` instead of the host crates). Halt and
  rework.
