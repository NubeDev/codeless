import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "@xterm/xterm/css/xterm.css";
import "../../styles/globals.css";

import ReactDOM from "react-dom/client";

import App from "../../app/App";
import {
  HttpSseClient,
  MockRpcClient,
  RpcProvider,
  isViteDevPort,
  readBaseUrl,
  readToken,
  type RpcClient,
} from "../../lib/rpc";
import { ShellProvider, type ShellCapabilities } from "../../lib/shell";

// Browser does not own its window chrome — the host browser draws the
// frame. The no-op `WindowControlsAdapter` is supplied by default.
const capabilities: ShellCapabilities = {
  customWindowControls: false,
};

// Transport selection, in priority order:
//   - `?mock=1` → force MockRpcClient.
//   - Otherwise pick `HttpSseClient` and probe `/healthz` once. The
//     local server responds 200 `ok`; if the probe fails (server not
//     running, wrong host, network blocked) fall back to mock so the
//     dev experience stays usable. The probe is short (1s timeout) so
//     a missing server does not visibly delay first paint.
//
// The previous "default to mock on Vite dev ports" rule made the
// zero-paste demo (server up, `pnpm dev`, open browser) silently land
// on the mock client. The probe path lets the same one-line setup
// reach the real server when it's there.
async function buildClient(): Promise<RpcClient> {
  const params = new URLSearchParams(window.location.search);
  if (params.get("mock") === "1") return new MockRpcClient();

  const baseUrl = readBaseUrl();
  const http = new HttpSseClient({ baseUrl, token: readToken() });

  if (await healthy(baseUrl)) return http;

  // No real server reachable. Mock keeps Vite-only dev workflows
  // working; production builds at a real origin keep `HttpSseClient`
  // so the surfaced error is a clear "server unreachable" rather
  // than a silent mock fallback.
  if (isViteDevPort(window.location.port)) return new MockRpcClient();
  return http;
}

async function healthy(baseUrl: string): Promise<boolean> {
  try {
    const ctl = new AbortController();
    const timer = window.setTimeout(() => ctl.abort(), 1000);
    const res = await fetch(`${baseUrl}/healthz`, { signal: ctl.signal });
    window.clearTimeout(timer);
    return res.ok;
  } catch {
    return false;
  }
}

void (async () => {
  const client = await buildClient();
  const isMock = client instanceof MockRpcClient;

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <ShellProvider capabilities={capabilities}>
      <RpcProvider client={client}>
        <App />
        {isMock && <MockBanner />}
      </RpcProvider>
    </ShellProvider>,
  );
})();

// Tiny non-intrusive corner badge so it's obvious which transport
// the app is talking to. Dismissable but stateless — refreshes back
// in if you reload; the URL/query string is the source of truth.
function MockBanner() {
  return (
    <div
      style={{
        position: "fixed",
        bottom: 8,
        right: 8,
        zIndex: 1000,
        padding: "4px 10px",
        fontSize: 11,
        fontFamily: "JetBrains Mono, monospace",
        borderRadius: 6,
        background: "rgba(234, 179, 8, 0.15)",
        color: "rgb(180, 130, 0)",
        border: "1px solid rgba(234, 179, 8, 0.45)",
        pointerEvents: "none",
      }}
    >
      mock mode · no backend
    </div>
  );
}
