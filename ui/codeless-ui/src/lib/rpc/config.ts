// Where the browser shell finds its codeless-server. Vite reads
// `VITE_*` env vars at build time; the runtime overrides come from
// `localStorage` so users can point a built bundle at a different host
// without a rebuild. Order: localStorage → env → window.origin.

const LS_BASE_URL = "codeless-rpc-base-url";
const LS_TOKEN = "codeless-rpc-token";

export function readBaseUrl(): string {
  return (
    safeLocalStorage(LS_BASE_URL) ??
    import.meta.env.VITE_CODELESS_BASE_URL ??
    window.location.origin
  );
}

export function readToken(): string | null {
  return safeLocalStorage(LS_TOKEN) ?? import.meta.env.VITE_CODELESS_TOKEN ?? null;
}

function safeLocalStorage(key: string): string | null {
  try {
    const v = window.localStorage.getItem(key);
    return v && v.length > 0 ? v : null;
  } catch {
    return null;
  }
}
