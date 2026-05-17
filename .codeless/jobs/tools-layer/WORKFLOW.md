# Workflow — tools-layer

The job is one phase, six stages, two REVIEW gates. Read this at the
top of every stage; the rules below apply per-stage, not once.

## Sequencing

- T1 → T2 → REVIEW → T3 → T4 → T5 → REVIEW.
- T1 is mechanical (new crate, workspace member, builds clean). Do
  not bundle Tool-trait scaffolding into it; T2 owns that.
- T2 freezes the public surface: `Tool`, `ToolCtx`, `ToolError`,
  registration, `testing::fake_ctx()`. The first REVIEW gate exists
  so a human signs off on the trait before T3 and T5 build against
  it. Do not skip ahead.
- T3 (codeless-mcp wiring) and T4 (policy types + NOTICE + SCOPE.md)
  are independent in principle but the agent runs them sequentially
  here so each commit has a single clear theme.
- T5 (browse.fetch port) lands last because it proves T2's surface
  and T3's plumbing together.

## Per-stage discipline

For each stage:

1. Read `SCOPE.md`, this file, and the linked sections of
   `DOCS/TOOLS-PORTING.md` (T1-T5 ticket rows + Tool surface +
   ToolCtx + Porting policy). Do not start writing without those in
   context.
2. Before changing any file, state in the chat which TOOLS-PORTING
   acceptance criterion the stage is targeting and how the stage will
   prove it (test name, grep, cargo command).
3. After changing files: run `cargo check --workspace`,
   `cargo test --workspace -p codeless-tools -p codeless-mcp`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo fmt --check`. Stage is not done if any are red.
4. Run the R1 grep before committing T1, T3, T5: `rg
   'std::process|tokio::process' crates/codeless-types crates/codeless-rpc
   crates/codeless-client` — must return zero matches.
5. Commit + push per the block below.

## Anti-patterns specific to this job

- **Do not invent `ToolCtx` fields the trait does not need yet.** The
  shape is frozen at the first REVIEW; new fields after that require
  re-opening the gate. Stick to what TOOLS-PORTING §ToolCtx lists.
- **Do not port a second tool while T5 is in flight.** Phase 2 picks
  the second tool *after* Phase 1 lands; running ahead defeats the
  point of the "shake out the abstractions" gap between phases.
- **Do not migrate `ai-runner`'s in-tree primitives.** TOOLS-PORTING
  §"ai-runner overlap" defers this — leave them alone.
- **Do not skip the NOTICE file.** Ported moxxy code without
  `codeless/NOTICE` is a license bug, not a TODO.
- **Do not soften clippy.** No `#[allow(clippy::...)]` to make a
  warning go away. Fix the cause.
- **Do not pre-create plugin scaffolding.** `register_tool(...)`
  exists for built-in tools in this job; the manifest reader and
  plugin loader are PLUGIN-SUBSTRATE.md item 6, not this job.

## REVIEW gate behaviour

Two gates. Each REVIEW pauses the next stage; the stage that *led*
to the gate still commits + pushes as normal.

**REVIEW after T2 — Tool surface frozen.** The handover must include:

- The final `Tool` trait signature (paste the trait body).
- The final `ToolCtx` field list with a one-line justification for
  each field, including the decision on `mcp_session` (SCOPE OQ-1).
- The registration mechanism chosen with a one-line justification
  (SCOPE OQ-2).
- Any deviation from TOOLS-PORTING §Tool surface, with reason.

**REVIEW after T5 — Phase 1 acceptance.** The handover must include:

- Output of `cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo fmt --check`.
- The R1 grep result.
- Test name and assertion that proves `browse.fetch` round-trips
  through MCP end-to-end.
- A one-paragraph note for the Phase 2 author: which abstraction hurt
  most in T5 (TOOLS-PORTING §"Phase 2" uses this to pick the second
  tool).

## Commit + push after every stage

At the end of every stage — including stages that precede a REVIEW
gate, including stages that only edit docs — the agent MUST:

1. Stage every change the stage produced (`git add -A` from the
   worktree root, or specific paths if the stage was surgical).
2. Commit with the message `stage N: <one-line title from
   template.yaml>` so the history mirrors the template stages
   one-for-one.
3. Push to the job's branch (`codeless/tools-layer`) so the work is
   recoverable even if the worktree is wiped.

A stage is not "done" until the push succeeds. If the commit or
push fails, fix the cause and retry — do not mark the stage `[x]`,
do not advance, and never `--force` or `--no-verify`. If a stage
genuinely produced no change (e.g. an investigation stage that
only updated `SCOPE.md` and that doc was already current), say so
in the handover and skip the commit, but the next stage's commit
must include any side-effect files the investigation touched.
