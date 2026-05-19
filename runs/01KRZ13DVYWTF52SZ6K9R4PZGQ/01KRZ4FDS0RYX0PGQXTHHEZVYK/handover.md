## Done

- reviewed stages 3 + 4 diff against BROWSER-LAUNCHER.md §"RPC additions" and SCOPE.md
- confirmed `EventFilter` shape: `All | Job{job_id} | Repo{repo_id} | Library` (kebab-case `scope` tag), `All` retained for global log per SCOPE.md
- confirmed every `Fs*Args` (including new `FsCwdArgs`) carries `repo_id`; `RpcServer::fs_cwd` signature now takes args
- confirmed runtime `fs_root_for_repo` reads `fs_root_canonical` from `attached_workspaces`, defence-in-depth registration check via `HostFs::root_is_registered`, returns `NotFound` on unknown/detached
- confirmed event fan-out matches via payload `repo_id` first, then a `job→repo` snapshot refreshed live from `JobQueued`
- confirmed SSE route accepts `scope=repo&repo_id=…` and `scope=library`, HTTP client builds the same; Tauri desktop `rpc_fs_cwd` shim updated
- confirmed TS `EventFilter` extended; `wire_ts.rs` registers `FsCwdArgs` + sibling Fs args
- R1: no new `tokio::process`/`std::process::Command` outside `codeless-adapters-host` (pre-existing hits in runtime/tools are unchanged by this diff)
- R2: no new `@tauri-apps/*` imports outside `src/shells/desktop/`
- R4: SQLite (`attached_workspaces`) is source of truth; adapter check is secondary
- R5: bearer-token trust boundary unchanged

## Next

- stage 6 (UI plumbing) — pass `activeRepoId` on every `subscribe` and every `fs.*` call; subscribe to `Library` in parallel for picker liveness

## What you need to know

- PASS: EventFilter gains Repo/Library while preserving All for the global log, every fs.* arg carries repo_id with a typed NotFound on unknown/detached repos, and R1/R2/R4/R5 invariants all hold across the stage 3+4 diff.
- The wire is now locked for this job; UI stages must bind to the shape above
- `InProcessRpc::fs_root_for_repo` is the helper future per-workspace RPCs should reuse
- `TauriIpcClient` callers must also send `repo_id` on `fs.*` and the new EventFilter variants; the Rust `rpc_fs_cwd` command was updated, but UI-side `TauriIpcClient` updates remain for stage 6

## Open questions

- (none)
