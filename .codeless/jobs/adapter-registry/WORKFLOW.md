# Workflow — adapter-registry

How to drive the stages in `template.yaml`. Read this before every
stage, alongside `SCOPE.md` and the workspace
[`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md)
§"TODO — adapter registry".

## Sequencing

- Stages 1 and 2 are decision-only — **no code commits**. Stage 1
  edits `SCOPE.md` (and the workspace `DOCS/WORKSPACE-ATTACH.md`
  TODO section); stage 2 is a REVIEW gate. Do not start stage 3
  until the gate is approved.
- Stages 3 through 8 may not be batched. Each one ships its own
  commit so the diff is a coherent unit and a revert is one commit.
- Stage 9 is the final REVIEW gate. Do not auto-merge.

## Per-stage discipline

Before writing any code in a stage:

1. Re-read `SCOPE.md` §"In scope" and §"Constraints". If the stage
   demands something not in §"In scope" (Gmail, the Settings UI,
   `Arc<ArcSwap<…>>` hot-reload), stop and surface it — do not
   silently expand scope.
2. Re-read the relevant section of
   `DOCS/WORKSPACE-ATTACH.md` §"TODO — adapter registry". The
   workspace doc is authoritative; this job's `SCOPE.md` is a brief.
3. Check the R1 boundary: any new `use` of `std::process` /
   `tokio::process` must be in `codeless-adapters-host`. Grep
   before committing:
   `rg 'tokio::process|std::process' crates/ --type rust`
   The match set outside `codeless-adapters-host` must not grow.
   `restart_server`'s `exec()` path is the only legitimate addition
   and lives in the host adapter.

Before committing a stage:

1. `cargo test --workspace` green.
2. `cargo clippy --workspace --all-targets -- -D warnings` green.
3. `cargo fmt --check` green.
4. The stage's test(s) actually exercise the new behaviour, not just
   compile against it. For stage 4 specifically: the
   write-then-fsync-then-restart test must actually crash the
   process between the secrets write and the restart signal, not
   just assert the file exists on disk.
5. Update `SCOPE.md` §"Deliverables" with a `[x]` against anything
   completed in the stage.

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

1. `checks` — run the stage's `verify:` list (or `verify_cmd`).
   Every step must pass. On failure: stop, fix, re-run; do not
   advance to `docs`.
2. `docs` — update `handover.md` for the next stage and the active
   session doc, in the same worktree, so the fresh agent that opens
   the next stage has the context it needs.
3. `git` — stage the changes, commit with the message
   `stage N: <one-line title from template.yaml>`, and push to
   `codeless/adapter-registry` so the work is recoverable.

A stage is not "done" until all three are green and the push
succeeds.

## REVIEW gates

Two gates: stage 2 (decisions) and stage 9 (server-side complete).

At each REVIEW gate, write a handover comment in the job chat with:

- One bullet per item the gate is checking.
- For stage 2: the chosen answer for each open question, with a
  one-line *why*, and a diff-link line for
  `DOCS/WORKSPACE-ATTACH.md` if the doc changed.
- For stage 9: the `cargo test --workspace` output (or its tail),
  the list of new RPC methods, and a one-paragraph note on what's
  intentionally deferred to the UI and Gmail follow-up jobs.

Do not proceed past a REVIEW gate without explicit approval in chat.

## Anti-patterns specific to this job

- **Do not** start the Settings UI work. The entire `ui/codeless-ui/`
  tree is off-limits in this job; the follow-up UI job consumes the
  RPC types this job ships.
- **Do not** add the Gmail adapter. `codeless-gmail` is a separate
  crate landing in a separate job; mentioning it in code in this job
  is scope creep.
- **Do not** wrap `DefaultRunnerFactory.enable_*` in `AtomicBool` /
  `ArcSwap`. That is stage 2 (hot-reload) and is explicitly
  deferred. Stage 8 lifts the fields into a `RunnerConfig` struct
  read once at boot from SQLite; the field semantics stay
  "constructed at boot, replaced by restart".
- **Do not** key `chat_adapters` on a single-column PK. The
  composite `(kind, instance_id)` is load-bearing per peer review;
  collapsing to `kind` forecloses Slack-personal + Slack-work
  without a future migration.
- **Do not** accept `set_chat_adapter_enabled(true)` without a
  prior successful validate. The structured `MissingSecrets`
  return value is the only correct refusal path; a generic
  `Conflict` is wrong.
- **Do not** trust filesystem perms as secrets protection. The
  `SecretBackend` trait is the point of indirection; if the
  keyring backend is a follow-up, ship it behind a feature flag and
  default to TOML — do not stub the keyring path with a fake.
- **Do not** unify the two new tables into one
  `enabled_components(kind, id, enabled, config_json)` table
  without surfacing the decision in chat. The peer review left this
  open; the bias is "two tables now, collapse later if config_json
  stays small". Either is defensible; silently picking one is not.
- **Do not** make `restart_server` succeed unconditionally. The
  partition into `resumable` vs `killed` plus the `force: true`
  escape is the whole point; a footgun verb is worse than no verb.
- **Do not** introduce per-workspace bearer tokens. R5: one trust
  boundary, one token, all RPCs.

## When to halt

- `cargo test --workspace` fails after a real fix attempt and you
  can't see the next move: mark the stage `[!]` in `SCOPE.md` and
  stop. Do not commit a partial implementation with a TODO.
- A stage's work turns out to require a decision that wasn't in
  stage 1's resolved list: stop, surface the decision in chat, do
  not silently choose.
- Any R1 grep regression (new `tokio::process` outside
  `codeless-adapters-host`): halt and rework the layering. R1 is
  not negotiable.
- The peer-review TODO at the bottom of
  `DOCS/WORKSPACE-ATTACH.md` grows a new exit-test requirement
  while this job is in flight: halt at the next stage boundary and
  reconcile before continuing.
