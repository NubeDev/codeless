import { create } from "zustand";

// Persistence key for the last-used thread id. localStorage is enough
// for the single-tenant trust boundary (R5): the bearer-token client
// already knows it is the sole user; the server-side "pinned thread"
// row is a future revision. If the stored id no longer resolves on
// the server (deleted thread), the footer transparently clears it on
// the next refresh.
const STORAGE_KEY = "codeless.assistant.currentThreadId";

function readStored(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeStored(value: string | null): void {
  if (typeof window === "undefined") return;
  try {
    if (value === null) window.localStorage.removeItem(STORAGE_KEY);
    else window.localStorage.setItem(STORAGE_KEY, value);
  } catch {
    // localStorage can throw on quota / disabled storage; the footer
    // keeps working with in-memory state, just without persistence.
  }
}

// Shared "what thread is the assistant currently bound to" state. The
// footer composer writes here when the user submits or creates a new
// thread; the `/assistant` page writes here when the rail's selection
// changes; both surfaces read it so the footer and the rail render the
// same thread without one owning the other (R4: SQLite is the source
// of truth — this store only holds the *pointer* into SQLite).
//
// `refreshTick` is a monotonically increasing counter the footer bumps
// after a successful append. Subscribers (`AssistantPage`,
// `AssistantThreadView`) read it as a dependency in their effects and
// re-fetch via `list_assistant_threads` / `list_assistant_messages`.
// We do this in lieu of a per-thread subscription channel — adding one
// is a runtime-side change (open question §2 in the session doc) and
// not in scope for F1.
type State = {
  currentThreadId: string | null;
  refreshTick: number;
  setCurrentThreadId: (id: string | null) => void;
  bumpRefresh: () => void;
};

export const useAssistantFocus = create<State>((set) => ({
  currentThreadId: readStored(),
  refreshTick: 0,
  setCurrentThreadId: (id) => {
    writeStored(id);
    set({ currentThreadId: id });
  },
  bumpRefresh: () => set((s) => ({ refreshTick: s.refreshTick + 1 })),
}));
