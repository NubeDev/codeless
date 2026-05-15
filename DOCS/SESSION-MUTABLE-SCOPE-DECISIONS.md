# SESSION-MUTABLE-SCOPE — recorded decisions

`SCOPE.md` for the `session-mutable-scope` job lists six open
questions and requires stage 1 to resolve them explicitly rather than
let later stages silently guess. This file records the resolution.
A later stage that contradicts an entry here without amending it
first is a workflow failure (per the job's `WORKFLOW.md`).

The deep design these decisions slot into is
[`SESSION-MUTABLE-SCOPE.md`](./SESSION-MUTABLE-SCOPE.md) (forthcoming
in the workspace `DOCS/` tree; until then, the job's `SCOPE.md` is
the standing reference).

## Q1 — Can WORK read proposed-but-not-yet-approved patches?

**Decision: no.**

A WORK stage executes against the current, merged rulebook —
`SCOPE.md`, `CLAUDE.md`, and the checked-in predicates at HEAD. The
contents of `DOCS/SCOPE-PROPOSED.md` are a queue of unmerged
proposals; reading them from WORK would create two competing rule
sets (current vs proposed) and a session ambiguity that the runtime
cannot resolve. WORK stages therefore treat `SCOPE-PROPOSED.md` as
opaque.

The Step 6 approval UX (CLI and/or UI) is the only consumer of
`SCOPE-PROPOSED.md`. REVIEW stages write to it; humans walk it; WORK
stages ignore it.

**Consequence for the runtime.** The Layer-1 file-set guard adds
`DOCS/SCOPE-PROPOSED.md` to the *append-only-by-REVIEW* file set:
WORK touching it is auto-FAIL, REVIEW appending to it is allowed,
the approval CLI mutating it (to remove an approved entry) is
allowed.

## Q2 — Is RULE-DEPRECATION its own patch type, or is removal the same as addition?

**Decision: removal is the same as addition; classified as a
loosening patch. Prose-only loosening / deletion is confirmed fine.**

`ScopePatch` carries a `kind` discriminant whose values are
`Tighten` and `Loosen`. Deletion of a rule is a `Loosen`: after the
patch lands, the rule no longer constrains, which is the same effect
as a textually-narrower replacement. Same parse-time guard applies:
a `Loosen` patch must cite a positive fixture (the previously-failing
case the loosened rule now permits) plus the `evidence_stage_id` of
the stage whose diff exhibits the fixture.

Prose-only loosening — for example, removing a sentence from
`SCOPE.md` that no predicate enforces — is permitted under the same
`Loosen` rules. The "positive fixture" in that case is a stage whose
output the prior wording would have flagged and the loosened wording
permits.

**Predicate deletion is not a `ScopePatch` kind.** Predicate files
are code; deleting one is a code change. Q5 records the path.

## Q3 — Does a strengthening patch re-trigger review of prior stages?

**Decision: no.**

Patches apply *forward* from the commit that lands them. Prior
stages keep the REVIEW verdicts they earned under the snapshot of
rules in force at the time. Re-running history would (a) require
re-running entire jobs against speculative rule sets, blowing the
session-budget envelope, and (b) destabilise the audit trail, since
"PASS" would become a moving target.

The new predicate, however, must pass on **current HEAD** when the
patch lands — that is the tightening patch's positive case. The
approval flow runs the new predicate in the approving human's commit
before the patch is merged; failure blocks the merge.

**Consequence.** Sessions reading a handover from before a
strengthening patch landed do not get retroactively re-flagged. If a
new rule would have flagged an older session's diff, that is a
backlog item for human triage, not a runtime concern.

## Q4 — Where does the predicate crate live in the crate graph?

**Decision: `crates/codeless-predicates/`, package name
`codeless-predicates`.**

Added as a workspace member in the root `Cargo.toml`:

```toml
members = [
    ...,
    "crates/codeless-predicates",
]
```

The crate is host-only (R1). Its dependency closure includes
`codeless-types` (mobile-safe wire types), `tokio`, and the
process-spawning surface — which it uses **through**
`codeless-adapters-host`, not by re-implementing process spawn. A
predicate that needs to run `cargo` or `git` calls into
`codeless-adapters-host`'s exported runner; the grep "process::Command
outside `codeless-adapters-host`" must remain zero, including inside
`codeless-predicates`.

The crate is shaped like an `xtask`: a binary entry point `cargo
xtask predicates` invoked from CI and from the loop's verify step.
It is unreachable from the mobile shell's dependency closure (the
mobile shell builds `codeless-types` + `codeless-client` only). A
closure check in CI confirms this.

