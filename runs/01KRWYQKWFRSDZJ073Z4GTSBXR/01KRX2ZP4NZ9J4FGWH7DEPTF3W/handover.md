## Done

- Reviewed stages 1–6 diff (wire types, persistence migration, parser, store, runtime hook in template_runner + trio_emitter, scoped_pause_hook module, resync rebuild)
- Verified R1: codeless-types/pause_point.rs has no host deps; runtime-only code stays in codeless-runtime; no new process::Command sites
- Verified R2: hook calls existing pause_job state transition + EventBus JobPaused; no new transport
- Verified R4/R5: resume goes through existing resume_job; StopReason::ScopedPausePoint is the only state-machine surface change
- Verified wire-format changes are additive: new PausePoint*/TodoSelector types + new StopReason variant; snap regenerated, no breaks
- `cargo build -p codeless-types -p codeless-runtime` clean

## Next

- Stage 8 (next session) — RPC + UI surfaces for chat-driven add/remove of pause points and the "planned pause" divider rendering

## What you need to know

- Review stage emits a sentinel; verdict line is `PASS: Layer-1 invariants hold — new pause-point wire types are types-crate-pure, the runtime hook reuses the existing pause_job primitive and JobPaused event, and the wire-format changes are additive only.`
- No code changes this stage; nothing to commit (working tree clean)
- Pre-existing `process::Command` usage outside codeless-adapters-host exists in other crates but is unrelated to this job's diff

## Open questions

- (none)
