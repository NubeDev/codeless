## Done

- threaded `repo_id` through every `fs.*` RPC arg type (`FsReadDirArgs`, `FsReadFileArgs`, `FsWriteFileArgs`, `FsStatArgs`, `FsCreateFileArgs`, `FsCreateDirArgs`, `FsMoveArgs`, `FsDeleteArgs`) and introduced `FsCwdArgs { repo_id }`; updated the `RpcServer` trait, in-process runtime, HTTP client, axum routes, and Tauri desktop commands together
- added `InProcessRpc::fs_root_for_repo(repo_id)` that resolves via `attached_workspaces.fs_root_canonical` and `HostFs::root_is_registered`, returning `NotFound` for unknown or detached `repo_id`
- added eight scoped helpers on `HostFs` (`read_dir_in`/`read_file_in`/`write_file_in`/`stat_in`/`create_file_in`/`create_dir_in`/`rename_in`/`delete_in`) plus the private `resolve_in(jail, path)` and the public `root_is_registered` defence-in-depth check; refactored `read_dir`/`stat` to share new module-level `read_dir_at`/`stat_at` helpers
- regenerated `crates/codeless-rpc/tests/wire-rpc.ts.snap` and `ui/codeless-ui/src/lib/rpc/generated/wire.ts`; registered the previously-unregistered `FsCreate*`/`FsMove`/`FsDelete`/`FsCwdArgs` types in both `examples/wire_ts.rs` and the snapshot test
- rewrote `crates/codeless-runtime/tests/fs.rs` to attach a workspace per-test; added regressions for unknown repo, jail-per-repo (cross-tab leakage), `fs_cwd`, the helper, and detached-repo refusal; updated the client and server HTTP round-trip tests in the same shape
- `cargo test --workspace --no-fail-fast` (71 suites green), `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --check` all clean
- committed as `cf00a1c` on `codeless/workspace-scoping` and pushed

## Next

- stage 5 (REVIEW) — lock the server-side scoping shape (`EventFilter` variants from stage 3 plus the `fs.*` `repo_id` threading + `FsCwdArgs` from this stage) before any UI work; if the gate passes, stage 6 starts the UI plumbing

## What you need to know

- the wire is now strict: `fs_cwd` requires `{ repo_id }` (no more unit body) and every `fs.*` RPC's JSON body must carry `repo_id` — UI callers will fail at runtime until stage 6 threads `activeRepoId` through
- `ui/codeless-ui/src/lib/rpc/methods.ts` still hand-rolls `Fs*Args` definitions without `repo_id`; stage 6 needs to either drop those for the generated `wire.ts` shapes or thread `repo_id` through the manual copies — the duplicate is a known drift hazard called out in `handover.md`
- `HostFs::resolve()` / `read_dir()` / `read_file()` / etc. (the "any registered root" path) are intentionally kept; `agent_chat` and the worktree manager still use them, this stage only flipped the RPC surface
- `fs_root_for_repo` is `pub` on `InProcessRpc` and returns the canonical `PathBuf`; future RPCs that need a per-workspace jail (worktree managers, attachment uploads, per-repo scratch dirs) should reach for it rather than re-deriving the lookup

## Open questions

- (none)
