## Done

- audited stage 1–8 diff (7d3e9e8..c31bded) against Layer-1 invariants: crate dep direction, single transport, trust boundary, wire-format stability

## Next

- (none) — gate sentinel below decides whether the job advances

## What you need to know

- snapshots `wire.ts.snap{,.actual}` and `wire-rpc.ts.snap{,.actual}` are byte-identical; the `.actual` files are not drift
- the `process::Command` hits in `template_runner.rs` and `trio_emitter.rs` predate this job (verified against base `d0ce6ab`); the new files (`scoped_pause_hook.rs`, `store/scheduled_pause_points.rs`, `template.rs` additions, `pause_point.rs`) introduce none
- resume of a scoped pause reuses existing `resume_job`; no transport-shaped surface was added

## Open questions

- (none)
