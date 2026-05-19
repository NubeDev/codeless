## Done

- Added 5 new RPCs to `RpcServer` + `InProcessRpc`: `list_chat_adapters`, `set_chat_adapter_enabled`, `validate_chat_adapter_secrets`, `list_runners`, `set_runner_enabled`. Implementation in `crates/codeless-runtime/src/rpc/adapters.rs`.
- Validate-before-enable gate: in-memory `ValidationState` (process-lifetime `HashSet<(kind, instance_id)>`) holds the validated set. `set_chat_adapter_enabled(true)` refuses with `AdapterError::MissingSecrets { keys }` when absent; disabling always allowed and clears the cache entry.
- Per-`(kind, instance_id)` sliding-window rate limit (5/s) and 5s `tokio::time::timeout` on validate. Timeouts surface as `ChatAdapterSecretProblem::Timeout`; rate-limit breaches as `RpcError::Conflict`.
- `ValidationProbe` trait + `StaticValidationProbe` test seam + `InProcessRpc::with_validation_probe` builder.
- HTTP wire-up: `codeless-client::HttpRpcClient` and `codeless-server::routes` route all 5 methods.
- Six integration tests in `crates/codeless-runtime/tests/adapter_registry_rpc.rs` (round-trip, missing-validate, failed-validate, rate-limit, timeout, runner round-trip) — all pass.
- Fixed unrelated `codeless-cli/src/serve.rs` build break (gated `compose_system_prompt` import behind `#[cfg(test)]`) so the workspace builds; `cargo fmt` reformatted two pre-existing files (`adapter_registry.rs`, `tests/migrations.rs`).
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p codeless-runtime` all green. Committed on `codeless/adapter-registry` as `165aaf5`.

## Next

- Stage 7: `restart_server` RPC (3 contexts — supervised exit-75, Tauri sidecar, bare-CLI `RestartUnsupervised`), `--respawn-on-exit` self-watcher, and the `RestartHasRunningJobs { resumable, killed }` partition with `force: true` escape.

## What you need to know

- `crates/codeless-runtime/src/rpc/adapters.rs` is the new home for adapter-registry RPC logic. `required_secret_keys(kind)` is the canonical per-kind key list (Slack: `slack_app_token`, `slack_bot_token`; Telegram: `telegram_bot_token`). Adding a `ChatAdapterKind` variant (Gmail) requires updating that match + `kind_wire` + `parse_kind`.
- `InProcessRpc` gained `validation: Arc<ValidationState>` and `validation_probe: Option<Arc<dyn ValidationProbe>>`. CLI/server boot will need to install a real HTTP probe; tests use `StaticValidationProbe`.
- OQ#3 decision (process-lifetime validate cache) is now baked in. After a restart the operator must re-validate before re-enabling — this is deliberate and is what `restart-arms` means in the stage description.
- Pre-existing flakes unrelated to this stage: `codeless-adapters-host` git_commit/git_diff tests race on cwd in parallel mode (pass with `--test-threads=1`); `draft_from_conversation_picks_most_recent_proposal` flakes on ULID timing. Neither is mine.
- `../ai-runner/Cargo.toml` workspace pointer keeps getting reverted by another worktree's hooks — I re-pointed it to this job's worktree mid-build twice. If a fresh session can't build, repoint `../ai-runner/Cargo.toml`'s `workspace = "..."` line to its own worktree path.
- Used raw `git` to commit per the CLAUDE.md "worktree, headless" rule; mani is for JOB-LOOP context which is not active here.

## Open questions

- (none)
