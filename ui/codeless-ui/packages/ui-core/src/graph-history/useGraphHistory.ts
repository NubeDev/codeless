// codeless-ported-from: rubix-workspace/rubix-ui-core/src/lib/graph-history/useGraphHistory.ts@59b23f40c39f676509e008ffa1ce3e1d90290603
/**
 * React binding for graph-history stores — exposes derived
 * `canUndo` / `canRedo` booleans alongside callable actions so
 * toolbar buttons can wire straight to the return value.
 *
 * `useGraphHistory(store)` reads from any store created with
 * `createGraphHistoryStore()`; the no-arg form uses the global one.
 */
import {
  useGraphHistoryStore,
  type GraphHistoryStore,
} from "./store";

export interface UseGraphHistoryResult {
  canUndo: boolean;
  canRedo: boolean;
  pending: boolean;
  undo: () => void;
  redo: () => void;
  clear: () => void;
}

export function useGraphHistory(
  store: GraphHistoryStore = useGraphHistoryStore,
): UseGraphHistoryResult {
  const undoLen = store((s) => s.undoStack.length);
  const redoLen = store((s) => s.redoStack.length);
  const pending = store((s) => s.pending);
  const undo = store((s) => s.undo);
  const redo = store((s) => s.redo);
  const clear = store((s) => s.clear);

  return {
    canUndo: undoLen > 0 && !pending,
    canRedo: redoLen > 0 && !pending,
    pending,
    undo: () => void undo(),
    redo: () => void redo(),
    clear,
  };
}
