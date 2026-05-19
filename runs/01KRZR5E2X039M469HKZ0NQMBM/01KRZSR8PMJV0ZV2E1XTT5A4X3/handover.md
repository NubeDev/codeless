## Done

- added `export_job` / `import_job` / `inspect_job_bundle` typed entries and arg/result types to `ui/codeless-ui/src/lib/rpc/methods.ts`
- created `ui/codeless-ui/src/modules/workspaces/ImportJobDialog.tsx` with bundle-path entry, manifest preview, conflict-policy selector (Refuse/Suffix/Replace), optional rename, and local-warnings banner
- wired `Import Job…` button into `ui/codeless-ui/src/settings/sections/WorkspacesSection.tsx`; navigates to the new Job on success with warnings packed into the URL hash
- added `export` button + success info line to `PageHeader` in `ui/codeless-ui/src/modules/jobs/JobPage.tsx`
- added dismissible `ImportedWarningsBanner` to JobPage that reads `#imported-warnings=...`
- committed as `stage 9: …` on `codeless/job-export` (commit 2de4369)

## Next

- stage 10 (final) per the job's stage list

## What you need to know

- Server-side RPCs `export_job` / `import_job` / `inspect_job_bundle` are NOT yet implemented (stages 4–6 produced no Rust source per prior handover). The UI compiles and shows the controls, but live calls will surface `method not found` until the runtime side lands.
- Could not run `tsc` — no `node_modules` in the worktree. Changes were visually checked; testids preserved (`workspaces-attach-button`, etc.) so existing tests should still pass.
- Used raw `git commit` because no `mani.yaml` lives in this inner repo (consistent with previous halt commits on this branch).
- `WorkspaceId` from SCOPE-JOB-EXPORT is represented as `RepoId` in the UI types (this codebase uses RepoId for workspace identity).
- Import button currently lives in `Settings → Workspaces` (closest existing "workspaces sidebar" surface); a future top-level `/workspaces` route per `WORKSPACE-ATTACH.md` Milestone 4 should re-host it.

## Open questions

- (none)
