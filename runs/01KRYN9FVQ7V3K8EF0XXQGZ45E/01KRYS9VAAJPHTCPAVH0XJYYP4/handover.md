## Done

- added `bypassed: boolean` field to the internal `StageData` (the "StagesOverview StageRow") type in `ui/codeless-ui/src/modules/jobs/StagesOverview.tsx`
- derived the flag in `mergeRollup` from `rollup.stage.bypassed_at != null`
- preserved the flag in the `stage-started` rebuild path via `existing?.bypassed ?? false`; other reducer arms carry it through their existing `...existing` spreads
- committed as `stage 9: codeless-types::Event and the list_stages payload`

## Next

- next stage: surface bypassed-after-failure as a distinct glyph (`~`) + tooltip in `StageRow` / `stageGlyph`, sourcing the reason from `rollup.stage.bypassed_reason` (will require threading that through `StageData` too)

## What you need to know

- wire `StageRow` already exposes both `bypassed_at` and `bypassed_reason` (see `ui/codeless-ui/src/lib/rpc/generated/wire.ts` L2575 / L2582 and matching Rust in `crates/codeless-types/src/stage.rs`); no schema or `codeless-types::Event` change is needed for stage 11
- `bypassed` is intentionally rollup-only — the event stream never carries the bypass timestamp, so re-running `list_stages` is the only way to flip it
- `cd ui/codeless-ui && pnpm typecheck` cannot run in this worktree (no `node_modules`); validate after a `pnpm install`

## Open questions

- (none)
