// Where the browser shell finds its codeless-server. Vite reads
// `VITE_*` env vars at build time; the runtime overrides come from
// `localStorage` so users can point a built bundle at a different host
// without a rebuild.
//
// Resolution order for the base URL:
//   1. localStorage `codeless-rpc-base-url`
//   2. `VITE_CODELESS_BASE_URL` (build-time)
//   3. When the page is being served from a Vite dev port (1420 or
//      5173), default to `http://127.0.0.1:7777` — the conventional
//      `codeless serve --bind` default. This is the zero-paste path
//      for `codeless serve` + `pnpm dev`: the browser hits localhost
//      and the server is the same hostname on a different port.
//   4. Same-origin (`window.location.origin`). Used by production
//      builds served from the same host as the server.
//
// The token resolution intentionally has no default: on loopback the
// server defaults to `AuthMode::Open` and the absence of a token is
// the correct, working state. Non-loopback deployments must set the
// token via localStorage or env.

const LS_BASE_URL = "codeless-rpc-base-url";
const LS_TOKEN = "codeless-rpc-token";
const LOCAL_SERVER_DEFAULT = "http://127.0.0.1:7777";

export function readBaseUrl(): string {
  const stored = safeLocalStorage(LS_BASE_URL);
  if (stored) return stored;
  const env = import.meta.env.VITE_CODELESS_BASE_URL;
  if (env) return env;
  if (isViteDevPort(window.location.port)) return LOCAL_SERVER_DEFAULT;
  return window.location.origin;
}

export function readToken(): string | null {
  return safeLocalStorage(LS_TOKEN) ?? import.meta.env.VITE_CODELESS_TOKEN ?? null;
}

/// True when the page is served from a port the Vite dev server
/// listens on. Codeless inherits Terax's `1420` default; the demo
/// uses Vite's own default `5173` because pnpm dev does not pass an
/// explicit `--port`. Keep both so either origin Just Works.
export function isViteDevPort(port: string): boolean {
  return port === "1420" || port === "5173";
}

function safeLocalStorage(key: string): string | null {
  try {
    const v = window.localStorage.getItem(key);
    return v && v.length > 0 ? v : null;
  } catch {
    return null;
  }
}
