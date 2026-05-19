## Done

- audited every production `EventBus::publish` site in `crates/codeless-runtime/`, classified each by event kind, envelope `(job_id, stage_id, task_id)`, and how a server-side `EventFilter::Repo` filter can resolve `repo_id`
- committed the report as `DOCS/EVENT-PUBLISH-AUDIT.md` on branch `codeless/workspace-scoping`, commit `702f9c2` titled `stage 1: publish-site report`

## Next

- (none) — fresh session picks up stage 2 of 10

## What you need to know

- the only Event variants that carry `repo_id` in payload are `RepoAdded`, `RepoRemoved`, `RepoUpdated`, `JobQueued`, `WorkspaceUnhealthy`, `WorkspaceRecovered`; everything else relies on envelope `job_id` → `jobs.repo_id`
- 11 red-row publish sites cannot resolve `repo_id` as the wire stands: `rpc/reviews.rs:{60,90,108}` and `rpc/scope_patches.rs:{116,174}` pass `job_id=None`; `rpc/assistant.rs:39`, `auto_bypass_failure_card.rs:117`, `rpc/assistant_planner.rs:168`, `rpc/chat.rs:153` stuff a synthetic `JobId(thread_id.0)` or `session_id` into the `job_id` slot
- test-only publish sites (`#[cfg(test)]` modules, `tests/*.rs`) were intentionally excluded — they do not feed real subscribers
- I did not run `cargo test` / `cargo clippy` / `cargo fmt --check` — this stage adds documentation only, no Rust source changed

## Open questions

- M5's contract for the assistant + unbound-chat surfaces: declare them `Library` (the report's recommendation) versus thread a real `repo_id` onto `assistant_threads` / chat sessions. Recommendation in the audit is `Library` for now; explicit confirmation belongs in a later stage.
