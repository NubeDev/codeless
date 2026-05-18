## Done

- Added `SettingsTab` value `"workspaces"` and registered the Workspaces tab in `SettingsApp` with the `Folder01Icon`.
- New `settings/sections/WorkspacesSection.tsx` renders the attached-workspaces table (active dot, Open, Detach) plus the `+ Attach` button; reuses `useWorkspacesSync` for hydration and shares the empty state with `EmptyWorkspacesState`.
- New `modules/workspaces/AttachWorkspaceDialog.tsx`: path input + shell-injected `PathPicker` browse, 250 ms debounced `validate_workspace_path` (keeping under the server's ~5/s cap), inline indicators + problems, name auto-derive, runner select fed by `ServerInfo.available_cli_runners`, and the confirm round-trip (`list_repos` to dedup → `add_repo` if new → `attach_workspace`, then `applyAttached` to the store).
- New `modules/workspaces/DetachWorkspaceDialog.tsx`: first attempt uses `DetachPolicy::Refuse`; if the server returns `running-jobs`, the modal expands to the Stop / Leave-running radio per §"Detach modal".
- New `settings/sections/WorkspacesSection.test.tsx`: 5 happy-path tests cover empty state, modal-from-CTA, full attach round-trip, PathPicker injection, and per-row detach. All tests + `tsc --noEmit` are green.
- Committed: `5e9a4c6 stage 6: M4b: …`.

## Next

- Stage 7 (M5 / job-page integration per the doc) — filter the jobs view by active workspace and add a "switch workspace" affordance when the user clicks a job from a different workspace; the per-tab store from M4 already exposes `activeRepoId`.

## What you need to know

- `add_repo` requires `clone_url` + `git_auth`; for local attaches we send `clone_url: ""` and `git_auth: { kind: "ssh", key_path: "" }`. The `MockRpcClient` accepts that as-is; a real server may need tightening (likely a future scope question).
- The runtime does not emit `workspace-attached` / `workspace-detached` events yet (noted in `useWorkspacesSync`); both dialogs reconcile the store directly after their RPCs resolve and rely on `applyAttached` / `applyDetached` being idempotent so a future live event is a no-op.
- `DetachWorkspaceDialog.parseRunningJobs` sniffs the `RpcError` message for the `running-jobs` tag (kebab-case from specta). When the runtime starts emitting a structured payload, swap this for a typed branch on `WorkspaceError`.
- The `/workspaces` top-level route + sidebar group are still deferred per the doc's M4 phasing — only the Settings tab landed in this stage.

## Open questions

- (none)
