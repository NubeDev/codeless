import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "@xterm/xterm/css/xterm.css";
import "../../styles/globals.css";

import { getCurrentWindow } from "@tauri-apps/api/window";
import ReactDOM from "react-dom/client";
import App from "../../app/App";
import { IS_MAC } from "../../lib/platform";
import { RpcProvider, TauriIpcClient } from "../../lib/rpc";
import { ShellProvider, type ShellCapabilities } from "../../lib/shell";
import { tauriWindowControls } from "./window-controls";

// Desktop owns its own window chrome on every platform except macOS,
// which keeps the native traffic lights via Tauri's overlay title bar.
const capabilities: ShellCapabilities = {
  customWindowControls: !IS_MAC,
};

if (capabilities.customWindowControls) {
  document.documentElement.dataset.chrome = "borderless";
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <ShellProvider
    capabilities={capabilities}
    windowControls={tauriWindowControls}
  >
    <RpcProvider client={new TauriIpcClient()}>
      <App />
    </RpcProvider>
  </ShellProvider>,
);

// Window starts hidden (per tauri.conf.json) so users never see a transparent
// shadow-only frame before React paints. Use setTimeout — rAF is throttled
// while the window is hidden and would never fire.
const showWindow = () => {
  getCurrentWindow()
    .show()
    .catch((e) => console.error("window.show failed:", e));
};
setTimeout(showWindow, 50);
// Safety net: if the first show somehow fails to take effect, force again.
setTimeout(showWindow, 500);
