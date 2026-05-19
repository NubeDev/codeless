import { create } from "zustand";

// Per-tab persistence for the last-used thread id. The "focused
// thread" is a projection of which thread *this tab* is currently
// viewing — two browser tabs against the same Codeless server view
// two different workspaces and so own independent focus pointers.
// `sessionStorage` keeps the value across reloads of the same tab
// (so a `?workspace=…` deep-link refresh rehydrates the right
// thread) while keeping the value isolated from any other tab on
// the same origin. The server-side "pinned thread" row is a future
// revision; if the stored id no longer resolves on the server
// (deleted thread), the footer transparently clears it on the next
// refresh.
//
// Key version bumped from `codeless.assistant.currentThreadId` (old
// localStorage) to `.v2` so an upgrade does not silently re-import
// the cross-tab-leaking value the old key holds.
const STORAGE_KEY = "codeless.assistant.currentThreadId.v2";

function readStored(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.sessionStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeStored(value: string | null): void {
  if (typeof window === "undefined") return;
  try {
    if (value === null) window.sessionStorage.removeItem(STORAGE_KEY);
    else window.sessionStorage.setItem(STORAGE_KEY, value);
  } catch {
    // sessionStorage can throw on quota / disabled storage; the
    // footer keeps working with in-memory state, just without
    // refresh-persistence.
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
