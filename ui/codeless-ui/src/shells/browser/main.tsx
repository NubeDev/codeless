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
import { JobsDashboard } from "../../modules/jobs";

// `?mock=1` swaps the transport for an in-memory `MockRpcClient` —
// useful for dev'ing UI surfaces without a running codeless-server.
function buildClient(): RpcClient {
  const params = new URLSearchParams(window.location.search);
  if (params.get("mock") === "1") return new MockRpcClient();
  return new HttpSseClient({ baseUrl: readBaseUrl(), token: readToken() });
}

// `?view=jobs` renders the new Phase 2 jobs dashboard in place of the
// upstream `<App />` shell — temporary route until the dashboard is
// wired into the App's panel structure.
function pickRoot(): React.ReactNode {
  const params = new URLSearchParams(window.location.search);
  if (params.get("view") === "jobs") return <JobsDashboard />;
  return <App />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <RpcProvider client={buildClient()}>{pickRoot()}</RpcProvider>,
);
