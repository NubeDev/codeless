# AGENT.md — personas, subagents, and runners

The word "agent" gets used three different ways in this codebase. This
doc names them, separates their responsibilities, and proposes how the
**persona** layer (currently chat-only) extends into **jobs** without
inventing a new template system.

Not a spec. A proposal to argue with before anything lands in
[`SCOPE.md`](./SCOPE.md) or [`JOB-MODEL.md`](./JOB-MODEL.md).

Related:
- [`SCOPE.md`](./SCOPE.md) — overall architecture, helper-role rules
- [`JOB-UI.md`](./JOB-UI.md) — stage chat / job chat split
- [`JOB-MODEL.md`](./JOB-MODEL.md) — handover contract
- [`SESSION-PEER-REVIEW-IMPROVEMENTS.md`](./SESSION-PEER-REVIEW-IMPROVEMENTS.md) — reviewer-as-separate-session, ties into per-stage persona override
- [`agents.ts`](../codeless/ui/codeless-ui/src/modules/ai/lib/agents.ts) — current persona record + KV persistence
- [`registry.ts`](../codeless/ui/codeless-ui/src/modules/ai/agents/registry.ts) — current subagent registry
- [`AgentsSection.tsx`](../codeless/ui/codeless-ui/src/settings/sections/AgentsSection.tsx) — the Settings → Agents UI

## The three layers

