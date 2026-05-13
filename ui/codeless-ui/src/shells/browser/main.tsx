import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "@xterm/xterm/css/xterm.css";
import "../../styles/globals.css";

import { useEffect, useState } from "react";
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

// Transport selection:
//   - `?mock=1` → opt into MockRpcClient explicitly. Used for UI-only
//     dev work without booting the Rust server.
//   - Otherwise always `HttpSseClient`. If the server is unreachable
//     the app renders an honest "cannot reach server" screen so the
//     UI never silently shows fake data.
function buildHttpClient(): HttpSseClient {
  return new HttpSseClient({ baseUrl: readBaseUrl(), token: readToken() });
}

function isMockRequested(): boolean {
  return new URLSearchParams(window.location.search).get("mock") === "1";
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

function Root() {
  // null = probing; "ok" = real server; "down" = unreachable.
  const [state, setState] = useState<"probing" | "ok" | "down" | "mock">(
    isMockRequested() ? "mock" : "probing",
  );
  const [client, setClient] = useState<RpcClient | null>(() =>
    isMockRequested() ? new MockRpcClient() : null,
  );
  const [baseUrl] = useState(readBaseUrl);

  useEffect(() => {
    if (state !== "probing") return;
    let cancelled = false;
    void (async () => {
      const ok = await healthy(baseUrl);
      if (cancelled) return;
      if (ok) {
        setClient(buildHttpClient());
        setState("ok");
      } else {
        setState("down");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [state, baseUrl]);

  if (state === "probing") return null;
  if (state === "down") return <ServerDownScreen baseUrl={baseUrl} onRetry={() => setState("probing")} />;
  if (!client) return null;

  const isMock = client instanceof MockRpcClient;
  return (
    <ShellProvider capabilities={capabilities}>
      <RpcProvider client={client}>
        <App />
        {isMock && <MockBanner />}
      </RpcProvider>
    </ShellProvider>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <Root />,
);

// Honest error screen when the Rust core is not reachable. Names the
// URL we tried so the user can see whether their config or the server
// is the problem, and offers a one-click retry that re-runs the probe
// rather than forcing a full page reload.
function ServerDownScreen({
  baseUrl,
  onRetry,
}: {
  baseUrl: string;
  onRetry: () => void;
}) {
  return (
    <div
      style={{
        minHeight: "100vh",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        fontFamily: "JetBrains Mono, monospace",
        padding: 24,
      }}
    >
      <div style={{ maxWidth: 540 }}>
        <h1 style={{ fontSize: 18, fontWeight: 600, marginBottom: 12 }}>
          cannot reach codeless server
        </h1>
        <p style={{ fontSize: 13, lineHeight: 1.5, opacity: 0.8 }}>
          The UI tried to reach the Rust core at:
        </p>
        <pre
          style={{
            background: "rgba(127,127,127,0.12)",
            padding: "8px 10px",
            borderRadius: 6,
            fontSize: 12,
            margin: "8px 0 16px",
          }}
        >
          {baseUrl}
        </pre>
        <p style={{ fontSize: 13, lineHeight: 1.5, opacity: 0.8 }}>
          Start the server (see <code>DOCS/START-SERVER-UI.md</code>) and
          click retry. To run the UI without a backend, append{" "}
          <code>?mock=1</code> to the URL.
        </p>
        <button
          onClick={onRetry}
          style={{
            marginTop: 16,
            padding: "6px 14px",
            fontSize: 13,
            fontFamily: "inherit",
            border: "1px solid currentColor",
            borderRadius: 6,
            background: "transparent",
            color: "inherit",
            cursor: "pointer",
          }}
        >
          retry
        </button>
      </div>
    </div>
  );
}

// Tiny non-intrusive corner badge so it's obvious which transport
// the app is talking to. Only renders under `?mock=1` now — the
// server-down case has its own dedicated screen.
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
      mock mode · ?mock=1
    </div>
  );
}
