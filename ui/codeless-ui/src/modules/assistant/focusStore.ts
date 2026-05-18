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
// Re-sort / re-fetch fan-out used to ride on a `refreshTick` counter
// every mutator bumped; that has been retired (`DOCS/SCOPE-ASSISTANT-PARITY.md`
// §W1c) in favour of the `assistant-thread-touched` event the runtime
// publishes on every `touch_assistant_thread`. Surfaces subscribe via
// `useEventStream` directly and refresh on the typed envelope.
type State = {
  currentThreadId: string | null;
  setCurrentThreadId: (id: string | null) => void;
};

export const useAssistantFocus = create<State>((set) => ({
  currentThreadId: readStored(),
  setCurrentThreadId: (id) => {
    writeStored(id);
    set({ currentThreadId: id });
  },
}));
