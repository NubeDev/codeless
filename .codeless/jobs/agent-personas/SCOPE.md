# Scope — agent-personas

## Goal

Unify the three meanings of "agent" in this codebase — **persona**
(config: system prompt + instructions + allowed subagents), **subagent**
(read-only spawnable worker), **runner** (the thing that drives a job
stage) — by making a single `Persona` record the contract that the
chat panel (already wired), the job-submit form (new), and per-stage
overrides (new) all consume. After this job lands, "Coder", "Architect",
"Code Reviewer", "Security", and "Designer" are first-class job
templates without inventing a new template system; subagent
whitelisting is encoded on the persona record; and SQLite is the
source of truth for personas so jobs and stages can FK them.

The deep design is in
[`DOCS/AGENT.md`](../../../DOCS/AGENT.md). This file is the per-job
brief; the doc is authoritative.

## In scope

- MVP slice 1: extend the UI-side `Persona` record with
  `use_for_jobs` and `allowed_subagents`; surface both in
  `AgentsSection.tsx`; enforce the whitelist in `runSubagent.ts`.
- MVP slice 2: persona dropdown in the create-job form; concatenate
  selected persona's `instructions` into the runner system prompt
  at job-start; seed model from `default_model`.
- MVP slice 3: persist `persona_id` on the job row (KV-only persona
  storage at this point, FK comes later).
- REVIEW gate to confirm MVP holds before SQLite migration.
- SQLite `personas` table per the schema sketch; built-in personas
  seeded on first boot with `built_in = 1`; user edits write new
  rows with `built_in = 0`.
- `RpcClient` surface: `list_personas` / `get_persona` /
  `upsert_persona` / `delete_persona`. The UI's `ai-agents` KV
  store becomes a cache mirroring SQLite via these methods.
- Promote `jobs.persona_id` to a FK; add `stages.persona_id` with
  the same FK; record the persona on each stage's handover.
- Per-stage persona override: stage YAML accepts a `persona:`
  declaration; runtime resolves and applies it to that stage's
  runner system prompt only.
- Expose personas where `use_for_jobs = 1` as MCP prompts.
- Final REVIEW gate to verify R1-R5 and the five "Rules that keep
  this honest."

## Out of scope

- Adding a fourth runner. Personas shape the prompt; they do not
  become a runner. Coding still goes through ClaudeRunner /
  CodexRunner / AnthropicRunner / OpenAIRunner.
- Widening subagent tool sets. Personas can narrow which subagents
  are spawnable; they cannot widen the read-only tool set the
  registry hands back.
- Snippet resolution at job time. MVP keeps `default_snippets`
  chat-only — see stage 1 decisions file. Revisit only if a real
  job need appears.
- Reviewer-as-separate-session design. That belongs to
  `SESSION-PEER-REVIEW-IMPROVEMENTS.md`; this job *binds* a
  reviewer persona to a stage but does not redefine the reviewer
  default.
- New persona variants (`Persona.web`, `Persona.job`,
  `Persona.review`). Mirrors R3 — one persona record format,
  forever. Add optional fields to the single record if needed.
- A separate `expose_via_mcp` flag. `use_for_jobs` is the single
  dimension gating MCP visibility.
- TODO comments. Per CLAUDE.md R4, no half-finished implementations.
  Mark unfinished stages `[!]` and halt.

## Constraints

- **R1 (crate direction).** Persona resolution is a `codeless-types`
  + `codeless-runtime` concern; no process spawn added to mobile-safe
  crates. `RpcClient` methods are typed in mobile-safe crates.
- **R2 (single transport).** UI imports `RpcClient` only. No
  `@tauri-apps/api/*`, no direct `fetch` to the server. The
  persona dropdown reads from `RpcClient.list_personas()` once
  SQLite lands; the KV cache mirrors that.
- **R3 (one UI framework).** No `AgentsSection.web.tsx` or
  similar. One `AgentsSection.tsx`; one `CreateJobForm`.
- **R4 (SQLite is source of truth).** Once the personas table
  lands, the `ai-agents` KV store is a *cache*, not a parallel
  source of truth. Read-through from RPC.
- **R5 (single-tenant trust).** Unchanged. Persona records are
  not per-user-scoped; the bearer token authorises all clients
  identically.
- **Personas are pure config.** No transport, no model calls, no
  side effects in the record. The UI never sends a persona blob
  to the LLM directly — the runtime composes the system prompt
  server-side.
- **Subagents stay read-only.** `runSubagent.ts` checks
  `persona.allowed_subagents` *before* resolving the subagent id;
  the registry then hands back only its `READ_ONLY_TOOLS` set.
  Two-layer enforcement.
- **Personas do not drive coding.** Per SCOPE.md helper-role rule
  #3: no Rig agent that writes code. Personas are advisory context
  for a runner, not a replacement for one.
- **A job must run end-to-end with no persona configured.**
  Personas enhance, never gate. Mirrors helper-role rule #1.
- **One persona record format, forever.** R3-shaped rule: add
  optional fields, never variants.
- **Comments per CLAUDE.md.** No emojis, no task-status comments,
  no restatements, no banners.

## Resolution required from "Open questions"

Stage 1 MUST resolve these into `DOCS/AGENT-DECISIONS.md` (new
file). Later stages cite the decisions; contradicting a recorded
decision without amending the file is a workflow failure:

1. Per-stage persona declaration syntax — in `JOB-MODEL.md`'s
   stage schema, or a new `personas:` block at the top of the job
   file? Lean toward the former.
2. Persona vs snippet overlap — does a stage's persona-override
   support a `snippets:` list, or inherit only from the persona?
   Lean inherit-only.
3. MCP exposure — `use_for_jobs` is the single gating dimension;
   confirm no separate `expose_via_mcp` flag is needed.
4. Snippet resolution at job time — keep chat-only for MVP;
   document the trigger for revisiting.
5. Reviewer-persona default — always `builtin:reviewer`, or
   configurable per-job? Lean configurable with `builtin:reviewer`
   as default. Note: the *default* is owned by
   `SESSION-PEER-REVIEW-IMPROVEMENTS.md`; this doc owns the
   binding mechanism only.

## Pointers

- Design: [`DOCS/AGENT.md`](../../../DOCS/AGENT.md)
- Current persona record + KV: `codeless/ui/codeless-ui/src/modules/ai/lib/agents.ts`
- Subagent registry: `codeless/ui/codeless-ui/src/modules/ai/agents/registry.ts`
- Subagent runner: `codeless/ui/codeless-ui/src/modules/ai/agents/runSubagent.ts`
- Settings UI: `codeless/ui/codeless-ui/src/settings/sections/AgentsSection.tsx`
- Job model (where per-stage syntax may land): `DOCS/JOB-MODEL.md`
- Peer-review tie-in: `DOCS/SESSION-PEER-REVIEW-IMPROVEMENTS.md`
- Workspace rules: `../CLAUDE.md` (workspace), `./CLAUDE.md` (inner repo)
