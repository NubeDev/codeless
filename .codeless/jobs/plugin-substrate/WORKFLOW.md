# Workflow — plugin-substrate

## Sequencing

PS2 -> PS3 -> PS4 form one logical block: the Assistant chat surface
moves to one server-resident shape. The REVIEW gate after PS4 must
confirm that block is solid before the persona/plugin layers depend
on it.

PS5 -> PS6 -> PS7 -> PS8 form the substrate proper: persona model,
manifest, attachment-result rendering, agent loop. PS5 must land
before PS6 (manifest references personas) and before PS8 (agent loop
reads persona). PS7 may land in parallel with PS8 conceptually but
keep it sequential to avoid review thrash.

PS-NOTES must NOT start until the REVIEW after PS8 passes. The whole
point of plugin #0 is to prove items 1-8 wired together; running it
on a half-built substrate produces noise, not signal.

PS-ACCEPT is the gate: integration-test coverage per substrate item
plus notes e2e plus the doc update. No coverage = no acceptance.

## Per-stage discipline

Each stage:

1. Re-read [DOCS/PLUGIN-SUBSTRATE.md](../../DOCS/PLUGIN-SUBSTRATE.md)
   and the relevant sub-sections of
   [DOCS/ASSISTANT-SCOPE.md](../../DOCS/ASSISTANT-SCOPE.md) and
   [DOCS/SCOPE.md](../../DOCS/SCOPE.md).
2. Write the test first when the stage introduces logic (R5 from
   CLAUDE.md). Unit tests live with the code; integration tests
   live under the relevant crate's `tests/`.
3. Run all three gates before committing: `cargo test --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cargo
   fmt --check`.
4. Commit and push (see block below).

## REVIEW gate behaviour

The two REVIEW gates pause the next stage; they do NOT skip the
commit + push for the stage that produced the gate. Write into the
handover at every REVIEW:

- which substrate item(s) the stage closed
- what the next stage assumes about this one
- any deferred work the reviewer needs to weigh before unpausing

## Anti-patterns specific to this job

- DO NOT introduce a new agent runtime under any name. PS8 reuses
  the existing `ai-runner`. The substrate doc is explicit.
- DO NOT let a plugin crate import `std::process` or
  `tokio::process` (R1).
- DO NOT let the `notes` plugin grow domain complexity. Its job is
  to exercise items 1-8 with the smallest possible footprint.
- DO NOT add WASI loading, libloading, or any dynamic plugin path.
  Static linking is the MVP shape; revisit later.
- DO NOT slip `fs.read` into a persona's `allowed_tools` example.
  Use `attachments.read`; raw FS access requires explicit reviewer
  sign-off.
- DO NOT loosen the `<plugin_id>_*` table-name check to a runtime
  warning. The static check at load time is the contract.

## Commit + push after every stage

At the end of every stage - including stages that precede a REVIEW
gate, including stages that only edit docs - the agent MUST:

1. Stage every change the stage produced (`git add -A` from the
   worktree root, or specific paths if the stage was surgical).
2. Commit with the message `stage N: <one-line title from
   template.yaml>` so the history mirrors the template stages
   one-for-one.
3. Push to the job's branch (`codeless/plugin-substrate`) so the
   work is recoverable even if the worktree is wiped.

A stage is not done until the push succeeds. If the commit or push
fails, fix the cause and retry - do not mark the stage `[x]`, do
not advance, and never `--force` or `--no-verify`. If a stage
genuinely produced no change, say so in the handover and skip the
commit, but the next stage's commit must include any side-effect
files the investigation touched.
