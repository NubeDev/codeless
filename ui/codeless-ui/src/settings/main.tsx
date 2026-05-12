import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "../styles/globals.css";

import { getCurrentWindow } from "@tauri-apps/api/window";
import ReactDOM from "react-dom/client";
import { ThemeProvider } from "@/modules/theme";
import { IS_MAC } from "@/lib/platform";
import {
  fallbackAppInfo,
  registerCrossWindowEvents,
  registerKVStoreFactory,
  ShellProvider,
  type AppInfo,
  type ShellCapabilities,
} from "@/lib/shell";
import { readTauriAppInfo } from "@/shells/desktop/app-info";
import { tauriAutostart } from "@/shells/desktop/autostart";
import { tauriCrossWindowEvents } from "@/shells/desktop/cross-window-events";
import { tauriExternalOpener } from "@/shells/desktop/external-opener";
import { tauriKVFactory } from "@/shells/desktop/kv-store";
import { tauriNetworkProbe } from "@/shells/desktop/network-probe";
import { tauriPaths } from "@/shells/desktop/paths";
import { tauriSettingsWindow } from "@/shells/desktop/settings-window";
import { tauriUpdater } from "@/shells/desktop/updater";
import { tauriWindowControls } from "@/shells/desktop/window-controls";

registerKVStoreFactory(tauriKVFactory);
registerCrossWindowEvents(tauriCrossWindowEvents);
import { SettingsApp } from "./SettingsApp";

// The settings window is a desktop-shell concept (multi-window Tauri).
// Mirror desktop's chrome rules: borderless on non-mac, native traffic
// lights on mac. When the audit's "Settings window mgmt" item lands and
// settings moves to in-app routing, this entry goes away.
const capabilities: ShellCapabilities = {
  customWindowControls: !IS_MAC,
};

if (capabilities.customWindowControls) {
  document.documentElement.dataset.chrome = "borderless";
}

function mount(appInfo: AppInfo) {
  ReactDOM.createRoot(
    document.getElementById("settings-root") as HTMLElement,
  ).render(
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
      <ThemeProvider>
        <SettingsApp />
      </ThemeProvider>
    </ShellProvider>,
  );
}

readTauriAppInfo()
  .then(mount)
  .catch(() => mount(fallbackAppInfo));

const showWindow = () => {
  getCurrentWindow()
    .show()
    .catch((e) => console.error("settings show failed:", e));
};
setTimeout(showWindow, 50);
setTimeout(showWindow, 500);
