## Done

- `StageSpec` (`crates/codeless-runtime/src/template.rs`) gained optional `persona: Option<String>` decoded from the structured-form YAML; bare-string stage entries cannot carry it by design. Empty / whitespace strings collapse to `None`.
- `PlannedStage` exposes the borrowed `persona: Option<&str>` so the orchestrator can read per-stage overrides without re-walking the source YAML.
- `Event::StageStarted` (`crates/codeless-types/src/event.rs`) gained `persona_id: Option<String>` with `#[serde(default)]`, so persisted-bus replays of older envelopes still decode.
- `TemplateRunner` resolves the stage persona via `SqliteStore::get_persona` at run time and applies its `instructions` as the stage's runner system prompt; inheritance order is stage `persona.instructions` → job-level `system_prompt` → runner default. The `StageStarted` envelope echoes the resolved id so the recorder can stamp `stages.persona_id`.
- `stage_recorder` reads `persona_id` from `Event::StageStarted` and writes it onto the row (replacing the prior hard-coded `None`).
- `submit_job` (`crates/codeless-runtime/src/rpc/jobs.rs`) validates every `stage.persona` against `personas` before scaffolding the job directory; an unresolved id returns `RpcError::InvalidArgument` and never reaches the runner.
- Regenerated `crates/codeless-rpc/tests/wire-rpc.ts.snap` and `ui/codeless-ui/src/lib/rpc/generated/wire.ts` to pick up the new `Event::StageStarted.persona_id` and `Stage.persona_id` fields.
- New tests: `template::tests::parses_per_stage_persona_override_on_structured_form`, `template::tests::empty_persona_string_collapses_to_none`, `template_runner::tests::stage_started_event_carries_per_stage_persona`, `rpc_in_process::submit_job_rejects_unknown_stage_persona`, and `rpc_in_process::submit_job_accepts_known_stage_persona`.
- `cargo build --workspace`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `pnpm -C ui/codeless-ui typecheck` are clean. New tests pass.

## Next

- Stage 10: expose personas where `use_for_jobs = 1` as MCP prompts; `use_for_jobs` is the single dimension gating MCP visibility — do not add a parallel `expose_via_mcp` flag (D3).

## What you need to know

- D5 is honoured: there is no special-case for REVIEW stages or for `builtin:reviewer`; the `persona:` key is resolved and applied uniformly for every stage. The reviewer-default itself remains owned by `DOCS/SESSION-PEER-REVIEW-IMPROVEMENTS.md`.
- D4 is honoured: the runner composes the system prompt from `persona.instructions` alone; `default_snippets` is still chat-only.
- Validation is fail-fast at submit. The runner additionally tolerates a missing persona row at run time (returns `Ok(None)` and degrades to the job-level prompt) — that covers the edge case where a user deletes a persona row between submit and dispatch.
- The pre-existing flake `rpc_in_process::job_filtered_subscription_drops_unrelated_events` still fails the same way it did at stages 7 and 8 (verified by `git stash`); not caused by this stage. `codeless-adapters-host::git_commit::commit_paths_creates_commit_for_new_file` likewise reproduces on `git stash`.
- The `wire-rpc.ts.snap.actual` file is tracked in the repo (it was created earlier when the snapshot drifted) — `SPECTA_UPDATE=1` regenerated both `.snap` and `.actual`, and they now match.

## Open questions

- (none)
