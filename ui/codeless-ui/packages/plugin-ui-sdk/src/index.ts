// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/index.ts@a7fecef1c641cc8800aa2162f108131c6b426451
/**
 * @codeless/plugin-ui-sdk
 *
 * Stable, versioned SDK for codeless plugin UI authors. Plugin authors
 * should import exclusively from this package — never reach into
 * `@codeless/ui-core` directly.
 *
 * Surface at stage 9:
 *
 *   - the `editable-collection` adapter (undo/redo/duplicate/copy/
 *     paste) and the command-stack primitives re-exported from
 *     `@codeless/ui-core`;
 *   - the Module Federation runtime seam (`mf`) + slot vocabulary;
 *   - the host-side slot-contribution registry (`registration`);
 *   - the `<PluginSlot/>` mount component;
 *   - the `rsbuild-shared` singleton-version source of truth;
 *   - the R6 ESLint flat-config (`eslint-config`).
 *
 * The shadcn primitive shells and the agent-client hooks that the
 * rubix port still ships (`components/BlockShell.tsx`,
 * `hooks/useAgentClient.ts`, …) remain alongside the source tree
 * with their `codeless-ported-from` headers; they light up once
 * `@codeless/rpc` and the host `@codeless/ui-kit` packages land.
 */

// ── Editable collections — undo/redo, duplicate, copy/paste ──────────────
export { useEditableCollection } from "./editable-collection";
export {
  clearItemClipboard,
  getItemClipboard,
} from "./editable-collection";
export type {
  EditableCollectionApi,
  EditableCollectionOptions,
  ItemDraft,
  CollectionMenuItem,
  PasteResult,
  PasteWarning,
} from "./editable-collection";

// ── Per-surface command-stack primitives (re-exported from ui-core) ──────
export {
  CommandScope,
  useCommandStack,
  useCommandStackStore,
  useCommandScopeId,
  useGraphHistory,
  useGraphHistoryStore,
  createGraphHistoryStore,
} from "@codeless/ui-core";
export type {
  CommandScopeProps,
  GraphHistoryEntry,
  GraphHistoryState,
  GraphHistoryStore,
  UseGraphHistoryResult,
} from "@codeless/ui-core";

// ── Slot vocabulary (DOCS/plugins/PLUGIN-UI-FEDERATION.md § Slots) ───────
export {
  SLOT_VOCABULARY,
  SLOT_NAMES,
  parseSlotId,
  isKnownSlot,
} from "./slots";
export type {
  SlotName,
  SlotShape,
  SlotCardinality,
  ParsedSlotId,
} from "./slots";

// ── Module Federation runtime seam ───────────────────────────────────────
export {
  setMfRuntime,
  getMfRuntime,
  resetMfRuntimeForTesting,
  pluginManifestUrl,
} from "./mf";
export type { MfRuntime } from "./mf";

// ── Slot-contribution registry (host-facing) ─────────────────────────────
export {
  registerPluginContributions,
  unregisterPluginContributions,
  getSlotContributors,
  subscribeToRegistry,
  resetRegistryForTesting,
} from "./registration";
export type {
  PluginContribution,
  PluginExpose,
  SlotContributor,
} from "./registration";

// ── The mount component the host renders at every slot site ──────────────
export {
  PluginSlot,
  resetPluginSlotCacheForTesting,
} from "./components/PluginSlot";
export type { PluginSlotProps } from "./components/PluginSlot";

// ── rsbuild shared-singleton pin (re-exported so the SDK is the one
//    import a plugin's rsbuild.config.ts needs for the shared map) ───────
export {
  sharedSingletons,
} from "./rsbuild-shared";
export type {
  MfSharedEntry,
  MfSharedSingletons,
} from "./rsbuild-shared";

// ── R6 ESLint flat-config (re-exported for `import` convenience) ─────────
export {
  codelessPluginEslintConfig,
} from "./eslint-config";
export type { CodelessFlatConfigEntry } from "./eslint-config";
