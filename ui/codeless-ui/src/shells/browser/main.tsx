import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "@xterm/xterm/css/xterm.css";
import "../../styles/globals.css";

import ReactDOM from "react-dom/client";

import App from "../../app/App";
import {
  HttpSseClient,
  RpcProvider,
  readBaseUrl,
  readToken,
} from "../../lib/rpc";

const rpc = new HttpSseClient({
  baseUrl: readBaseUrl(),
  token: readToken(),
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <RpcProvider client={rpc}>
    <App />
  </RpcProvider>,
);
