# Workflow — workspace-attach

How to drive the stages in `template.yaml`. Read this before every
stage, alongside `SCOPE.md` and the workspace
[`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md).

## Sequencing

- Stages 1 and 2 are decision-only — **no code commits**. Stage 1
  edits `SCOPE.md` (and the workspace `DOCS/WORKSPACE-ATTACH.md`); stage
  2 is a REVIEW gate. Don't start stage 3 until the gate is approved.
- Stages 3-7 may not be batched. Each one ships its own commit so the
  diff is a coherent unit and a revert is one commit.
- Stage 8 is the final REVIEW gate. Do not auto-merge.

## Per-stage discipline

Before writing any code in a stage:

1. Re-read `SCOPE.md` §"In scope" and §"Constraints". If the stage
   demands something not in §"In scope", stop and surface it — don't
   silently expand scope.
2. Re-read the relevant section of `DOCS/WORKSPACE-ATTACH.md`. The
   workspace doc is authoritative; this job's `SCOPE.md` is a brief.
3. Check the R1 boundary: any new `use` of `std::process` /
   `tokio::process` must be in `codeless-adapters-host`. Grep before
   committing:
   `rg 'tokio::process|std::process' crates/ --type rust`
   The match set outside `codeless-adapters-host` must not grow.

Before committing a stage:

1. `cargo test --workspace` green.
2. `cargo clippy --workspace --all-targets -- -D warnings` green.
3. `cargo fmt --check` green.
4. The stage's test(s) actually exercise the new behaviour, not just
   compile against it. For stage 3 specifically: the canonicalisation
   test must assert three forms collapse to one row, not just that one
   row was inserted.
5. Update `SCOPE.md` §"Deliverables" with a `[x]` against anything
   completed in the stage.

Commit + push via **mani** from the workspace root:

```
./bin/mani --config mani.yaml run commit --projects codeless \
  MSG='stage N: <one-line title>'
./bin/mani --config mani.yaml run push --projects codeless
```

No `--force`, no `--no-verify`. If a hook fails, fix the cause.

## REVIEW gates

Two gates: stage 2 (decisions) and stage 8 (server-side complete).

At each REVIEW gate, write a handover comment in the job chat with:

- One bullet per item the gate is checking.
- For stage 2: the chosen answer for each open question, with a
  one-line *why*, and a diff-link line for `DOCS/WORKSPACE-ATTACH.md`
  if the doc changed.
- For stage 8: the `cargo test --workspace` output (or its tail), the
  list of new RPC methods, and a one-paragraph note on the host
  adapter switch (was `Option<PathBuf>`, now allowed-roots list).

Do not proceed past a REVIEW gate without explicit approval in chat.

## Anti-patterns specific to this job

- **Do not** add `default_runner: String`. Use the `RunnerId` newtype
  if one exists; if not, add it in `codeless-types` as part of stage 4
  rather than punting to a `String`. The doc was updated to forbid the
  stringly-typed form.
- **Do not** key the unique index on `fs_root_display`. The whole
  point of the two-column schema is that the canonical column is the
  source of truth for "is this already attached".
- **Do not** detect missing workspaces only on the next `fs.*` call.
  The 30s liveness sweep is in scope (stage 7) precisely because lazy
  detection lets a user's stale workspace sit silently broken.
- **Do not** drift the `--fs-root` flag's semantics. It becomes a
  bootstrap upsert; it does *not* dynamically reflect the attached set
  (`ServerInfo.fs_root` is frozen — see SCOPE.md §Goal).
- **Do not** introduce per-workspace bearer tokens. R5: one trust
  boundary, one token, all RPCs.
- **Do not** start UI work. The whole UI surface is a follow-up job;
  if a stage tempts you toward `ui/codeless-ui/`, stop.
- **Do not** treat `Conflict` as a single error variant. The doc
  introduces `WorkspaceError` precisely so the UI doesn't string-match.

## When to halt

- `cargo test --workspace` fails after a real fix attempt and you
  can't see the next move: mark the stage `[!]` in `SCOPE.md` and
  stop. Do not commit a partial implementation with a TODO (R4 in
  codeless/CLAUDE.md).
- A stage's work turns out to require a decision that wasn't in stage
  1's resolved list: stop, surface the decision in chat, do not
  silently choose.
- Any R1 grep regression (new `tokio::process` outside
  `codeless-adapters-host`): halt and rework the layering. R1 is not
  negotiable.
