# AGENT-DECISIONS.md

Decisions recorded by stage 1 of the `agent-personas` job. Later
stages cite these by number; contradicting a decision without
amending this file is a workflow failure (see
`.codeless/jobs/agent-personas/SCOPE.md` and `WORKFLOW.md`).

Scope of authority: this file owns the *binding mechanism* between
personas and the job/stage model. It does not own the
reviewer-persona default itself — that belongs to
`DOCS/SESSION-PEER-REVIEW-IMPROVEMENTS.md` and decision D5 below
defers to it.

The persona record format is fixed: a single record shape with
optional fields. No `Persona.web` / `Persona.job` / `Persona.review`
variants, now or later. Every decision below is constrained by that
rule.

---

## D1 — Per-stage persona declaration lives on the stage, not the job header

**Decision.** A per-stage persona override is declared inside the
existing stage object in the job's template YAML, as a `persona:`
key alongside the stage's other fields. No new top-level
`personas:` block at the job header.

**Syntax.** The value is a single string id resolved against the
`personas` table:

```yaml
stages:
  - title: "implement"
    persona: "builtin:coder"
  - title: "review"
    persona: "builtin:reviewer"
```

Plain `builtin:<slug>` resolves to a seeded built-in row; a bare id
(`"persona_01J..."`) resolves to a user row. No inline persona
blobs; no anonymous personas. If the id does not resolve, the stage
fails fast at job-submit, before any runner spawns.

The job-level persona is set once at submit time on the job row
(`jobs.persona_id`). A stage with no `persona:` key inherits the
job's persona; a job with no persona falls back to the runner
default. Helper-role rule #1 holds: a job must run end-to-end with
no persona configured anywhere.

**Why this shape.**

- It matches the lean in SCOPE.md ("lean toward the former").
- The stage is already the unit the runtime instantiates a runner
  for, so the override lives where it is consumed. A top-level
  `personas:` block would force a second indirection (`stage.uses:
  coder`) without adding expressiveness.
- One key, one string — easy to parse, easy to diff, hard to
  misuse. No partial override semantics to design.
- Keeps `JOB-MODEL.md`'s stage schema as the single place to look
  when reading a template.

**Consequences for later stages.**

- Stage 9 extends the stage YAML schema with exactly one optional
  string field, `persona`.
- The runtime resolves the id at job-submit and stores it on the
  stage row (`stages.persona_id`, added in stage 8), so a re-run
  reproduces the exact persona even if the row was edited later —
  see D4 for why the resolution time matters.

---

## D2 — A per-stage persona override inherits the persona record verbatim; no per-stage snippet list

**Decision.** A stage's `persona:` declaration selects an existing
persona row and applies its `instructions`, `default_model`,
`allowed_subagents`, and (chat-only) `default_snippets` as-is. The
stage YAML does **not** accept a parallel `snippets:` list or any
other field that overrides or merges into the persona record.

**Why inherit-only.**

- Mirrors the SCOPE.md lean ("Lean inherit-only").
- Personas are pure config (SCOPE.md "Personas are pure config"); a
  stage-local snippet list would re-introduce a second config
  surface to keep in sync with the persona record.
- The persona id is the unit a re-run replays. If a stage could
  layer extra snippets on top, "re-run with the same persona" stops
  meaning the same thing.
- If a real job needs a different snippet set, the right move is a
  new persona row, not a stage-local override. Persona rows are
  cheap once SQLite lands (stage 6).

**Consequences.**

- Stage 9 does not add a `snippets:` key to the stage schema.
- Per-stage handover (added in stage 8) records `persona_id` only,
  not a merged config blob.

---

## D3 — `use_for_jobs` is the single dimension gating MCP visibility; no separate `expose_via_mcp` flag

**Decision.** A persona is exposed as an MCP prompt iff
`use_for_jobs = 1`. There is no separate `expose_via_mcp` column,
flag, or runtime override. The same boolean gates two things:

1. Whether the persona appears in the job-submit dropdown (stage 3).
2. Whether the persona is published as an MCP prompt (stage 10).

**Why one dimension.**

- SCOPE.md "Out of scope" calls this out explicitly: "A separate
  `expose_via_mcp` flag. `use_for_jobs` is the single dimension
  gating MCP visibility."
- WORKFLOW.md anti-pattern: "A parallel `expose_via_mcp` flag …
  Resist the urge."
- Semantically, an MCP prompt *is* a way to start a job using that
  persona. If the persona is not for jobs, exposing it via MCP
  would be a footgun.
- One flag is easier to reason about and easier to test. Two flags
  invite "what about the matrix where one is on and the other is
  off?" — a question with no useful answer.

**Consequences.**

- The SQLite schema added in stage 6 has `use_for_jobs INTEGER NOT
  NULL` and **no** `expose_via_mcp` column.
- Stage 10 reads `use_for_jobs = 1` as its sole filter when
  publishing MCP prompts.
- Chat-only personas (`use_for_jobs = 0`) stay in the agents UI
  list but are absent from both the job-submit dropdown and the
  MCP prompts surface.

