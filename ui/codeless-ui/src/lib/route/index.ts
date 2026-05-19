// Minimal path-based route store. Reads `location.pathname` and the
// query string on mount, listens for `popstate`, exposes
// `useRoute()` / `navigate()` for components that want to deep-link
// into a particular view.
//
// We deliberately do not pull in `react-router`: the app's routes are
// shallow (jobs/:id, file/:path, terminal/:id), Zustand already
// covers the state-fan-out side, and a hand-rolled hook keeps the
// addition under 80 lines with no peer-dependency surface.
//
// Vite's dev server falls back to `index.html` for unknown paths by
// default, so deep-links survive reload without server config. A
// production deployment serving the static bundle needs the same
// catch-all (the `codeless-server` will gain a `/*` -> index.html
// route once it serves the bundle directly).

import { useEffect, useSyncExternalStore } from "react";

export type Route = {
  pathname: string;
  search: string;
};

type Listener = () => void;

const listeners = new Set<Listener>();

function snapshot(): Route {
  if (typeof window === "undefined") {
    return { pathname: "/", search: "" };
  }
  return {
    pathname: window.location.pathname || "/",
    search: window.location.search || "",
  };
}

// Cache the snapshot so `useSyncExternalStore` sees a stable
// reference between identical reads — without it React's tearing
// guard fires on every render even when the URL has not changed.
let lastSnap = snapshot();
function getSnapshot(): Route {
  const next = snapshot();
  if (
    next.pathname !== lastSnap.pathname ||
    next.search !== lastSnap.search
  ) {
    lastSnap = next;
  }
  return lastSnap;
}

function subscribe(cb: Listener): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

function notify() {
  // Force the snapshot to refresh before we wake subscribers.
  lastSnap = snapshot();
  for (const cb of listeners) cb();
}

if (typeof window !== "undefined") {
  window.addEventListener("popstate", notify);
}

export function useRoute(): Route {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

// Query params the router preserves automatically across navigation
// when the caller does not supply them in `path`. The workspace
// deep-link (BROWSER-LAUNCHER.md §"Deep-link is router-managed") is
// the canonical example: tab changes via `navigate('/jobs/123')`
// must not strip `?workspace=<repo_id>` from the URL, otherwise
// browser-back lands on the wrong workspace after a tab switch.
const PRESERVED_QUERY_PARAMS = ["workspace"] as const;

function mergePreservedParams(path: string): string {
  if (typeof window === "undefined") return path;
  const [pathPart, queryPart = ""] = path.split("?", 2);
  const target = new URLSearchParams(queryPart);
  const current = new URLSearchParams(window.location.search);
  let changed = false;
  for (const key of PRESERVED_QUERY_PARAMS) {
    if (target.has(key)) continue;
    const v = current.get(key);
    if (v) {
      target.set(key, v);
      changed = true;
    }
  }
  if (!changed && !queryPart) return path;
  const search = target.toString();
  return search ? `${pathPart}?${search}` : pathPart;
}

/// Push a new path into history and tell subscribers. No-op when the
/// target equals the current pathname so repeated calls do not flood
/// the history stack.
export function navigate(path: string, opts: { replace?: boolean } = {}): void {
  if (typeof window === "undefined") return;
  const merged = mergePreservedParams(path);
  if (merged === window.location.pathname + window.location.search) return;
  if (opts.replace) {
    window.history.replaceState(null, "", merged);
  } else {
    window.history.pushState(null, "", merged);
  }
  notify();
}

/// React hook variant that re-runs `effect` whenever the pathname
/// matches `pattern`. Used by the tab system to focus the jobs tab
/// on `/jobs*` without re-rendering the dashboard.
export function useRouteEffect(
  pattern: RegExp,
  effect: (match: RegExpExecArray) => void,
): void {
  const { pathname } = useRoute();
  useEffect(() => {
    const m = pattern.exec(pathname);
    if (m) effect(m);
    // The regex is the closure that defines whether we ran; the
    // effect closure captures the latest callback by virtue of
    // being recreated on every render. Intentionally exclude
    // `effect` and `pattern` from deps so a stable closure does
    // not refire on every parent re-render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pathname]);
}
