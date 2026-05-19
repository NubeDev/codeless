## Done

- Reviewed cumulative job diff (stages 1–12) for R1/R2/R4/R5 invariants
- Confirmed `-types` changes (FailureClass + StopReason variants) are additive and iOS/Android-safe
- Confirmed UI changes route exclusively through `RpcClient` / regenerated `wire.ts`; no `@tauri-apps/*` imports added; `ReviewGatePanel.tsx` untouched
- Emitted PASS sentinel

## Next

- (none) — fresh session picks up stage 15

## What you need to know

- This stage is REVIEW-only; no code committed
- The end-to-end UI surfacing test coverage was actually landed in stage 12 (commit 7a127e0): `StagesOverview.test.tsx` covers `~` glyph + tooltip with policy + `failure_detail`, `JobTimeline.test.tsx` covers the chip tooltip carrying threaded `Previous-stage failure` block, and `ReviewGatePanel.tsx` was not modified in this job
- `JobTimeline.tsx` source already rendered the bypass chip before this job (last touched in `21d9f8b`); only test coverage was added here
- Wire-format additions: `FailureClass::"infrastructure-error"`, `StopReason::"infrastructure"`, four optional `StageRollup` columns (`bypassed_at`, `bypassed_reason`, `failure_class`, `failure_detail`) — all additive

## Open questions

- (none)

PASS: -types changes are additive enum variants, UI changes stay inside the RpcClient boundary, ReviewGatePanel is untouched, and all wire-format extensions are backward-compatible.
