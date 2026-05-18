// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/index.ts@a7fecef1c641cc8800aa2162f108131c6b426451
/**
 * @codeless/plugin-ui-sdk
 *
 * Stable, versioned SDK for codeless plugin UI authors. Plugin authors
 * should import exclusively from this package — never reach into
 * `@codeless/ui-core` directly.
 *
 * This stage-8 surface ports the rubix `editable-collection` adapter
 * (undo/redo/duplicate/copy/paste) and the matching command-stack
 * primitives from `@codeless/ui-core`. The full hooks + components
 * surface (`useNode`, `useSlotWriter`, `BlockShell`, MF helpers, the
 * registration table, the R6 ESLint rule) is ported in stage 9 — the
 * source files exist in this package already (carrying their
 * `codeless-ported-from` headers) and stage 9 wires them into the
 * tsconfig once `@codeless/rpc` and the host `@codeless/ui-kit` land.
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
