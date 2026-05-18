## Done

- Added `attach_workspace` / `detach_workspace` / `list_workspaces` / `validate_workspace_path` to `RpcMethodMap` in `ui/codeless-ui/src/lib/rpc/methods.ts`, with typed args/result re-exports from `./wire`.
- Registered the workspace types (`AttachedWorkspace`, `AttachWorkspaceArgs/Result`, `DetachPolicy`, `DetachWorkspaceArgs`, `ListWorkspacesResult`, `ValidateWorkspacePathArgs/Result`, `WorkspaceProblem`, `WorkspaceError`) in `crates/codeless-rpc/examples/wire_ts.rs` so future codegen emits them; mirrored those types into `ui/codeless-ui/src/lib/rpc/generated/wire.ts` so the UI build sees them now without a Rust regen step.
- Added `ui/codeless-ui/src/lib/rpc/workspace-attach.test.ts` — the M3 exit test pinning the four method names and arg/result shapes via `expectTypeOf` + a `Pick<RpcMethodMap, …>` compile-time guard. Vitest: 5/5 pass; `tsc --noEmit` clean.
- Committed as `7469478` on `codeless/workspace-attach-ui`.

## Next

- Stage 2: shell-injected `PathPicker` capability and the browser + Tauri injectors (per DOCS/WORKSPACE-ATTACH.md §"UX — picking a path"); `TauriIpcClient` already inherits the new methods via the same generic `call<M>` boundary, but a parallel snapshot/pickup may be wanted.

## What you need to know

- `HttpSseClient` / `TauriIpcClient` / `MockClient` all dispatch through the generic `call<M extends RpcMethod>` shape, so adding `RpcMethodMap` entries was sufficient — no new transport code was needed. `MockClient.call`'s `switch` falls through to `RpcError("internal", "mock: unhandled method ...")` for the four new methods; later stages should add mock cases when components start consuming them.
- `cargo` can't be run in this worktree (the sibling `ai-ui` path-dep at `/home/user/.codeless/ai-ui` isn't checked out), so I patched `generated/wire.ts` by hand using the existing `crates/codeless-types/tests/wire.ts.snap` as the source of truth. The next agent who *can* run `cargo run -p codeless-rpc --example wire_ts` will re-emit the file with these types in their alphabetical position; the registration in `wire_ts.rs` is the durable change.
- `pnpm install` ran in `ui/codeless-ui` to make vitest work; the install was local-only and isn't committed (`node_modules` already gitignored).
- The Rust source of truth (`crates/codeless-types/src/workspace.rs`, `crates/codeless-rpc/src/server.rs`) already exposes the four methods on the `RpcServer` trait — this stage only landed the TS pickup.

## Open questions

- (none)