**Cargo.toml member entry (exact):**

```toml
[workspace]
members = [
    "crates/codeless-types",
    "crates/codeless-rpc",
    "crates/codeless-runtime",
    "crates/codeless-adapters-host",
    "crates/codeless-server",
    "crates/codeless-client",
    "crates/codeless-cli",
    "crates/codeless-tauri-desktop",
    "crates/codeless-predicates",
]
```

(Existing entries preserved; only `codeless-predicates` is added.)

## Q5 — Predicate staleness lifecycle (deletion path that does not route through "WORK edits SCOPE.md")

**Decision: predicate deletion is part of the approving human's
commit on a paired loosening patch. Never a WORK or REVIEW automated
edit.**

When a `Loosen` patch removes or weakens a rule whose enforcement
relied on a checked-in predicate, the patch's proposal cites the
predicate that becomes stale. The Step 6 approval UX surfaces both:
"approve loosening + delete predicate file" or "approve loosening +
keep predicate (it still applies to a narrower case)". The approving
human's single commit contains both the prose change and the
predicate file removal.

**Why not via REVIEW.** REVIEW stages do not write code; they
propose patches. Deleting `crates/codeless-predicates/src/foo.rs` is
a code change. Routing predicate deletion through REVIEW would
require REVIEW to land code commits — which is exactly what the
ramp's Layer-1 guards prohibit.

**Why not via WORK.** WORK cannot touch rule-bearing files, and
predicate files are rule-bearing.

**Aux case: a predicate that crashes on every diff.** That is a
bug, not a rule deprecation. Fixed via a normal human-authored
hotfix commit; same path as any other bug in code under
`crates/codeless-predicates/`.

## Q6 — Aggressiveness of prose-to-predicate promotion suggestions

**Decision: conservative. Surface candidates in the approval UI as
informational only; never auto-promote; do not block patch merge on
missing promotion.**

The approval CLI / UI flags a prose rule as a *promotion candidate*
when **both** of the following hold:

- The rule's text contains at least one phrase from a small
  allowlist of categorical words: `must not`, `never`, `always`,
  `zero`, `no <noun>`.
- The same rule has been cited as the FAIL reason in two or more
  prior REVIEW verdicts (the runtime persists REVIEW FAIL cite
  strings in the events table; the approval UX queries it).

Flagging is purely informational: the human sees "consider promoting
to a predicate" alongside the patch. The flag does not block any
patch. Writing a predicate is itself a real code change with its own
review path, and producing a flaky predicate to satisfy an
auto-promotion heuristic would be worse than leaving the rule as
prose.

**Why a conservative threshold.** A noisy prose-to-predicate
suggestion (one fired per REVIEW) trains the human to dismiss the
signal. Two-citation gating plus categorical-phrase gating biases
toward *patterns the human keeps re-explaining*, which is the
correct trigger.

## Provenance

This file is authored by stage 1 of the `session-mutable-scope` job
and committed in the same commit as the stage's `JOB-MODEL.md` /
`JOB-LOOP.md` tightening. A later stage that needs to revise a
decision must amend this file in the same commit as the
revision-bearing code, and the stage's handover must call out the
amendment under `What you need to know`.
