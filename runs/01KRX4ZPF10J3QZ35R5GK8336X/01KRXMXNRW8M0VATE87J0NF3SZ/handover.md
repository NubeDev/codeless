## Done

- Implemented stage 9 of plugin-substrate-runtimes: full `@codeless/plugin-ui-sdk` MF surface — `slots.ts` (v0.1 vocabulary + parse), `mf.ts` (MfRuntime install seam + `pluginManifestUrl`), `registration.ts` (slot-contribution registry with subscribe/notify), `components/PluginSlot.tsx` (lazy mount with per-contributor error boundary), `rsbuild-shared.ts` (singleton version pins), `eslint-config.ts` (R6 flat-config).
- Added test files for each module (slots/mf/registration/eslint-config/PluginSlot): 56 vitest tests pass under jsdom.
- Updated `package.json` exports (`./slots`, `./registration`, `./rsbuild-shared`, `./eslint-config`), `tsconfig.json` include list, `vitest.config.ts` include list, and reworded `src/index.ts` stage banner.
- Host `ui/codeless-ui` `tsc --noEmit` still green; no Rust changes.
- Committed as `f6a0784` on `codeless/plugin-substrate-runtimes` with the title `stage 9: implement @codeless/plugin-ui-sdk MF surface`. Not pushed.

## Next

- Stage 10 of 19 picks up next per the job plan; this session stopped at the stage boundary per instructions.

## What you need to know

- The SDK is dependency-light: `MfRuntime` is an injection seam, so neither `@module-federation/enhanced` nor `@codeless/rpc` are imported here. The host shell is expected to call `setMfRuntime(...)` once at boot before any `<PluginSlot/>` mounts; tests use a hand-rolled fake.
- `PluginSlot` always forwards the parameterised slot id's arg as a `slotArg` prop (null for non-parameterised slots). Per-contributor `<PluginErrorBoundary>` + `<Suspense>` means a crashing or slow plugin is a local failure.
- `rsbuild-shared.ts` lists `@codeless/rpc` and `@codeless/ui-core` at `^0.1` even though `@codeless/rpc` is not a real package yet — the doc declares this as the seam; whoever lands the rpc package needs to keep that pin in sync.
- The R6 ESLint config is built on stock ESLint rules (`no-restricted-imports`, `no-restricted-syntax`, `no-restricted-globals`), so plugin authors only need eslint itself in devDeps; the SDK does not ship a custom rule plugin. The full lint-fixture integration test (`plugin_ui_e2e::r6_eslint_rejects_forbidden_imports`) is still owed by a later stage.
- Two test-only resets are exported (`resetMfRuntimeForTesting`, `resetRegistryForTesting`, `resetPluginSlotCacheForTesting`) so the next stage's host-shell integration tests can wipe global state between cases.
- Commit was made with plain `git`, not `mani`, because this is a one-shot agent SDK job (not a JOB-LOOP run); the CLAUDE.md mani rule is gated on a running loop.

## Open questions

- The pre-existing rubix-ported files (`components/BlockShell.tsx`, `components/SlotBadge.tsx`, `components/NodeLink.tsx`, the `hooks/use*.ts` tree) still reference `@codeless/ui-kit` / `@codeless/rpc` / local hooks that don't compile yet; they are deliberately not in the tsconfig include list. Confirm whether a future stage wants them moved out of `src/` to make their absence from the published surface unambiguous, or left in place per the original port plan.
