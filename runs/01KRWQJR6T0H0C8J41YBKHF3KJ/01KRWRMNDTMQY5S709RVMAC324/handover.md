## Done

- reviewed M3a/b/c diff (5a02de6..a1689f2..af2bf41) against Layer-1 invariants
- confirmed wire types in `ui/codeless-ui/src/lib/rpc/generated/wire.ts` match `crates/codeless-types/tests/wire.ts.snap` (AttachWorkspaceArgs/Result, AttachedWorkspace, DetachPolicy/DetachWorkspaceArgs, ListWorkspacesResult, ValidateWorkspacePath{Args,Result}, WorkspaceProblem, WorkspaceError)
- confirmed both `HttpSseClient` and `TauriIpcClient` reach the four methods via the pre-existing generic `call()` plumbing (no second transport, no bespoke per-method code paths)
- confirmed `PathPicker` capability + injectors keep the trust boundary at `validate_workspace_path` on the server
- PASS: M3 holds R1 (no host-only deps leak into mobile-safe crates, no new `process::Command` users), single-transport invariant (no parallel IPC path), R4/R5 (picker explicitly untrusted; server validates), and wire formats are byte-identical to the codeless-types specta snapshot

## Next

- next session picks up Stage 5 (M4 WORK — Settings → Workspaces tab UI consuming the new RPC methods and PathPicker)

## What you need to know

- `ui/codeless-ui/src/lib/rpc/generated/wire.ts` had the workspace-attach block hand-appended with a comment that the next `cargo run -p codeless-rpc --example wire_ts` regen will rewrite it in alphabetical position — shapes already match the snap so the regen is a no-op semantically
- `TauriIpcClient` needs no per-method code: the generic `call()` already maps `<snake>` → `invoke("rpc_<snake>", { args })`; tests in `tauri-ipc-client.workspace-attach.test.ts` pin that contract
- `browserPathPicker` falls back to `window.prompt` (no File System Access API path → server path mapping is possible from a sandboxed handle, by design)

## Open questions

- (none)
