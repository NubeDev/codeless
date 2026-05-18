## Done

- Created `ui/codeless-ui/packages/plugin-ui-sdk/` forked from `rubix-workspace/extension-ui-sdk@a7fecef` with renames `@rubix/agent-client`→`@codeless/rpc`, `@rubix/ui-core`→`@codeless/ui-core`, `@rubix/ui-kit`→`@codeless/ui-kit`. Every ported file carries `// codeless-ported-from: rubix-workspace/extension-ui-sdk/<path>@a7fecef…` header.
- Created `ui/codeless-ui/packages/ui-core/` with the graph-history primitives (`CommandScope`, `useCommandStack`, `useCommandStackStore`, `useGraphHistory`, `createGraphHistoryStore`) ported from `rubix-ui-core/src/lib/graph-history/*@59b23f4…` so the test bed has a working `@codeless/ui-core` import target.
- Enabled pnpm workspaces in `ui/codeless-ui/pnpm-workspace.yaml` (`. + packages/*`).
- Rubix-derived `editable-collection.test.tsx` passes 21/21 under codeless's vitest 4 + jsdom config; package typecheck clean; host `pnpm typecheck` clean; host `pnpm test` still 118/118.
- Committed as `stage 8: fork rubix extension-ui-sdk into ui/codeless-ui/packages/plugin-ui-sdk/` (42d3036) on `codeless/plugin-substrate-runtimes`.

## Next

- Stage 9: implement `@codeless/plugin-ui-sdk` proper — `mf.ts` (slot vocabulary per stage 1), `registration.ts`, `PluginSlot` React component, `rsbuild-shared.ts` (singleton version pin), `eslint-config.ts` enforcing R6. The remaining ported hooks (`useAction`, `useNode`, `useSlotWriter`, etc.) and components (`BlockShell`, `NodeLink`, `SlotBadge`) already exist with codeless-ported-from headers; stage 9 wires them into tsconfig once `@codeless/rpc` and `@codeless/ui-kit` are added.

## What you need to know

- The host `ui/codeless-ui` `package.json` is itself a member of the new workspace (the `.` entry in `pnpm-workspace.yaml`); no host changes required, host tests unaffected.
- `@codeless/ui-core` is a minimal kernel — only graph-history is ported. Stage 9/10 will need `useAgent`, `useNodes`, `useGraphStoreOptional`, `useGraphStoreNode`, `useGraphStoreSubscription`, `mf` helpers, plus `@codeless/ui-kit` (`Badge`) and the full `@codeless/rpc` typedefs before the rest of the ported SDK files can compile.
- `plugin-ui-sdk/tsconfig.json` intentionally lists only `editable-collection.{tsx,test.tsx}` + `index.ts` in `include`. The other ported files are present (for the codeless-ported-from header audit) but excluded from compilation until their cross-package deps land.
- `vitest.config.ts` and `ui-core/src/index.ts` are the only files in the new packages without a `codeless-ported-from` header — both are codeless-authored aggregation files (config + barrel re-export), not direct rubix ports.
- Rubix SHA used for all SDK files: `a7fecef1c641cc8800aa2162f108131c6b426451`. Rubix SHA for ui-core graph-history files: `59b23f40c39f676509e008ffa1ce3e1d90290603`.

## Open questions

- Stage 9 will introduce `@codeless/rpc` and `@codeless/ui-kit`; if those packages already exist outside the codeless-ui workspace in some other shape, the workspace wiring may need a `references:` entry. Today they don't exist anywhere in this repo, so stage 9 will likely create them as additional `packages/*` members.
- `@codeless/ui-core` here is intentionally minimal. If a future stage wants to converge it with the existing host `ui/codeless-ui/src/lib/rpc/` types, that's a structural decision (extract host lib into the package, or fork-and-diverge) worth surfacing to the job author.
