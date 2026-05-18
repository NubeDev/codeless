## Done

- reviewed stage 1 diff: DOCS/SCOPED-PAUSE-POINTS.md grammar + rejection rules, DOCS/SCOPE.md cross-ref, job SCOPE.md open-question resolutions, handover.md
- confirmed no Rust source touched and the design respects R1, single-transport, R4/R5, and wire-format invariants
- PASS: design keeps StopReason variant in codeless-types, table in host-only runtime, no new transport, no wire-format break

## Next

- (none) — fresh session picks up stage 3 (wire types in codeless-types)

## What you need to know

- PASS: stage 1 is docs-only, additive design that schedules on top of the existing pause_job primitive without altering crate-dep direction, transport, trust boundary, or wire formats
- Sentinel above is the runtime-parsed gate signal; FAIL would have halted the job
- Grammar reference lives at DOCS/SCOPED-PAUSE-POINTS.md; in-flight brief at .codeless/jobs/scoped-pause-points/SCOPE.md

## Open questions

- (none)