---

## D4 — `default_snippets` stays chat-only for MVP; document the revisit trigger

**Decision.** Snippet resolution at job-start is not implemented in
this job. The persona record keeps `default_snippets` as an array
of snippet ids, but the runtime ignores that field when composing a
runner's system prompt. The chat panel continues to be the only
consumer of `default_snippets`.

**Why defer.**

- SCOPE.md "Out of scope": "Snippet resolution at job time. MVP
  keeps `default_snippets` chat-only — see stage 1 decisions file.
  Revisit only if a real job need appears."
- Snippets are addressed by id; resolving them at job-submit means
  the runtime needs read access to the snippets store and a
  composition rule (order, separators, dedupe vs persona
  instructions). None of those have been pinned down, and pinning
  them now without a concrete use case risks locking in a wrong
  shape.
- Persona `instructions` already give the runner system prompt all
  the per-persona text it needs for the MVP slices.

**Revisit trigger.** Reopen this decision when *any one* of the
following is true:

1. A built-in persona's `instructions` field grows past ~4 KB
   because the same boilerplate is being inlined into multiple
   personas — i.e. a snippet is the right factoring.
2. A user reports that editing a snippet does not affect a running
   job, and the right fix is to resolve snippets at job-submit
   rather than copying their text into `instructions`.
3. The peer-review tie-in (`SESSION-PEER-REVIEW-IMPROVEMENTS.md`)
   requires a reviewer persona to share a snippet set with a coder
   persona without duplicating the text.

Until one of those triggers fires, the runtime composes the system
prompt from `persona.instructions` alone. If a snippet is genuinely
needed in a job context today, inline its text into `instructions`
on the persona row — the cost of that copy is the cost of deferring
this decision, and it is cheap.

**Consequences.**

- Stage 3 (job-submit composition) concatenates `instructions`
  only; it does not read `default_snippets`.
- Stage 6's SQLite schema keeps the `default_snippets` JSON array
  column so a future implementation does not need a migration to
  populate it — only to read it.
- Stage 10's MCP prompt body is `persona.instructions` alone;
  consumers wanting snippet expansion get the same answer they get
  in a job.

---

## D5 — Per-stage reviewer binding is configurable; the *default* reviewer persona is owned elsewhere

**Decision.** A stage that runs a peer review resolves its reviewer
persona using the same `persona:` key as any other stage (D1). The
binding mechanism is general: there is no special-case "reviewer"
slot in the stage schema, and no hard-coded `builtin:reviewer`
fallback in this job's runtime code.

What this job ships:

- The mechanism to set `stage.persona` to any persona id, including
  `builtin:reviewer`, on any stage.
- The runtime composition path that applies that persona's
  instructions to the stage's runner system prompt only.

What this job does **not** ship:

- A default value for the reviewer-stage `persona` field when the
  job does not specify one. The default lives in
  `DOCS/SESSION-PEER-REVIEW-IMPROVEMENTS.md` and is consumed by the
  peer-review feature, not by this job. If that doc names
  `builtin:reviewer` as the default, the peer-review feature reads
  that decision; this job does not encode it.

**Why split it this way.**

- SCOPE.md "Resolution required" #5: "Lean configurable with
  `builtin:reviewer` as default. Note: the *default* is owned by
  `SESSION-PEER-REVIEW-IMPROVEMENTS.md`; this doc owns the
  binding mechanism only."
- WORKFLOW.md anti-pattern: "Re-defining the reviewer-default.
  Per-stage persona binding is this job's; the *default reviewer
  persona* is owned by `SESSION-PEER-REVIEW-IMPROVEMENTS.md`. Do
  not contradict it."
- Two docs setting the same default is the worst outcome — they
  drift, and the resolution order between them is undefined.
  Keeping ownership single is the point.

**Consequences.**

- Stage 9 implements `persona:` on any stage, including review
  stages, without branching on stage kind.
- Built-in personas seeded in stage 6 include `builtin:reviewer`
  (per SCOPE.md "Coder, Architect, Code Reviewer, Security,
  Designer"), so the id is available for any doc that wants to
  cite it as a default.
- If `SESSION-PEER-REVIEW-IMPROVEMENTS.md` later says "default
  reviewer persona is `builtin:reviewer`", the peer-review feature
  is the one that fills `stage.persona` with that id at template
  expansion time; this job's runtime treats it as just another id.

---

## Decisions not made here

Out of scope for stage 1, by design:

- The exact SQL schema for `personas` and `stages.persona_id` —
  pinned in stage 6.
- The `RpcClient` method signatures for `list_personas` etc. —
  pinned in stage 7.
- Migration path from existing KV-store `ai-agents` entries to
  SQLite rows. SCOPE.md `WORKFLOW.md` anti-pattern: "Auto-promoting
  KV-store personas to SQLite at first boot." A deliberate
  migration is required and will be pinned no later than stage 6.
- The default reviewer persona itself (see D5).
