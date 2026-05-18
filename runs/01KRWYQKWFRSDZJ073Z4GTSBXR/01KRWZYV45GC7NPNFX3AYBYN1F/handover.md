## Done

- nothing — stage 3 was already implemented and committed as 89b63cb on branch `codeless/scoped-pause-points` before this session started

## Next

- stage 4 (parser) per the job plan — picks up `pause_points:` in template.yaml, resolves stage names to ordinals, enforces the 512-byte reason cap, and emits `PausePoint` values

## What you need to know

- types live in `crates/codeless-types/src/pause_point.rs`: `PausePointId(Ulid)`, `PausePointPosition::{Before,After}` (kebab-case), `PausePointTarget::{Stage{ordinal}, StageTodo{stage_ordinal, selector}}` tagged by `kind`, `TodoSelector::{Ordinal, Trio, TitleSubstring}` tagged by `selector`, and `PausePoint{id, target, position, reason: Option<String>}`
- specta derives present; iOS/Android-safe (only serde + ulid + specta deps, matching the rest of `codeless-types`)
- round-trip serde tests cover every variant plus the `reason`-missing default; specta snapshot updated at `crates/codeless-types/tests/wire.ts.snap`
- workspace `cargo test` cannot run inside this worktree because `codeless-server` references `../../ai-ui/crates/ai-ui-core` which is outside the worktree path — verify on a full checkout, not here

## Open questions

- (none)
