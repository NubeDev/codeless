// codeless-ported-from: rubix-workspace/rubix-ui-core/src/lib/graph-history/store.ts@59b23f40c39f676509e008ffa1ce3e1d90290603
/**
 * Client-side undo/redo stack for graph mutations.
 *
 * Entries are opaque — each supplies its own `undo` / `redo` promises.
 * That keeps the store domain-agnostic: anything reducible to "do then
 * inverse" can live here.
 *
 * A single global instance backs the host canvas. Block UI surfaces
 * (Gantt boards, dashboards, list views) create their own via
 * `createGraphHistoryStore()` and provide them through `<CommandScope>`
 * so a Ctrl+Z inside a block does not pop a host-canvas entry.
 */
import { create, type StoreApi, type UseBoundStore } from "zustand";

export interface GraphHistoryEntry {
  label: string;
  ts: number;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
}

export interface GraphHistoryState {
  undoStack: GraphHistoryEntry[];
  redoStack: GraphHistoryEntry[];
  pending: boolean;
  record(entry: Omit<GraphHistoryEntry, "ts"> & { ts?: number }): void;
  undo(): Promise<void>;
  redo(): Promise<void>;
  clear(): void;
}

const MAX_STACK = 100;

export function createGraphHistoryStore(): UseBoundStore<
  StoreApi<GraphHistoryState>
> {
  return create<GraphHistoryState>((set, get) => ({
    undoStack: [],
    redoStack: [],
    pending: false,

    record(entry) {
      const stamped: GraphHistoryEntry = { ...entry, ts: entry.ts ?? Date.now() };
      set((s) => ({
        undoStack: [...s.undoStack, stamped].slice(-MAX_STACK),
        redoStack: [],
      }));
    },

    async undo() {
      const s = get();
      if (s.pending) return;
      const entry = s.undoStack[s.undoStack.length - 1];
      if (!entry) return;
      set({ pending: true });
      try {
        await entry.undo();
        set((cur) => ({
          undoStack: cur.undoStack.slice(0, -1),
          redoStack: [...cur.redoStack, entry].slice(-MAX_STACK),
        }));
      } finally {
        set({ pending: false });
      }
    },

    async redo() {
      const s = get();
      if (s.pending) return;
      const entry = s.redoStack[s.redoStack.length - 1];
      if (!entry) return;
      set({ pending: true });
      try {
        await entry.redo();
        set((cur) => ({
          redoStack: cur.redoStack.slice(0, -1),
          undoStack: [...cur.undoStack, entry].slice(-MAX_STACK),
        }));
      } finally {
        set({ pending: false });
      }
    },

    clear() {
      set({ undoStack: [], redoStack: [], pending: false });
    },
  }));
}

export const useGraphHistoryStore = createGraphHistoryStore();

export type GraphHistoryStore = UseBoundStore<StoreApi<GraphHistoryState>>;
