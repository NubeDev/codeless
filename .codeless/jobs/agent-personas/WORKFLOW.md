# Workflow — agent-personas

## Sequencing

The ramp is intentionally ordered. Do not reorder:

- Stage 1 (decisions) must land before any code stage cites a
  decision.
- MVP slices 1-3 land in order: persona record fields → job-submit
  dropdown → persist `persona_id`. Each is independently shippable;
  each makes the next cheap.
- REVIEW gate before SQLite. Last chance to catch a wrong field
  shape before a migration locks it in.
- SQLite table → RpcClient surface → FK promotion → per-stage
  override. Each strictly depends on the one before.
- MCP exposure is the cap, then final REVIEW.

## Per-stage discipline

Each stage:

1. Re-reads `SCOPE.md`, this `WORKFLOW.md`, the relevant section
   of `DOCS/AGENT.md`, and `DOCS/AGENT-DECISIONS.md` (after stage 1
   creates it).
2. Lands code + tests in the same commit.
3. Runs `cargo test --workspace`, `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo fmt --check` for Rust
   changes; runs `pnpm -C codeless/ui/codeless-ui typecheck` and
   `pnpm -C codeless/ui/codeless-ui lint` for UI changes. All
   green before commit. `-D warnings` is non-negotiable.
4. Updates `SCOPE.md` or this file ONLY if the stage discovers a
   workflow gap. Code stages do not touch SCOPE/WORKFLOW casually.
5. Writes the handover with `done` = paths actually touched and
   `next` = a one-sentence pointer to the next stage's first
   action.

## Commit + push after every stage

At the end of every stage — including stages that precede a REVIEW
gate, including stages that only edit docs — the agent MUST:

1. Stage every change the stage produced (`git add -A` from the
   worktree root, or specific paths if the stage was surgical).
2. Commit with the message `stage N: <one-line title from
   template.yaml>` so the history mirrors the template stages
   one-for-one.
3. Push to the job's branch (`codeless/agent-personas`) so the
   work is recoverable even if the worktree is wiped.

A stage is not "done" until the push succeeds. If the commit or
push fails, fix the cause and retry — do not mark the stage `[x]`,
do not advance, and never `--force` or `--no-verify`. If a stage
genuinely produced no change, say so in the handover and skip the
commit, but the next stage's commit must include any side-effect
files the investigation touched.

## REVIEW gate behaviour

Two REVIEW gates: pre-SQLite (after MVP slice 3) and final (after
MCP exposure). Each gate:

- Commits and pushes the stage that *led* to it before pausing.
- Writes a handover summarising what landed and what comes next.
- Does NOT advance until the user resumes.

The pre-SQLite gate is the load-bearing one: after it, fields are
locked into a migration. Use the gate to confirm field names, JSON
shapes, and the read-only enforcement path are right.

## Anti-patterns specific to this job

- **A fourth runner.** Personas shape the prompt; they do not
  drive coding. If you find yourself adding a runner trait impl
  for "PersonaRunner", you have crossed the line.
- **`Persona.web` / `Persona.job` / `Persona.review`.** One record
  format, forever. Add optional fields; do not branch the type.
- **Widening subagent tools.** Personas can narrow
  `allowed_subagents`; they cannot widen the `READ_ONLY_TOOLS`
  set the registry hands back. Two-layer enforcement is
  load-bearing.
- **Sending persona blobs to the LLM client-side.** The UI calls
  `RpcClient`; the runtime composes the system prompt server-side.
  If you find a `fetch` posting `{ system: persona.instructions }`
  from the UI, you have broken R2.
- **A parallel `expose_via_mcp` flag.** `use_for_jobs` is the
  single gating dimension for MCP visibility. Resist the urge.
- **Re-defining the reviewer-default.** Per-stage persona binding
  is this job's; the *default reviewer persona* is owned by
  `SESSION-PEER-REVIEW-IMPROVEMENTS.md`. Do not contradict it.
- **Auto-promoting KV-store personas to SQLite at first boot.**
  Built-in personas seed with `built_in = 1`. User-edited KV
  personas need an explicit migration path (decided in stage 1)
  or are left as KV — they are not silently promoted.
- **Drive-by refactor of the agents UI.** AgentsSection.tsx gets
  exactly two additions (toggle + multi-select). Resist tidying.

## Run-of-show summary (for handover assembly)

| Stage | Layer | Touches |
|-------|-------|---------|
| 1 decisions | docs | DOCS/AGENT-DECISIONS.md |
| 2 MVP slice 1 | UI | agents.ts, AgentsSection.tsx, runSubagent.ts |
| 3 MVP slice 2 | UI + runtime | CreateJob form, runner system-prompt composition |
| 4 MVP slice 3 | runtime | jobs schema (persona_id column, no FK yet), submit/get_job |
| 5 REVIEW (pre-SQLite) | — | gate |
| 6 SQLite personas table | runtime | new migration, built-in seed, codeless-types |
| 7 RpcClient surface | runtime + UI | list/get/upsert/delete_personas, KV-as-cache |
| 8 FK promotion + stages | runtime | jobs.persona_id FK, stages.persona_id, handover record |
| 9 per-stage override | runtime | template stage YAML parsing, system-prompt application |
| 10 MCP prompts | runtime | use_for_jobs-gated MCP prompt exposure |
| 11 REVIEW final | — | R1-R5 + five honesty rules verified |
