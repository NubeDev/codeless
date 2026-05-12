import { LazyStore } from "@tauri-apps/plugin-store";

import type { KVStoreAdapter, KVStoreFactory } from "@/lib/shell";

// One `LazyStore` per named bucket, mapped to a file under the OS
// app-data directory. File names mirror the upstream Terax conventions
// so existing user data round-trips: `codeless-<name>.json`. Each
// store is cached so callers asking for the same name receive the
// same instance and therefore the same `onChange` subscriber list.

function makeTauriStore(name: string): KVStoreAdapter {
  const store = new LazyStore(`codeless-${name}.json`, {
    defaults: {},
    autoSave: 200,
  });
  return {
    get: <T>(key: string) => store.get<T>(key).then((v) => v ?? undefined),
    set: async (key, value) => {
      await store.set(key, value);
      await store.save();
    },
    delete: async (key) => {
      await store.delete(key);
      await store.save();
    },
    loadAll: () => store.entries(),
    onChange: (cb) => store.onChange<unknown>(cb),
  };
}

export const tauriKVFactory: KVStoreFactory = (() => {
  const cache = new Map<string, KVStoreAdapter>();
  return {
    open: (name) => {
      let s = cache.get(name);
      if (!s) {
        s = makeTauriStore(name);
        cache.set(name, s);
      }
      return s;
    },
  };
})();
