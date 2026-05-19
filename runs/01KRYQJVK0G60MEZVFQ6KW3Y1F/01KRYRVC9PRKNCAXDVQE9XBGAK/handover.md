## Done

- Added `crates/codeless-types/src/adapter.rs` with the full adapter-registry wire surface: `ChatAdapterKind` (`Slack`, `Telegram`; `Gmail` deliberately omitted per SCOPE), `ChatAdapterRow`, `RunnerRow`, `ListChatAdaptersResult`, `ListRunnersResult`, `SetChatAdapterEnabledArgs`, `SetRunnerEnabledArgs`, `ValidateChatAdapterSecretsArgs`, `ValidateChatAdapterSecretsResult` (with `ChatAdapterSecretProblem` payload), `RestartServerArgs { force }`, `RestartServerResult {}`, and `AdapterError` enum with `MissingSecrets { keys }`, `ValidationFailed { reason }`, `RestartUnsupervised { hint }`, `RestartHasRunningJobs { resumable, killed }`, `Conflict`, `NotConfigured`. All derive `serde` + `specta::Type`; live in the iOS/Android-safe `codeless-types` crate.
- Re-exported the new types from `codeless-types::lib` and `codeless-rpc::lib`.
- Added `RpcError::Adapter(AdapterError)` variant to `codeless-rpc/src/error.rs`; mapped it to HTTP 409 with a JSON `AdapterError` body in `codeless-server/src/routes.rs` (mirroring the existing `Workspace(WorkspaceError)` pattern); added the missing match arms in `codeless-runtime/src/job_driver_loop.rs` (non-retryable, kind label `"adapter"`).
- Registered every new type in `crates/codeless-types/tests/specta_snapshot.rs` and regenerated `wire.ts.snap`.
- `cargo test -p codeless-types -p codeless-rpc -p codeless-runtime -p codeless-server` all green; `cargo fmt --check` clean on the touched crates.
- Committed as `b2961f5` on `codeless/adapter-registry` with the stage-5 title.

## Next

- Stage 6: implement the four chat-adapter RPCs (list / set_enabled / validate / restart-arming) plus the two runner RPCs end-to-end in `codeless-rpc` (trait method additions) + `codeless-runtime` (handler logic). Wire the validate-before-enable coupling so `set_chat_adapter_enabled(true)` returns `AdapterError::MissingSecrets` / `ValidationFailed` without a cached prior `validate_chat_adapter_secrets`. Apply the per-`(kind, instance_id)` 5/s rate limit and 5s hard timeout.

## What you need to know

- `AdapterError` is serialised as `kebab-case` externally tagged (matches `WorkspaceError` convention) so the UI branches on the variant string without parsing free text. Transport mapping is HTTP 409 with the JSON `AdapterError` in the body, same shape `WorkspaceError` already uses.
- `ChatAdapterKind` is `kebab-case` (`slack`, `telegram`). `Gmail` is left out on purpose — adds when the `codeless-gmail` crate lands (per the doc).
- The `JobId` import in `adapter.rs` is needed for `RestartHasRunningJobs { resumable: Vec<JobId>, killed: Vec<JobId> }`.
- `crates/codeless-cli` has a pre-existing clippy error (`no compose_system_prompt in serve`) on master/this branch; `cargo clippy --workspace` fails on it independent of these changes. Scoped clippy on `codeless-types` + `codeless-rpc` is clean.
- Build hiccup: `../ai-runner/Cargo.toml` pins `workspace = "../job-01KRYN9FVQ7V3K8EF0XXQGZ45E"` (a different worktree). I temporarily flipped it to this worktree to run cargo, then reverted it. Future stages need to do the same workaround if they need workspace-level cargo. The CODELESS/JOB-LOOP infra owns that pointer; do not commit the flip.
- The follow-up RPC stage will need to add typed `RpcServer` trait methods (`list_chat_adapters`, `set_chat_adapter_enabled`, `validate_chat_adapter_secrets`, `list_runners`, `set_runner_enabled`, `restart_server`) — args/result types are already in place.

## Open questions

- (none)
