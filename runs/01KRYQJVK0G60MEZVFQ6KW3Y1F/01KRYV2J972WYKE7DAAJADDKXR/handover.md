## Done

- Added `restart_server` to `RpcServer` trait (`crates/codeless-rpc/src/server.rs`) and the in-process implementation in `crates/codeless-runtime/src/rpc/restart.rs`.
- Implemented running-jobs partition: resumable iff runner ∈ {`mock`, `anthropic`} AND latest stage transition within `RESUMABLE_WINDOW` (30s); everything else (PTY-bound `claude` / `codex` / `copilot`, or stale checkpoint) → killed.
- `force = false` refuses with `AdapterError::RestartHasRunningJobs { resumable, killed }` whenever any job is `Running`; `force = true` proceeds.
- Three execution contexts wired through `RestartContext` (`SupervisedCli` → exit 75; `TauriDesktop` → exit 0; `Bare` → `AdapterError::RestartUnsupervised` with a hint mentioning `--respawn-on-exit`).
- New host adapter module `crates/codeless-adapters-host/src/respawn.rs` exposes `supervise(child_argv)` (R1-gated process spawn) and the `CODELESS_SUPERVISED` env marker.
- New CLI flag `codeless serve --respawn-on-exit`: parent strips the flag, sets the env marker, and re-execs `current_exe()` until the child exits with a non-`EX_TEMPFAIL` code.
- New `serve_with_extra_shutdown` in `codeless-server::lib` so the CLI's listen loop drains on either Ctrl-C or the `RestartTrigger` notify; the CLI returns `ExitCode::from(desired_exit_code)` after the listener drops.
- HTTP route + client wiring for `restart_server`; `RestartServerArgs` / `RestartServerResult` gained `Default` so the route can accept a missing body.
- Integration test `crates/codeless-runtime/tests/restart_server_partition.rs` (5 tests, all green) asserts the partition, the `force=true` escape, the bare-context refusal, the Tauri-context exit-0 path, and the empty-set proceed path.
- `cargo build --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test -p codeless-runtime -p codeless-types -p codeless-rpc -p codeless-server -p codeless-client -p codeless-cli` all green. Committed as `7da9d4b` on `codeless/adapter-registry`.

## Next

- Stage 8: lift Slack / Telegram adapter spawn out of `codeless-cli/src/serve.rs` into a boot-time `ChatAdapterRegistry` driven by the `chat_adapters` table; lift `DefaultRunnerFactory.enable_*` into a `RunnerConfig` driven by `runner_config`. The `--enable-*` flags continue as upsert shims; no behavioural change beyond source of truth.

## What you need to know

- The runtime constructor defaults `RestartContext::Bare`, so unit tests cannot accidentally arm a shutdown; the CLI sets the context once at boot via `InProcessRpc::with_restart_context`.
- `RestartTrigger` lives on `InProcessRpc` behind `restart_trigger()`. It is `Clone` (Arc-shared `Notify` + `parking_lot::Mutex<TriggerState>`); the CLI clones it once for the listener select and once to read `desired_exit_code()` after drain.
- The `is_resumable_runner` set is a one-line match on the runner id (`mock` | `anthropic`). New template-driven runners need both this list and the `DefaultRunnerFactory` factory wiring; PTY-bound runners stay out of the list deliberately.
- R1 check: `rg 'tokio::process|std::process::Command' crates/ --type rust` outside `codeless-adapters-host` shows only pre-existing matches (the long-standing `which`-shell-out in `serve.rs::resolve_mcp_binary`, and the test-only `Command::new("git")` calls used to set up fixture repos). No new process-spawn site outside the host adapter.
- Pre-existing flakes unchanged: three `codeless-adapters-host` `git_commit` / `git_diff` tests race on cwd under parallel `cargo test --workspace`; they pass under `--test-threads=1`. Not in this stage's blast radius.
- The four open questions are resolved in `DOCS/SCOPE.md` §"Adapter registry, stage 1" (committed pre-stage by `080857e`). Stage 8 inherits the validate-cache being process-lifetime: after restart, the operator must re-validate before re-enabling — this is deliberate UX and is what "restart-arms" means downstream.

## Open questions

- (none)
