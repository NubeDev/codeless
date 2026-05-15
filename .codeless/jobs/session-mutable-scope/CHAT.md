# Chat for this job

## user @ 2026-05-15T08:19:48Z

reviewe scope for me, and confirm it as a quick summery

## assistant @ 2026-05-15T08:20:12Z

## Scope review — session-mutable-scope

**What this job ships:** Turn SCOPE.md/CLAUDE.md from static prose into a runtime-mutated rulebook via REVIEW-stage patches, with Layer-1 parse-time guards so the rulebook compounds rather than rots. Ramp covers Steps 0–6; stops at human-approval UX.

**Shape looks right:**
- Goal, in-scope, and out-of-scope are tight and mutually consistent — Steps 0–6 in scope, TEST-emitted patches / auto-merge / multi-tenant explicitly ruled out.
- Constraints correctly pin R1 (predicate runner host-only, `process::Command` stays put), R4 (patches are file artifacts in `DOCS/SCOPE-PROPOSED.md`, not a new SQLite table), and the wire-format sacredness rule (`JOB-MODEL.md`, `JOB-LOOP.md`, `handover.rs` never patched by REVIEW).
- `ScopePatch` correctly placed in `codeless-types` (mobile-safe).
- WORKFLOW correctly sequences: Step 1 before 2 (diff-verify is a REVIEW pre-check), Step 3 parallelisable, Step 5 before 6 (parser before UX), and bundles H1/H3/H7 with Steps 0–2 per the design doc.
- Two REVIEW gates well-placed: pre-Step-5 (last chance to fix mutable-set list) and post-Step-6 (ramp complete).
- "Open questions" are forced to be resolved in stage 1 into a decisions file rather than silently guessed — good.

**Things worth flagging before you run:**

1. **Stage 1 carries a lot.** "Step 0 docs-only" also resolves all six open questions *and* tightens JOB-MODEL.md with worked examples/anti-examples per section. That's plausibly two stages (decisions-first, then doc tightening) — but bundling is defensible since both are docs-only. Your call.

2. **H1/H3/H7 stage label is opaque.** The stage line reads `H1/H3/H7 handover correctness` without naming what H1/H3/H7 *are*. Future-you (or another agent) reading just `template.yaml` won't know. Either inline a one-liner ("per-stage handover keys, keyed discovery, write-time validation") or cite the doc section. The SCOPE.md prose already expands it; the `stages[]` entry should too since that's what the runner folds into prompts.

3. **Step 4 shadow-mode telemetry.** Scope says "instrument the four-week kill-criterion metric (proposal noise rate)" but doesn't say *where* it lands (SQLite events table? log line? a metrics endpoint?). Given R4, probably the events bus — worth pinning before stage 4 runs so the agent doesn't invent a metrics store.

4. **Step 6 says "CLI command (and/or UI affordance)".** The "and/or" is the kind of ambiguity that becomes a stage-time argument. Recommend picking one for this job (CLI is lower-risk, satisfies R2/R3 trivially) and deferring the UI to a follow-up.

5. **Pre-Step-5 REVIEW gate criteria are stated but not checklisted.** "Confirm 0-3 are integrated, kill-criterion telemetry is wired, mutable-set / wire-format-set lists are correct" — consider making each a literal checkbox in a per-stage `*.md` so the gate is mechanical, not interpretive.

**Verdict:** Scope is coherent and enforceable. Worth resolving (2) and (4) before you flip back to work mode; (1), (3), (5) are nice-to-haves you can leave to the stages.
