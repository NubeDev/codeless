## Done

- Refactored `crates/codeless-runtime/src/rpc/assistant_planner.rs` to import `crate::auto_bypass_presets::POLICY_PRESETS` and render the auto-bypass policy catalogue from that list instead of the hand-baked `PLANNER_AUTO_BYPASS_POLICY_DOC` const.
- Split the old monolithic const into three pieces (`PLANNER_AUTO_BYPASS_POLICY_HEAD`, `PLANNER_AUTO_BYPASS_CUSTOM_ROW`, `PLANNER_AUTO_BYPASS_POLICY_TAIL`) and added `append_auto_bypass_policy_doc` which loops the six presets and appends the Custom row (the seventh variant, not in the preset list because Custom carries operator free text).
- Added four tests covering all seven variants:
- `auto_bypass_policy_doc_snapshot` — byte-exact rendered string assert
- `auto_bypass_policy_doc_covers_every_variant` — each of the seven variant ids appears exactly once
- `build_planner_prompt_includes_auto_bypass_catalogue_when_set_policy_visible` — end-to-end render through `build_planner_prompt` with `assistant.*` persona; asserts head/tail framing reachable through the public entry point
- `build_planner_prompt_omits_auto_bypass_catalogue_when_set_policy_hidden` — negative case: personas without `set_policy` granted see no catalogue
- Closing trio green: `cargo test -p codeless-runtime --lib assistant_planner` (19 pass), `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo fmt --check` clean, `cargo test --workspace` green except one pre-existing failure (`worktree_preserved_when_stop_wins_against_completion`) confirmed unrelated by stash + retest.
- Committed `16bbdf6` and pushed via mani on `feat/assistant-parity`.

## Next

- W3c: draft_job card embeds the policy picker, update card dispatches `set_job_policy` with the paused-job rule. Touch points per parity doc §W3: `ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx` (renders the new card types — folds into the W1 shared tool-card surface). The planner's `draft_job` already accepts an optional `auto_bypass_policy` field per the existing `BuiltinAssistantTool` args; the card UX work is wiring the picker into the composer-embedded card from W2a, and adding the `update` card that dispatches `set_job_policy` honouring the `AUTO-BYPASS-DECISIONS.md` Q5 paused-job rule.

## What you need to know

- `append_auto_bypass_policy_doc` writes `\n` first to give a blank line between the per-tool list and the catalogue head. The TAIL also starts with `\n` to give a blank line between the Custom row and the "Propose `set_policy` when..." paragraph. If you reformat the const trio, double-check the snapshot expectation: the snapshot test asserts byte-exact equality and will fail loudly on any whitespace drift.
- Custom is intentionally not in `POLICY_PRESETS` (W3a handover called this out). It is rendered by the dedicated `PLANNER_AUTO_BYPASS_CUSTOM_ROW` const so a future preset addition does not accidentally drop it. The cardinality test caps the variant count at 7 — if a new `AutoBypassPolicy` variant lands, that test will need updating in the same commit as the new preset/Custom row.
- The pre-existing `worktree_preserved_when_stop_wins_against_completion` failure persists unchanged on master + this branch; not caused by W3b. Same flakiness class as the migrator `todos`-table miss W3a flagged.
- The shell `cd` keeps resetting back to `codeless/` between bash invocations, so `mani` calls need to be prefixed with `cd /home/user/code/rust/codeless-workspace && ./bin/mani --config mani.yaml ...` each time.

## Open questions

- (none)
