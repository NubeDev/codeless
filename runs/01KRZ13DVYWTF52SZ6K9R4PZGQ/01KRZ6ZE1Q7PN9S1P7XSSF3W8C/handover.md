## Done

- verified stage 7 is committed at 92292e3 on branch codeless/workspace-scoping with message starting "stage 7: read ?workspace=<repo_id>..."
- changes confined to ui/codeless-ui: src/lib/route/index.ts (navigate() preserves `workspace` query param) + new src/lib/route/index.test.ts, src/modules/workspaces/store.ts (seeds activeRepoId from `?workspace=` via readWorkspaceParamFromUrl; subscribes to store and mirrors via history.replaceState through writeWorkspaceParamToUrl) + extended src/modules/workspaces/store.test.ts
- ran the two affected vitest files: 2 files / 19 tests pass

## Next

- (none) — fresh session will pick up stage 8

## What you need to know

- working tree is clean; no new edits this session, only verification
- writeWorkspaceParamToUrl uses url.pathname+search+hash (not url.toString()) to stay safe under the Tauri custom-scheme webview, which preserves the deep-link behaviour on the existing Tauri path
- the store subscribe block is module-level and guarded by `typeof window !== "undefined"`; URL writes are no-op when the param already matches, so popstate-driven state writes cannot loop
- navigate() merges a PRESERVED_QUERY_PARAMS list (currently just `workspace`) so tab-changes via navigate('/jobs/123') do not strip the deep-link
- I did not run the full workspace `cargo test`/`clippy`/`fmt`; this stage only touches TypeScript under ui/codeless-ui

## Open questions

- (none)
