// Persistent key/value storage for app state that survives reloads:
// user preferences, AI session history, custom agents, snippets, todos.
// Each consumer asks for a *named* store, so writes to one don't
// collide with another's keyspace.
//
// Backends:
//   - desktop / Tauri → `@tauri-apps/plugin-store` (`LazyStore`, one
//                       file per name under the OS app-data directory)
//   - browser / mobile → `localStorage` with `codeless-<name>:` prefix
//
// The factory is a module-level singleton — there's only ever one
// persistence backend per shell, and non-React call sites (the
// `settings/store.ts` setters, AI lib loaders) read from it lazily at
// use time. The shell entry registers a factory before mounting React.

export interface KVStoreAdapter {
  /** Single-key read; undefined when the key is absent. */
  get<T>(key: string): Promise<T | undefined>;
  /** Persist a single key/value pair. Resolves after durability. */
  set(key: string, value: unknown): Promise<void>;
  /** Remove a single key. */
  delete(key: string): Promise<void>;
  /** Read every entry in one round-trip. */
  loadAll(): Promise<Array<[string, unknown]>>;
  /** Subscribe to writes from any origin (own process + cross-window
   *  on desktop). The returned unlisten releases the subscription. */
  onChange(
    cb: (key: string, value: unknown) => void,
  ): Promise<() => void>;
}

export interface KVStoreFactory {
  open(name: string): KVStoreAdapter;
}

function makeLocalStorageStore(name: string): KVStoreAdapter {
  const prefix = `codeless-${name}:`;
  return {
    get: async <T>(key: string): Promise<T | undefined> => {
      const raw = localStorage.getItem(prefix + key);
      if (raw === null) return undefined;
      try {
        return JSON.parse(raw) as T;
      } catch {
        return undefined;
      }
    },
    set: async (key, value) => {
      localStorage.setItem(prefix + key, JSON.stringify(value));
    },
    delete: async (key) => {
      localStorage.removeItem(prefix + key);
    },
    loadAll: async () => {
      const out: Array<[string, unknown]> = [];
      for (let i = 0; i < localStorage.length; i++) {
        const k = localStorage.key(i);
        if (!k || !k.startsWith(prefix)) continue;
        const raw = localStorage.getItem(k);
        if (raw === null) continue;
        try {
          out.push([k.slice(prefix.length), JSON.parse(raw)]);
        } catch {
          // Skip malformed entries rather than crashing the whole load.
        }
      }
      return out;
    },
    onChange: async (cb) => {
      const handler = (e: StorageEvent) => {
        if (!e.key || !e.key.startsWith(prefix)) return;
        if (e.newValue === null) return;
        try {
          cb(e.key.slice(prefix.length), JSON.parse(e.newValue));
        } catch {
          // ignore
        }
      };
      window.addEventListener("storage", handler);
      return () => window.removeEventListener("storage", handler);
    },
  };
}

// Default factory caches per-name instances so callers invoking
// `getStore("prefs")` twice receive the same adapter (and therefore
// the same subscriber list). Tauri's LazyStore is similarly cached on
// the host side.
export const localStorageKVFactory: KVStoreFactory = (() => {
  const cache = new Map<string, KVStoreAdapter>();
  return {
    open: (name) => {
      let s = cache.get(name);
      if (!s) {
        s = makeLocalStorageStore(name);
        cache.set(name, s);
      }
      return s;
    },
  };
})();

let activeFactory: KVStoreFactory = localStorageKVFactory;

/** Shell entries call this once before mounting React. */
export function registerKVStoreFactory(factory: KVStoreFactory): void {
  activeFactory = factory;
}

export function getStore(name: string): KVStoreAdapter {
  return activeFactory.open(name);
}
