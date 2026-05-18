/**
 * @codeless/ui-core — host UI primitives shared with plugin authors.
 *
 * Today this is a minimal kernel covering the per-surface undo/redo
 * command stack. The host shell and the plugin SDK both depend on
 * this package so plugin surfaces can wrap themselves in a
 * `<CommandScope>` and get their own stack without reaching into
 * host internals.
 *
 * Surface grows as the plugin SDK lands more contribution sites; this
 * file is the single re-export point so plugin authors never see the
 * subdirectory layout.
 */
export {
  CommandScope,
  useCommandStack,
  useCommandStackStore,
  useCommandScopeId,
  useGraphHistory,
  useGraphHistoryStore,
  createGraphHistoryStore,
} from "./graph-history";
export type {
  CommandScopeProps,
  GraphHistoryEntry,
  GraphHistoryState,
  GraphHistoryStore,
  UseGraphHistoryResult,
} from "./graph-history";
