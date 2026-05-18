// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/mf.ts@a7fecef1c641cc8800aa2162f108131c6b426451
/**
 * Module Federation shared-singleton factory — re-exported from
 * `@codeless/ui-core/mf` so plugin authors depend only on
 * `@codeless/plugin-ui-sdk` (their SDK facade) and never reach into
 * `ui-core` directly.
 *
 * Stage 9 fills in the real implementation; this file is the
 * stage-8 fork header so the rename is auditable.
 *
 * Usage in a plugin's rsbuild.config.ts:
 *
 * ```ts
 * import { createSharedSingletons } from "@codeless/plugin-ui-sdk/mf";
 * new ModuleFederationPlugin({ ..., shared: createSharedSingletons() });
 * ```
 */
export { createSharedSingletons, type MfSharedSingletons } from "@codeless/ui-core/mf";
