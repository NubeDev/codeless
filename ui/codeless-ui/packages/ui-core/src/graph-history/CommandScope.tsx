// codeless-ported-from: rubix-workspace/rubix-ui-core/src/lib/graph-history/CommandScope.tsx@59b23f40c39f676509e008ffa1ce3e1d90290603
/**
 * `<CommandScope>` — a per-surface undo/redo command stack.
 *
 * Wraps a plugin UI route, a property panel, or any other editable
 * subtree so descendants get their own stack instead of sharing the
 * global host store. Inside the subtree, `useCommandStack()` returns
 * the scoped store. Call sites outside any scope fall back to the
 * global store transparently.
 */
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  type ReactElement,
  type ReactNode,
} from "react";

import {
  createGraphHistoryStore,
  useGraphHistoryStore,
  type GraphHistoryStore,
} from "./store";
import { useGraphHistory, type UseGraphHistoryResult } from "./useGraphHistory";

interface CommandScopeContextValue {
  id: string;
  store: GraphHistoryStore;
}

const CommandScopeContext = createContext<CommandScopeContextValue | null>(null);

export interface CommandScopeProps {
  id: string;
  resetOnUnmount?: boolean;
  children: ReactNode;
}

export function CommandScope({
  id,
  resetOnUnmount = false,
  children,
}: CommandScopeProps): ReactElement {
  const storeRef = useRef<GraphHistoryStore | null>(null);
  if (storeRef.current === null) {
    storeRef.current = createGraphHistoryStore();
  }

  const value = useMemo<CommandScopeContextValue>(
    () => ({ id, store: storeRef.current as GraphHistoryStore }),
    [id],
  );

  useEffect(() => {
    if (!resetOnUnmount) return;
    const store = storeRef.current;
    return () => {
      store?.getState().clear();
    };
  }, [resetOnUnmount]);

  return (
    <CommandScopeContext.Provider value={value}>
      <div data-cmd-scope={id} style={{ display: "contents" }}>
        {children}
      </div>
    </CommandScopeContext.Provider>
  );
}

export function useCommandStackStore(): GraphHistoryStore {
  const ctx = useContext(CommandScopeContext);
  return ctx?.store ?? useGraphHistoryStore;
}

export function useCommandScopeId(): string | null {
  const ctx = useContext(CommandScopeContext);
  return ctx?.id ?? null;
}

export function useCommandStack(): UseGraphHistoryResult {
  const store = useCommandStackStore();
  return useGraphHistory(store);
}
