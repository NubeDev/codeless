# JOB-MODEL.md — the handover contract

The handover is the only contract between sessions. A run writes one
markdown document at `runs/<job_id>/handover.md` (per stage, not per
job — see H1 in `SESSION-MUTABLE-SCOPE.md`) inside its worktree on
completion; the next session reads that file before anything else and
makes its first decision from it. There is no other channel. The
session that emits a malformed handover has effectively shipped no
work, because the next session cannot pick it up.

The wire form is `crates/codeless-types/src/handover.rs`. This
document is the prose authority for what each field *means*, with one
worked example and one anti-example per section so the contract is
unambiguous to both the model writing the handover and the runtime
validating it.

## Shape

Four `##` headings, in this fixed order, each section a bullet list
(`-` or `*`). Empty sections render `- (none)` so a parser still
finds the heading.

```
## Done
## Next
## What you need to know
## Open questions
```

Prose between bullets is ignored by the parser. Unknown headings are
a parse error — the contract is the four canonical sections, full
stop.

**Worked example (skeleton, empty stage).**

```
## Done

- (none)

## Next

- (none)

## What you need to know

- (none)

## Open questions

- (none)
```

An empty stage is still a well-formed handover: every heading is
present in order, every section renders `- (none)`. The runtime's
write-time validator (see **Validation** below) rejects this on the
grounds that `Done` cannot legitimately be empty, but the *parse*
succeeds — the distinction matters because the parser and the
validator are separate failure modes.

**Anti-example (missing heading, reordered, prose where a list
belongs).**

```
## Done
- shipped the predicate runner

## What you need to know
- host-only crate

## Next
- write the parse-time guards

(no Open questions section)
```

Three faults: `Next` and `What you need to know` are swapped, the
`Open questions` heading is missing entirely, and the handover ends
with a prose paragraph instead of a fourth section. The parser
rejects on the first fault and never reads the rest — handover
authors who "save space" by omitting empty sections produce
unparseable artifacts.

## Done

What landed in this session. Committed code, decisions ratified by a
REVIEW stage, anything the next session does **not** need to redo.
Each bullet is one shipped thing.

**Worked example.**

```
## Done

- created `crates/codeless-predicates/` workspace member (Cargo.toml
  updated; depends on `codeless-types` only)
- seeded three predicates: no-tokio-process-outside-adapters-host,
  no-emoji-in-source, handover-four-sections-present
- wired `cargo xtask predicates` to run all seeded probes against the
  current diff and exit non-zero on failure
```

Each bullet names *what* and points at *where*. A reader who has not
seen this session can map every bullet to a file or commit.

**Anti-example.**

```
## Done

- worked on predicate runner
- made progress on stage 3
- some tests pass
```

This is unactionable. "Worked on", "made progress", "some tests"
force the next session to re-derive what was actually completed by
reading the diff — defeating the point of the handover.

## Next

What the next session should pick up first. The **top bullet is the
canonical next action**; the runtime treats it as the seed prompt for
the next session.

**Worked example.**

```
## Next

- land stage 4: emit `ScopePatch` proposals from REVIEW stages to
  `DOCS/SCOPE-PROPOSED.md` in shadow mode
- before writing code, re-read
  `DOCS/SESSION-MUTABLE-SCOPE-DECISIONS.md` question 4 for the exact
  crate path
```

The top bullet is a single concrete action with a verifiable end
state. The second bullet front-loads the prerequisite the next
session must not skip.

**Anti-example.**

```
## Next

- continue
- maybe look at the review code
- think about whether the test should be in runtime or types
```

"Continue" is not an action. "Think about" is a question, not a
next step — it belongs in **Open questions**. Drift like this is how
sessions waste a re-read of the entire job to find the next move.

## What you need to know

Constraints, invariants, gotchas that a fresh reader would not infer
from the diff alone. The bar is *load-bearing context* — if the next
session can derive it from the code, leave it out.

**Worked example.**

```
## What you need to know

- the predicate crate is host-only per R1; do not add it to the
  mobile shell's dependency closure or the closure check in CI fails
- `MockRunner::with_script` was extended with `expect_diff_paths`;
  any new test that asserts on handover diff verification should use
  it instead of hand-rolling a temp git repo
- `SCOPE-PROPOSED.md` is appended-to, never rewritten; the patch
  parser keys on the trailing `---`-separated patch block
```

Each entry is a fact the next session would otherwise have to
discover by trial and error.

**Anti-example.**

```
## What you need to know

- this is a Rust project
- we use cargo
- there are tests in the crates/ directory
- be careful with the code
```

Surface trivia and platitudes. A new session reading
`crates/*/Cargo.toml` learns all of this in one second. "Be careful"
is not a constraint.

## Open questions

Unresolved decisions. The next session **resolves these before
implementing anything new**. Each entry names a real choice with
real options, not a vague worry.

**Worked example.**

```
## Open questions

- should `ScopePatch.kind` carry a free-form string or an enum? An
  enum locks the parser harder (good for Layer-1 guards) but forces a
  schema bump for every new patch kind. Recommend enum + explicit
  `Unknown(String)` fallback for forward compat — confirm before
  stage 4 code lands.
```

A reviewer can answer "yes/no/option C" without further investigation
because the question states the trade-off.

**Anti-example.**

```
## Open questions

- not sure about the patch design
- naming?
- might be a problem with concurrency
```

These force the next session to interview the previous session — but
the previous session is gone. If the question is unframable, the
right move is to do the investigation in this session and frame it,
or escalate the stage to `[!]` and halt.

## Validation

The runtime validates handover write-time (H7). Rejected handovers
mean the stage is not "done" — the session must repair the document
before commit + push. The current checks:

- All four headings present, in order.
- `Done` is non-empty (no run can legitimately have done nothing —
  if a stage aborts, the `Done` entry is the abort record itself).
- `Next` is non-empty when the job has remaining stages.
- Every path mentioned in `Done` must appear in the stage's git diff
  (diff-verify, see Step 2). A path in `Done` that the commit did
  not touch is auto-FAIL with no model invoked.

Schema versioning, not REVIEW patches, governs this file: the wire
format is sacred per `SCOPE.md`, and the wire-format file list in
`JOB-LOOP.md`'s **Rule-bearing files** section pins `JOB-MODEL.md`,
`JOB-LOOP.md`, and `handover.rs` as off-limits to the patch path
(`SESSION-MUTABLE-SCOPE-DECISIONS.md` provenance rule). A change to
the shape requires a `schema_version` bump in `handover.rs` plus a
migration.

**Worked example (validator rejection that the session must
repair).**

```
## Done

- updated crates/codeless-runtime/src/template_runner.rs to parse
  the PASS/FAIL sentinel

## Next

- wire diff-verify in front of the REVIEW prompt

## What you need to know

- ...

## Open questions

- (none)
```

…written by a session whose commit only touched
`crates/codeless-runtime/src/review_gate.rs`. Diff-verify rejects:
the path in `Done` does not appear in the commit's diff. The fix is
*not* to delete the bullet; it is to update the bullet to the path
the commit actually touched (or to amend the commit so the diff
matches the claim).

**Anti-example (validator pretends to pass).**

```
## Done

- everything that the stage was supposed to do
```

`Done` is non-empty, so the cheapest validator passes; diff-verify
has nothing to check because no path is named. The next session
inherits a handover that says nothing happened *and* nothing
useful, with no way to recover the actual landing surface short of
reading the diff by hand — which is the failure mode the handover
exists to prevent.
