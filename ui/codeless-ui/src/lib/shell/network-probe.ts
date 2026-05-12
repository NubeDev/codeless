// Probe a URL for reachability — used by Settings → Models to verify
// the user's LM Studio base URL responds before they save it.
//
// Browser shells use `fetch`, but CORS makes any cross-origin response
// status invisible — the fetch either succeeds or throws "TypeError:
// Failed to fetch" with no body or status. We surface that as
// "reachable" (200) on success and 0 on failure, since the UI only
// distinguishes "reachable vs not". The Tauri shell uses an
// `http_ping` command that bypasses CORS via the host HTTP client and
// returns the real status code.

export interface NetworkProbeAdapter {
  /** Returns the HTTP status, or 0 when the host is unreachable. */
  ping(url: string): Promise<number>;
}

export const browserNetworkProbe: NetworkProbeAdapter = {
  ping: async (url) => {
    try {
      await fetch(url, {
        method: "GET",
        mode: "no-cors",
        cache: "no-store",
        signal: AbortSignal.timeout(2000),
      });
      // `no-cors` fetches return an opaque response — we can't read
      // the status. Reaching this line means the request didn't throw,
      // which is the best signal of reachability the browser gives us.
      return 200;
    } catch {
      return 0;
    }
  },
};
