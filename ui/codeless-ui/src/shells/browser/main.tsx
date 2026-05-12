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

// `?mock=1` swaps the transport for an in-memory `MockRpcClient` —
// useful for dev'ing UI surfaces without a running codeless-server.
function buildClient(): RpcClient {
  const params = new URLSearchParams(window.location.search);
  if (params.get("mock") === "1") return new MockRpcClient();
  return new HttpSseClient({ baseUrl: readBaseUrl(), token: readToken() });
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <ShellProvider capabilities={capabilities}>
    <RpcProvider client={buildClient()}>
      <App />
    </RpcProvider>
  </ShellProvider>,
);
