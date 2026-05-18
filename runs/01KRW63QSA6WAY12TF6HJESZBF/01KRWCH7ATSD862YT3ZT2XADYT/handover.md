## Done

- Extracted `POLICY_PRESETS` (with `NO_POLICY` / `POLICY_CUSTOM` sentinels, `policyFromPicker` / `pickerFromPolicy`, and `PolicyPreset` / `PresetPolicyKind` types) from `ui/codeless-ui/src/modules/jobs/composer/policyPresets.ts` to `ui/codeless-ui/src/lib/policy/presets.ts`. Moved the test alongside as `lib/policy/presets.test.ts`. Updated `composer/index.ts` and `composer/JobComposer.tsx` imports to `@/lib/policy/presets`. Deleted the old composer-scoped files.
- Added Rust mirror `crates/codeless-runtime/src/auto_bypass_presets.rs` with the six preset rows (`quick`, `long-term`, `cheap`, `best-judgement`, `just-code`, `relentless`), `policy_for_preset_id`, and a `ts_mirror_in_sync` test that parses the TS file and asserts byte-for-byte parity. Registered as `pub mod auto_bypass_presets` in `crates/codeless-runtime/src/lib.rs`.
- Ran the closing trio: UI `pnpm test` / `pnpm typecheck` / `pnpm lint`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --check`; targeted `cargo test -p codeless-runtime auto_bypass_presets`. All green.
- Committed and pushed via mani as `W3a extract POLICY_PRESETS to ui/codeless-ui/src/lib/policy/presets.ts and Rust mirror` on `feat/assistant-parity` (commit `4a0f83e`).

## Next

- W3b planner prompt builder consumes the preset list, snapshot test covers all seven variants. Touch points: `crates/codeless-runtime/src/rpc/assistant_planner.rs` (currently `build_planner_prompt` does not enumerate auto-bypass variants — see lines around 257 and 390). The W3b consumer should import `crate::auto_bypass_presets::POLICY_PRESETS` and render the seven variants (six presets plus the Custom variant) into the planner prompt with their hints. Add a snapshot test covering all seven.

## What you need to know

- The Rust mirror's `policy_for_preset_id` intentionally omits `Custom` — Custom carries operator free text the planner cannot synthesise from a preset row. W3b's seven-variant snapshot will need to render Custom separately (with no canned hint).
- `AutoBypassPolicy::policy_name()` returns `"Best-judgement"` (hyphen) for the badge wire name while the TS preset label is `"Best judgement"` (space). They are deliberately separate strings — `policy_name()` is the stable enum tag for events / badges, the preset label is the picker display. I dropped a parity test linking the two during development; do not re-add one without changing one side.
- Pre-existing test failure unrelated to W3a: `cargo test -p codeless-runtime --test migrations migrator_creates_all_tables_from_appendix_a` fails because the test expects a `todos` table the runtime migrator does not yet create. Confirmed pre-existing by stashing my changes and re-running on `9d53514`. Comes from the parallel `todos-recorder-and-gate` job per SCOPE.md sequencing dependency.
- The TS file's `POLICY_PRESETS` entries each span four lines (id / label / hint / closing brace). The Rust `ts_mirror_in_sync` test parses by line-prefix on `id:`, `label:`, `hint:` — if a future edit reformats the TS file (e.g. inlines the entries onto one line) the parser will need updating.

## Open questions

- (none)
