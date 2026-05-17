# WORKFLOW — slack-integration

## Sequencing

Stages 1-4 are sequential build-up: RPC args first, then crate scaffold,
then parser, then wiring. Stage 5 is a REVIEW gate. Stages 6-7 add
the event-driven outbound side. Stage 8 is the final REVIEW.

## Per-stage discipline

- Read SCOPE.md (this job's scope doc) at the start of every stage.
- Read CLAUDE.md R1 (crate dependency direction) before adding any
  dependency — codeless-slack is host-only and must never be imported
  by mobile-safe crates.
- Read CLAUDE.md R2 (no emojis, no task-status comments) before
  writing any comment.
- Verify `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
  passes before committing.

## Commit + push after every stage

At the end of every stage — including stages that precede a REVIEW
gate, including stages that only edit docs — the agent MUST:

1. Stage every change the stage produced (`git add -A` from the
   worktree root, or specific paths if the stage was surgical).
2. Commit with the message `stage N: <one-line title from
   template.yaml>` so the history mirrors the template stages
   one-for-one.
3. Push to the job's branch (`codeless/slack-integration`) so the
   work is recoverable even if the worktree is wiped.

A stage is not "done" until the push succeeds. If the commit or
push fails, fix the cause and retry — do not mark the stage `[x]`,
do not advance, and never `--force` or `--no-verify`.

## Anti-patterns

- Do not add Block Kit interactive flows. Plain text commands only.
- Do not edit prior Slack messages on state change — post new ones.
- Do not add per-channel state or "last job ID" memory.
- Do not add Slack reactions as decisions.
- Do not add `slack.read_thread` or any agent-tool variant — that is
  a separate integration (see SCOPE.md "Out of scope" section).
