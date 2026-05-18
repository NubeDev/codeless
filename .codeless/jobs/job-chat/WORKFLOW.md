# Workflow — job-chat

How the agent drives the stages in `template.yaml`. Re-read this file
at the top of every stage; the rules survive stage boundaries.

## Sequencing

The 17 stages cluster into three phases mirroring
[`JOB-CHAT.md` § Recommended sequencing](../../../DOCS/JOB-CHAT.md#recommended-sequencing--c1--c2--c3):

- **C1 — Unified chat substrate (stages 1–8).** Schema, wire types,
  RPCs, events, Web UI, Telegram adapter, echo-suppression helper.
  Two REVIEW gates inside this phase: M-C1-A (substrate compiles
  end-to-end, no transport code yet) and M-C1-B (Telegram round-
  trips with web UI). REVIEW M-C1-B is the hard gate before any
  supervisor work — if it fails, the supervisor stages will compound
  the bug.
- **C2 — Supervisor agent, read-only (stages 9–11).** Module
  scaffold, read-only tool surface, Claude wiring. REVIEW M-C2
  before action tools.
- **C3 — Action tools + pre-armed goals (stages 12–16).** Action
  tools, `supervisor_goals` migration + store, pre-armed loop,
  rehydration, Slack parity. REVIEW M-C3 before documentation.
- **Stage 17 — Documentation + final verification.** Doc updates +
  green-checks across the workspace.

**Do not batch across REVIEW gates.** Each REVIEW gate is a
checkpoint where the operator inspects the worktree, the diff, and
the JOB-CHAT.md status updates. If a REVIEW gate fails, fix the
cause, re-run that stage, do not advance.

**Stage 1 is documentation only.** Resolve the open questions, write
the answers into this job's `SCOPE.md` and propagate them into
`DOCS/JOB-CHAT.md` inline (do not leave the doc's "Open questions"
section talking about decisions that have been made). No Rust, no
TypeScript, no migrations in stage 1.

## Per-stage discipline

Every stage follows the same arc. Read first, write second, verify
third, commit fourth.

### Before writing any code

1. **Re-read this file** (top of every stage).
2. **Re-read [`SCOPE.md`](./SCOPE.md)** — constraints + open
   questions, especially the R1/R2/R3/R4/R5 rules.
3. **Re-read [`DOCS/JOB-CHAT.md`](../../../DOCS/JOB-CHAT.md)** for
   the section the stage is implementing. The doc wins on any
   disagreement; update the doc, do not diverge.
4. **Read the latest [`handover.md`](./handover.md)** if it exists —
   the previous stage's per-stage handover is the load-bearing
   bridge across the stage boundary (per
   [`CLAUDE.md`](../../../CLAUDE.md): anything that must survive
   the boundary is on disk, not in your head).

### While writing

- **Wire-format additivity.** New `Event` variants get
  `#[serde(rename = "kebab-case")]`. New optional fields on existing
  variants get `#[serde(default, skip_serializing_if =
  "Option::is_none")]`. Same rule as #31's
  `TodoCompleted.failure_detail` and `StageTrioGateWaiting`. Older
  replay events must decode unchanged.
- **One concept per file** (R3 of `codeless/CLAUDE.md`). The
  supervisor module is the one structural exception in this job:
  it has `mod.rs`, `prompt.rs`, `tools/{read_state,actions}.rs`,
  `goals.rs`, `rehydrate.rs`. Each file owns one concept.
- **Comments explain *why*, never *what*.** No emojis. No
  task-status comments (`// added in stage 7`, `// fix for OQ-CHAT-1`).
  No restatements. No decorative banners.
- **The supervisor never imports `std::process` or `tokio::process`.**
  Grep-enforced at stage 9. The supervisor's only voice is
  `post_chat_message`; `eprintln`, `println`, and `tracing::info`
  that surfaces to a user are all banned in the module.
- **No `--force`, no `--no-verify`.** If a hook fails, fix the
  cause.

### Tests live with the code (R5 of `codeless/CLAUDE.md`)

- Same commit as the logic. The runtime state machine has unit
  tests per transition; integration tests use in-memory SQLite +
  the `MockRunner` harness. Adapter tests use canned API impls
  (`CannedTelegramApi`, `CannedSlackApi`); never hit the real Bot
  API.
- Wire-snapshot regeneration is a separate visible commit step
  via `SPECTA_UPDATE=1 cargo test -p codeless-types --test
  specta_snapshot`; do not let it land silently in a code commit.
- UI wire regeneration via `cargo run -p codeless-rpc --example
  wire_ts` after every wire change.

### Verify before commit (the `checks` trio item)

The closing-trio gate now routes failed rails through auto-bypass
([`DOCS/sessions/2026-05-18-trio-gate-failure-routing.md`](../../../DOCS/sessions/2026-05-18-trio-gate-failure-routing.md));
a failed `checks` produces a real stage failure with a reason in the
UI, not a silent hang. That makes `checks` strict — every step on
this list must pass before `docs`:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
pnpm -C ui/codeless-ui typecheck
pnpm -C ui/codeless-ui test
pnpm -C ui/codeless-ui lint
```

A failure on any step means stop, fix the cause, re-run — do not
advance to `docs`, do not paper over with `#[allow(...)]`, do not
mark the stage `[x]`.

## REVIEW gate behaviour

REVIEW gates pause the *next* stage; they do **not** pause the
stage that produced them. The stage that *led* to the gate still
runs its full closing trio (`checks`, `docs`, `git`) before the gate
fires. At a REVIEW gate, the handover must include:

1. **What landed** — bullet list, every code path with file + line
   refs. Use `[file.rs:42](file.rs#L42)` style so the operator can
   click through.
2. **What the next stage assumes** — the state the next stage needs
   to find on disk (migrations applied, wire types regenerated,
   a specific RPC reachable from `curl`, etc.).
3. **Open questions surfaced in this phase** — anything the doc
   didn't predict. If you found a fifth open question while
   implementing C2, name it here so the operator can decide before
   C3 starts.
4. **Risks the operator should look at before resuming** — files
   that smelled wrong, tests that passed but felt brittle, places
   you considered a drive-by refactor and chose not to.

REVIEW gates are not rubber-stamps. If the operator asks for
changes, the next non-REVIEW stage's prompt is the operator's
comment plus the doc bullet for that stage; do not re-run the
prior stage to "redo" — fix forward.

## Closing trio — the last three todos of every stage

Every stage's todo checklist ends with the same three items, in
order. The user watches these tick over in the `Stages` overview;
they are how the user confirms a long-running stage actually
landed instead of just looking like it did. Do **not** rename or
reorder them.

1. `checks` — run the stage's verify steps from the section above.
   Every step must pass. On failure: stop, fix, re-run; do not
   advance to `docs`. The trio-gate now surfaces a per-rail
   `failure_detail` so a failure here will appear in the UI with
   the specific step that broke; that's the signal that the rail
   actually failed (not that the gate is stuck).
2. `docs` — update `handover.md` for the next stage and the active
   session doc under `DOCS/sessions/`, in the same worktree, so
   the fresh agent that opens the next stage has the context it
   needs. The session doc follows the shape of
   [`2026-05-18-trio-gate-failure-routing.md`](../../../DOCS/sessions/2026-05-18-trio-gate-failure-routing.md):
   Symptom (if applicable), Root cause / What this stage does,
   Fix (file + line refs), Tests added, What this does NOT change,
   Operational notes.
3. `git` — stage the changes (`git add -A` from the worktree root,
   or specific paths if the stage was surgical), commit with the
   message `stage N: <one-line title from template.yaml>`, and
   push to `codeless/job-chat`. The commit message must match the
   stage's YAML title verbatim through the colon; the history then
   mirrors the template stages one-for-one and the per-stage
   commit graph is the audit trail.

A stage is not "done" until all three todos are green and the push
succeeds. If `checks` or `git` fails, fix the cause and retry — do
not mark the stage `[x]`, do not advance, and never `--force` or
`--no-verify`. If a stage genuinely produced no change (e.g. an
investigation stage that only updated `SCOPE.md` and that doc was
already current), say so in the handover and mark `git` as
`skipped — no diff`, but the next stage's commit must include any
side-effect files the investigation touched.

## Anti-patterns specific to this job

- **Do not invent a `codeless-supervisor` crate.** The supervisor
  is a module inside `codeless-runtime`. Hard rule 2 of JOB-CHAT.md
  + R1 of `codeless/CLAUDE.md`. A grep test at stage 9 enforces
  this; do not add the crate to `Cargo.toml [workspace] members`.
- **Do not give the supervisor a side-channel voice.** No
  `tracing::info!(message = "I just stopped the job")`. No
  `println!`. No direct event publish. Every user-visible word
  from the supervisor is a row in `chat_messages`.
- **Do not write `UPDATE chat_messages SET body = ...` or
  `... SET external_id = ...`.** Rows are immutable past insert
  except for `metadata_json` (delivery receipts only). Edits in
  v0.1 insert a new row (OQ-CHAT-1 bias).
- **Do not preview pre-armed actions.** Hard rule 4 of JOB-CHAT.md:
  pre-armed fires immediately; preview is for ad-hoc only. Adding
  a preview to pre-armed actions is the symmetry violation the
  doc explicitly forbids.
- **Do not couple the supervisor's loop to the coding runner's
  lifetime past Run termination.** The supervisor stays alive
  through a runner crash so it can post the post-mortem; it exits
  only when the Run reaches a terminal status. If you find
  yourself adding "runner exited" handling that cancels the
  supervisor early, stop — re-read the Lifetime section.
- **Do not let the Telegram or Slack adapters keep an in-memory
  message store.** Hard rule 1 of JOB-CHAT.md. The bus subscription
  is the source of truth on the outbound side; `list_job_messages`
  is the source of truth on cold-load.
- **Do not let stage 13's `supervisor_goals` migration touch the
  `chat_messages` table.** Two separate schema concerns; one
  migration each. The `authorised_by` column references
  `chat_messages.id` by string, not a FK in v0.1 — the FK lands
  when JOB-WORKFLOW (B) cleans up the Job/Run split (per
  JOB-CHAT.md's data-model note).
- **Do not regenerate the wire snapshot inside a code commit.**
  Always its own commit step in the same stage, so a reviewer can
  separate "wire format intentionally changed" from "code change
  also touched wire format by accident".
- **Do not skip the rehydration test in stage 15.** It is the only
  test that pins the "survives a server restart" promise; without
  it the deadline-stop feature reads as working in dev but breaks
  silently in any operational restart.

## When to ask vs proceed

- **Ask** (use `AskUserQuestion` or post a question into the job's
  chat thread and wait): a decision that would change the wire
  format, a request to add a transport not in v0.1, a request to
  refactor `codeless-bot-core` beyond what stage 8 extracts, any
  ambiguity about what counts as "destructive" for the ad-hoc
  preview window.
- **Decide and document** (record in `SCOPE.md § Open questions`
  with a one-line *why*): naming choices, internal module layout
  inside `codeless-runtime::supervisor`, test helper shapes,
  vitest fixture file names.
- **Just do it**: anything explicitly listed in the stage's YAML
  description or in the doc's punch lists.

## What "done" looks like at each REVIEW gate

- **M-C1-A (after stage 5)** — `cargo test -p codeless-types -p
  codeless-runtime --lib` green; `codeless-rpc/src/methods.rs`
  has the three new method types; `codeless-types/src/event.rs`
  has the two new variants; specta snapshot regenerated and
  committed.
- **M-C1-B (after stage 8)** — manual smoke: a message typed in
  the web UI's CHAT tab appears in a bound Telegram thread within
  one event tick (or vice versa). `bot_chat_e2e` green. UI vitest
  green. JOB-CHAT.md "Status" rows show C1 shipped.
- **M-C2 (after stage 11)** — manual smoke: with a Run in
  Running, typing "what stage is it on?" in the web CHAT tab gets
  a real supervisor reply within a few seconds. `supervisor_e2e`
  green for spawn / answer / exit. JOB-CHAT.md status records C2.
- **M-C3 (after stage 16)** — manual smoke: type "if this runs
  more than an hour, stop and tell me why" → goal arms; advance
  the clock (or wait an hour on a real run); `stop_job` fires;
  post-action summary lands referencing the authorising message
  id. Restart the server mid-Run, advance the clock again, assert
  the goal still fires. Slack adapter round-trips at parity with
  Telegram.

## References

- Authoritative design:
  [`DOCS/JOB-CHAT.md`](../../../DOCS/JOB-CHAT.md)
- Per-job scope: [`SCOPE.md`](./SCOPE.md)
- Adding-job rules: [`setup/ADDING-JOB.md`](../../../setup/ADDING-JOB.md)
- Agent rules: [`CLAUDE.md`](../../../CLAUDE.md),
  [`codeless/CLAUDE.md`](../../../CLAUDE.md)
- Trio-gate failure routing this job relies on:
  [`DOCS/sessions/2026-05-18-trio-gate-failure-routing.md`](../../../DOCS/sessions/2026-05-18-trio-gate-failure-routing.md)
