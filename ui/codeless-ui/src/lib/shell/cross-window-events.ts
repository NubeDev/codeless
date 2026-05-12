// Lightweight cross-window event bus. Today's only consumers are the
// settings preference mirror (`codeless://prefs-changed`), the API-key
// change broadcast (`codeless://ai-keys-changed`), and the settings-tab
// focus signal (`codeless:settings-tab`). All three exist because the
// desktop shell uses a separate Tauri webview for Settings — writes
// in one window need to reach subscribers in the other.
//
// Browser/mobile shells only ever have one window. The default in-
// process adapter routes events through a shared `EventTarget`, which
// is functionally identical from a same-window subscriber's
// perspective. The cross-window concept simply collapses to a same-
// process callback.

export interface CrossWindowEventsAdapter {
  emit(event: string, payload?: unknown): Promise<void>;
  listen<T = unknown>(
    event: string,
    cb: (payload: T) => void,
  ): Promise<() => void>;
}

// EventTarget allows multiple listeners per event name and is part of
// the standard browser API. CustomEvent.detail carries the payload.
const bus = new EventTarget();

export const inProcessCrossWindowEvents: CrossWindowEventsAdapter = {
  emit: async (event, payload) => {
    bus.dispatchEvent(new CustomEvent(event, { detail: payload }));
  },
  listen: async (event, cb) => {
    const handler = (e: Event) => {
      cb((e as CustomEvent).detail);
    };
    bus.addEventListener(event, handler);
    return () => bus.removeEventListener(event, handler);
  },
};

let activeBus: CrossWindowEventsAdapter = inProcessCrossWindowEvents;

export function registerCrossWindowEvents(
  adapter: CrossWindowEventsAdapter,
): void {
  activeBus = adapter;
}

export function getCrossWindowEvents(): CrossWindowEventsAdapter {
  return activeBus;
}
