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
//   - `?mock=1`  → force MockRpcClient
//   - `?real=1`  → force HttpSseClient even on the Vite dev port
//   - else, on the Vite dev port (1420) with no explicit baseUrl
//     configured (localStorage or VITE_CODELESS_BASE_URL), default to
//     mock so the dev experience works without a backend running
//   - else, HttpSseClient against readBaseUrl()
function buildClient(): RpcClient {
  const params = new URLSearchParams(window.location.search);
  if (params.get("mock") === "1") return new MockRpcClient();
  if (params.get("real") === "1") {
    return new HttpSseClient({ baseUrl: readBaseUrl(), token: readToken() });
  }
  const onViteDev = window.location.port === "1420";
  const hasExplicitBase =
    !!safeLocalStorage("codeless-rpc-base-url") ||
    !!import.meta.env.VITE_CODELESS_BASE_URL;
  if (onViteDev && !hasExplicitBase) return new MockRpcClient();
  return new HttpSseClient({ baseUrl: readBaseUrl(), token: readToken() });
}

function safeLocalStorage(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

const client = buildClient();
const isMock = client instanceof MockRpcClient;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <ShellProvider capabilities={capabilities}>
    <RpcProvider client={client}>
      <App />
      {isMock && <MockBanner />}
    </RpcProvider>
  </ShellProvider>,
);

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