| Layer | What it is | Where it lives today |
|---|---|---|
| **Persona** | A *system-prompt preset + custom instructions + snippets + default model + allowed subagents*. Pure config. Coder / Architect / Code Reviewer / Security / Designer are the built-ins. | [`agents.ts`](../codeless/ui/codeless-ui/src/modules/ai/lib/agents.ts), persisted in the `ai-agents` KV store; surfaced by [`AgentSwitcher.tsx`](../codeless/ui/codeless-ui/src/modules/ai/components/AgentSwitcher.tsx) and [`AgentsSection.tsx`](../codeless/ui/codeless-ui/src/settings/sections/AgentsSection.tsx). |
| **Subagent** | A tool-restricted, *read-only* spawnable worker (explore, code-review, security, general). The current session calls one as a tool to fan out research without polluting its context. | [`registry.ts`](../codeless/ui/codeless-ui/src/modules/ai/agents/registry.ts), [`runSubagent.ts`](../codeless/ui/codeless-ui/src/modules/ai/agents/runSubagent.ts), [`tools/subagent.ts`](../codeless/ui/codeless-ui/src/modules/ai/tools/subagent.ts). |
| **Runner** | The thing that actually drives a job stage: `ClaudeRunner`, `CodexRunner`, `AnthropicRunner`, `OpenAIRunner`. Vendored in `ai-runner/`. No persona today — it just executes. | [`SCOPE.md` "Runner layer"](./SCOPE.md#runner-layer--adopt-the-rubix-agentai-runner-crate). |

These are not peers. The hierarchy is:

```
Runner       (host process: spawns CLI or calls API)
 └─ runs under a Persona      (system prompt + instructions + allowed subagents)
     └─ may spawn Subagents   (read-only tools, narrow scope)
```

Personas are *config*. Subagents are *capabilities a persona is allowed
to use*. Runners are *transport*. Keeping the boundary clean is what
lets the same persona record drive both the chat panel and a job stage
without duplicating prompts.

## What the Settings → Agents page does today

The Settings → Agents page is **layer 1 only** — chat-panel personas.
The active persona is read by
[`AiInputBar.tsx`](../codeless/ui/codeless-ui/src/modules/ai/components/AiInputBar.tsx);
its `instructions` field becomes the system prompt for the next
chat-side LLM call. Custom instructions on the page are a global
prefix applied to every persona. Snippets are reusable text fragments
the user can drop in via `#handle`.

None of this currently reaches the job runtime.

## Proposal — one persona record, three call sites

Treat the persona as a small, composable contract:

```ts
type Persona = {
  id: string;
  name: string;
  description: string;
  instructions: string;          // system-prompt prefix
  default_model?: string;        // seeds runner / chat model
  allowed_subagents: string[];   // whitelist; empty = no spawning
  default_snippets?: string[];   // snippet ids to auto-include (chat-only for MVP)
  use_for_jobs: boolean;         // appears in job-submit dropdown; also governs MCP exposure
  built_in: boolean;             // seeded on first boot; UI renders read-only badge
};
```

The same record is consumed by three call sites:

### 1. AI chat panel (already built)

The switcher in the input bar swaps the active persona. Already wired.
The only addition needed is `allowed_subagents`: today the chat agent
can call any subagent in the registry. Once personas declare a
whitelist, e.g. the Designer persona can't spawn a security audit it
has no business asking for.

### 2. Job submit (new)

The "create job" form gets a persona dropdown. The selected persona's
`instructions` are concatenated into the runner's system prompt at
job-start; `default_model` seeds the model field. A "Coder" job and an
"Architect" job become first-class job templates without inventing a
new template system. This is exactly what
[`SCOPE.md` "Prompts"](./SCOPE.md#prompts) hints at when it says job
templates are surfaced as MCP prompts — personas *are* the templates,
once they're typed properly.

The persona id is stored on the job row (`jobs.persona_id`). A re-run
reproduces it; an MCP client can ask "what persona ran this job?"

### 3. Per-stage override (new, ties into peer-review)

This doc owns the **binding mechanism** (how a stage names a persona,
how the runtime resolves it). Which persona the *review* stage uses by
default is owned by
[`SESSION-PEER-REVIEW-IMPROVEMENTS.md`](./SESSION-PEER-REVIEW-IMPROVEMENTS.md) —
the two docs should not both redefine the default.

Each stage may declare its own persona. The natural mapping:

- Plan stages → Architect
- Implement stages → Coder
- Review gates → Code Reviewer / Security

The stage's `handover.md` records which persona ran (see
[`SESSION-PEER-REVIEW-IMPROVEMENTS.md` H1](./SESSION-PEER-REVIEW-IMPROVEMENTS.md#h1-per-stage-handover-not-just-per-job)
for per-stage handovers; persona-per-stage is the natural companion).
This is also how the peer-review proposal's
[P1 "Reviewer is a separate session"](./SESSION-PEER-REVIEW-IMPROVEMENTS.md#p1-reviewer-is-a-separate-session-not-the-same-model-thread)
gets a concrete reviewer identity: the review stage runs under the
Code Reviewer persona, with read-only subagents only.

## Rules that keep this honest

1. **Personas are pure config.** No transport, no model calls, no
   side effects in the persona record. The runtime reads them via
   `RpcClient`; the UI never sends a persona blob to the LLM directly.
   This keeps personas serialisable, syncable across browser / desktop
   / mobile, and storable in SQLite alongside the job row that used
   them — which matters because of [R4 in `CLAUDE.md`](../CLAUDE.md):
   "SQLite is the source of truth," including for which persona a
   stage ran under.

2. **Subagents stay read-only.** The current registry whitelist
   (`READ_ONLY_TOOLS = ["read_file", "list_directory", "grep", "glob"]`
   in [`registry.ts`](../codeless/ui/codeless-ui/src/modules/ai/agents/registry.ts))
   is load-bearing. Enforcement is two-layered:
   [`runSubagent.ts`](../codeless/ui/codeless-ui/src/modules/ai/agents/runSubagent.ts)
   checks `persona.allowed_subagents` before resolving the subagent id;
   the registry then hands back only its `READ_ONLY_TOOLS` set. A
   persona cannot widen the tool set — only narrow which subagents are
   spawnable.

3. **Personas do not drive coding.** A persona is *advisory context
   for a runner*, not a replacement for one. Coding still goes through
   `ClaudeRunner` / `CodexRunner` / `AnthropicRunner` / `OpenAIRunner`,
   per [`SCOPE.md` helper-role rule #3](./SCOPE.md#helper-role--rig-optional-never-gates-a-job)
   ("no Rig agent that writes code"). Personas shape the prompt; they
   do not become a fourth runner.

4. **A job must run end-to-end with no persona configured.** Personas
   enhance; they never gate. Mirrors the helper-role rule #1: if the
   user doesn't pick one, the runner uses its default system prompt
   and the job still runs.

5. **One persona record format, forever.** No `Persona.web`,
   `Persona.job`, `Persona.review` variants. If a call site needs
   extra fields, add them to the single record as optional. Mirrors
   [R3 in `CLAUDE.md`](../CLAUDE.md) ("one UI framework, forever").

## Schema sketch

```sql
-- new table; personas live in SQLite, not just KV, so jobs can FK them
CREATE TABLE personas (
  id              TEXT PRIMARY KEY,
  name            TEXT NOT NULL,
  description     TEXT,
  instructions    TEXT NOT NULL,
  default_model   TEXT,
  allowed_subagents TEXT NOT NULL DEFAULT '[]',  -- json array
  default_snippets  TEXT NOT NULL DEFAULT '[]',  -- json array; chat-only for MVP
  use_for_jobs    INTEGER NOT NULL DEFAULT 0,
  built_in        INTEGER NOT NULL DEFAULT 0,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL
);

-- job and stage gain optional persona refs
ALTER TABLE jobs   ADD COLUMN persona_id TEXT REFERENCES personas(id);
ALTER TABLE stages ADD COLUMN persona_id TEXT REFERENCES personas(id);
```

Built-in personas seed on first boot; user edits write a new row with
`built_in = 0` so updates to the seed list never silently overwrite
user edits.

The UI's existing `ai-agents` KV store becomes a *cache* that mirrors
the SQLite source of truth via `RpcClient.list_personas()`.

## What changes in the Settings → Agents UI

Two small additions make the page do real work for jobs, not just
chat:

- **"Use as job default"** toggle on each persona — sets
  `use_for_jobs = 1`, makes it appear in the job-submit dropdown.
- **"Allowed subagents"** multi-select — wires layer 1 to layer 2
  explicitly.

Everything else on the page (custom-instructions block, snippets) is
already correct.

## Open questions

1. **Per-stage persona declaration syntax.** YAML stages need a way to
   say `persona: builtin:reviewer`. Does this go in
   [`JOB-MODEL.md`](./JOB-MODEL.md)'s stage schema, or in a new
   `personas:` block at the top of the job file? Probably the former —
   one less thing to look up.
2. **Persona vs. snippet overlap.** Snippets are short, droppable; a
   persona's `default_snippets` is a way to pre-load them. Should a
   stage's persona-override also support a `snippets:` list, or only
   inherit from the persona? Lean toward inherit-only to keep the
   surface tight.
3. **MCP exposure.** Personas as MCP prompts (per
   [`SCOPE.md` "Prompts"](./SCOPE.md#prompts)) is the obvious move.
   `use_for_jobs` is the **single** dimension that gates MCP visibility —
   do not add a separate `expose_via_mcp` flag. Chat-only personas
   (e.g. a personal "explain to me like I'm five" persona) are noise
   to an MCP client and stay hidden by the same boolean that keeps
   them out of the job-submit dropdown.
4. **Snippet resolution at job time.** Chat uses `#handle` as an
   inline expansion the user types. For jobs there is no typist —
   `default_snippets` would have to be inlined into the system prompt
   at job-start. MVP: keep snippets chat-only; revisit if a real job
   need appears.
5. **Reviewer-persona / peer-review tie-in.** The review stage in
   [`SESSION-PEER-REVIEW-IMPROVEMENTS.md` P1](./SESSION-PEER-REVIEW-IMPROVEMENTS.md#p1-reviewer-is-a-separate-session-not-the-same-model-thread)
   wants a separate session under a reviewer model. Is the reviewer
   persona always `builtin:reviewer`, or configurable per-job? Probably
   configurable, with `builtin:reviewer` as the default.

## Minimum-viable first slice

If we want to ship this incrementally without blocking on the full
schema:

1. Add `use_for_jobs: boolean` and `allowed_subagents: string[]` to
   the existing UI-side `Persona` record. KV-only for now.
2. Wire the job-submit form's persona dropdown to read the same KV
   store and concatenate `instructions` into the runner system
   prompt.
3. Persist the persona id on the job row only (skip per-stage for
   now). One column, one migration.
4. Defer per-stage override and the SQLite-as-source-of-truth move
   until peer-review lands and forces the issue.

Three small steps, each independently shippable, each visible to the
user.
