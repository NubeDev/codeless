# Workflow — telegram-integration

## Read order at the top of every stage

1. [`SCOPE.md`](./SCOPE.md) — this job's scope.
2. [`DOCS/SCOPE-TELEGRAM-INTEGRATION.md`](../../../DOCS/SCOPE-TELEGRAM-INTEGRATION.md)
   — the design rationale and surface ramp. Source of truth for
   the *why*.
3. [`DOCS/SCOPE-SLACK-INTEGRATION.md`](../../../DOCS/SCOPE-SLACK-INTEGRATION.md)
   — the sibling integration. Read it to understand the parts
   shared via the `BotTransport` trait.
4. The [`slack-integration`](../slack-integration/) job dir —
   confirm what shipped, what's still in flight, what shape the
   adapter ended up as. The merge story between these two jobs
   depends on this every stage.
5. [`CLAUDE.md`](../../../CLAUDE.md) — agent rules. R1 and R5
   especially apply here.

## Sequencing — the hard gate

**Stage 1 is a gate, not a step.** If
`ResumeJobArgs.bypass`, `ResumeJobArgs.next_stage_comment`,
`JobResumed.actor`, and `JobResumed.comment` are not all present
on `main` (and visible from this worktree's base) when stage 1
runs, the agent writes a handover that names which fields are
missing, marks stage 1 `[!]` (blocked), and halts. Do not
duplicate any of those fields. Do not branch-merge from the
slack-integration branch into this worktree to "borrow" them —
they must come from `main`.

The check at stage 1 is a real grep against the working tree:

- `grep -n 'pub bypass' crates/codeless-rpc/src/methods.rs` →
  must find the field on `ResumeJobArgs`.
- `grep -n 'pub next_stage_comment' crates/codeless-rpc/src/methods.rs`
  → must find the field on `ResumeJobArgs`.
- `grep -nE 'actor:.*Option<String>' crates/codeless-types/src/event.rs`
  → must find it on `JobResumed`.
- `grep -nE 'comment:.*Option<String>' crates/codeless-types/src/event.rs`
  → must find it on `JobResumed`.

All four must pass. If any fails, stop. Do not move to stage 2.

## Per-stage discipline

- **One stage = one outcome.** The template's stage text is the
  outcome; the agent decides the steps.
- **Read before writing.** Before editing any file
  `codeless-slack` already created, read it. The shared layout
  (Approach path 1 vs 2 from SCOPE) is the largest single
  decision in this job; do not start refactoring blind.
- **Tests live with the code (R5 from CLAUDE.md).** A new parser
  branch lands with its unit test. A new transport-trait method
  lands with at least one in-process mock test.
- **`cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo fmt --check` all green
  before a commit.** Non-negotiable. `clippy -D warnings` is
  load-bearing for the workspace.
- **Conditional compilation.** All Telegram-specific code lives
  behind the `telegram-bot` cargo feature (or whatever the
  shared `codeless-bot` crate's feature is named). A default-
  features build of `codeless serve` must compile and run
  without `teloxide` in the dep graph.

## REVIEW gate behaviour

Each `REVIEW` line in `template.yaml` pauses the runner before
the *next* stage. The stage *leading into* a REVIEW still commits
and pushes its own work (per the commit + push rule below).

At a REVIEW gate, the handover MUST include:

- What shipped in the stage that just finished (one paragraph,
  reference real files / commits).
- What the next stage will do.
- For the REVIEW after stage 2 specifically: which Approach path
  was chosen (1 or 2 from SCOPE), why, and what new crate /
  module layout the operator should expect to see in subsequent
  stages.
- For the REVIEW after stage 6 specifically: a recorded
  end-to-end run from a real Telegram client, with the
  `/status`, `/start`, `/stop`, and `/resume` flows tried
  against a real running job. Screenshots or a transcript in
  the handover.
- For the final REVIEW: confirmation that the keep-it-running
  loop is end-to-end from a phone, including the outbound
  failure notification with the structured REVIEW-context block.

## Anti-patterns specific to this job

- **Re-implementing the parser inside `transport/telegram.rs`.**
  The whole point of the shared adapter is that the parser is
  written once. If the Telegram-side grammar diverges from
  Slack's, fix the parser, not by duplicating it.
- **Letting `chat_id` or `allowed_user_ids` be discovered at
  runtime from incoming messages.** Both come from config. The
  bot does not "learn" who its operator is from whoever DMs it
  first — that is an authorisation bypass disguised as
  ergonomics.
- **Reading the bot token from the env var inside `codeless
  serve`.** `init-session.sh` writes it into the secrets store
  once; the server reads only the secrets store. Violating this
  defeats Risk 4's mitigation.
- **`unwrap()` on network responses from `api.telegram.org`.**
  Long-polling drops, retries, and 429s are routine. Handle
  them; do not panic.
- **Editing the design doc to match the code's shortcuts.** If
  the code can't match the design, fix the code or escalate at
  a REVIEW gate — never silently update the doc to paper over
  the gap.

## Commit + push after every stage

At the end of every stage — including stages that precede a REVIEW
gate, including stages that only edit docs — the agent MUST:

1. Stage every change the stage produced (`git add -A` from the
   worktree root, or specific paths if the stage was surgical).
2. Commit with the message `stage N: <one-line title from
   template.yaml>` so the history mirrors the template stages
   one-for-one.
3. Push to the job's branch (`codeless/telegram-integration`) so
   the work is recoverable even if the worktree is wiped.

A stage is not "done" until the push succeeds. If the commit or
push fails, fix the cause and retry — do not mark the stage `[x]`,
do not advance, and never `--force` or `--no-verify`. If a stage
genuinely produced no change (e.g. an investigation stage that
only updated `SCOPE.md` and that doc was already current), say so
in the handover and skip the commit, but the next stage's commit
must include any side-effect files the investigation touched.
