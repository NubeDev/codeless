import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "@xterm/xterm/css/xterm.css";
import "../../styles/globals.css";

import { getCurrentWindow } from "@tauri-apps/api/window";
import ReactDOM from "react-dom/client";
import App from "../../app/App";
import { IS_MAC } from "../../lib/platform";
import { RpcProvider, TauriIpcClient } from "../../lib/rpc";
import {
  fallbackAppInfo,
  registerCrossWindowEvents,
  registerKVStoreFactory,
  ShellProvider,
  type AppInfo,
  type ShellCapabilities,
} from "../../lib/shell";
import { readTauriAppInfo } from "./app-info";
import { tauriAutostart } from "./autostart";
import { tauriCrossWindowEvents } from "./cross-window-events";
import { tauriExternalOpener } from "./external-opener";
import { tauriKVFactory } from "./kv-store";
import { tauriNetworkProbe } from "./network-probe";
import { tauriPaths } from "./paths";
import { tauriSettingsWindow } from "./settings-window";
import { tauriUpdater } from "./updater";
import { tauriWindowControls } from "./window-controls";

// Register the singleton adapters before any consumer module runs.
// The settings module and AI loaders read `getStore(name)` lazily at
// call time, so this happens-before relationship is what makes the
// module-level registry safe.
registerKVStoreFactory(tauriKVFactory);
registerCrossWindowEvents(tauriCrossWindowEvents);

// Desktop owns its own window chrome on every platform except macOS,
// which keeps the native traffic lights via Tauri's overlay title bar.
const capabilities: ShellCapabilities = {
  customWindowControls: !IS_MAC,
};

if (capabilities.customWindowControls) {
  document.documentElement.dataset.chrome = "borderless";
}

function mount(appInfo: AppInfo) {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <ShellProvider
      capabilities={capabilities}
      windowControls={tauriWindowControls}
      externalOpener={tauriExternalOpener}
      updater={tauriUpdater}
      appInfo={appInfo}
      paths={tauriPaths}
      autostart={tauriAutostart}
      settingsWindow={tauriSettingsWindow}
      networkProbe={tauriNetworkProbe}
    >
      <RpcProvider client={new TauriIpcClient()}>
        <App />
      </RpcProvider>
    </ShellProvider>,
  );
}

// Tauri's getName/getVersion are async; await them so the first paint
// already has the right "About" strings. Falls back gracefully if the
// Tauri runtime isn't there (e.g. running this entry under `vite dev`
// without `tauri dev`).
readTauriAppInfo()
  .then(mount)
  .catch(() => mount(fallbackAppInfo));

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
