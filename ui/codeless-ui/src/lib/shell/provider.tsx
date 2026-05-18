import { createContext, useContext, type ReactNode } from "react";

import { fallbackAppInfo, type AppInfo } from "./app-info";
import { noopAutostart, type AutostartAdapter } from "./autostart";
import type { ShellCapabilities } from "./capabilities";
import {
  browserExternalOpener,
  type ExternalOpenerAdapter,
} from "./external-opener";
import {
  browserNetworkProbe,
  type NetworkProbeAdapter,
} from "./network-probe";
import { browserPathPicker, type PathPicker } from "./path-picker";
import { noopPaths, type PathsAdapter } from "./paths";
import {
  browserSettingsWindow,
  type SettingsWindowAdapter,
} from "./settings-window";
import { noopUpdater, type UpdaterAdapter } from "./updater";
import {
  noopWindowControls,
  type WindowControlsAdapter,
} from "./window-controls";

interface ShellValue {
  capabilities: ShellCapabilities;
  windowControls: WindowControlsAdapter;
  externalOpener: ExternalOpenerAdapter;
  updater: UpdaterAdapter;
  appInfo: AppInfo;
  paths: PathsAdapter;
  pathPicker: PathPicker;
  autostart: AutostartAdapter;
  settingsWindow: SettingsWindowAdapter;
  networkProbe: NetworkProbeAdapter;
}

const ShellContext = createContext<ShellValue | null>(null);

type ProviderProps = {
  capabilities: ShellCapabilities;
  windowControls?: WindowControlsAdapter;
  externalOpener?: ExternalOpenerAdapter;
  updater?: UpdaterAdapter;
  appInfo?: AppInfo;
  paths?: PathsAdapter;
  pathPicker?: PathPicker;
  autostart?: AutostartAdapter;
  settingsWindow?: SettingsWindowAdapter;
  networkProbe?: NetworkProbeAdapter;
  children: ReactNode;
};

export function ShellProvider({
  capabilities,
  windowControls,
  externalOpener,
  updater,
  appInfo,
  paths,
  pathPicker,
  autostart,
  settingsWindow,
  networkProbe,
  children,
}: ProviderProps) {
  const value: ShellValue = {
    capabilities,
    windowControls: windowControls ?? noopWindowControls,
    externalOpener: externalOpener ?? browserExternalOpener,
    updater: updater ?? noopUpdater,
    appInfo: appInfo ?? fallbackAppInfo,
    paths: paths ?? noopPaths,
    pathPicker: pathPicker ?? browserPathPicker,
    autostart: autostart ?? noopAutostart,
    settingsWindow: settingsWindow ?? browserSettingsWindow,
    networkProbe: networkProbe ?? browserNetworkProbe,
  };
  return (
    <ShellContext.Provider value={value}>{children}</ShellContext.Provider>
  );
}

export function useShell(): ShellValue {
  const v = useContext(ShellContext);
  if (!v) throw new Error("useShell must be used inside <ShellProvider>");
  return v;
}

export function useShellCapabilities(): ShellCapabilities {
  return useShell().capabilities;
}

export function useWindowControls(): WindowControlsAdapter {
  return useShell().windowControls;
}

export function useExternalOpener(): ExternalOpenerAdapter {
  return useShell().externalOpener;
}

export function useUpdaterAdapter(): UpdaterAdapter {
  return useShell().updater;
}

export function useAppInfo(): AppInfo {
  return useShell().appInfo;
}

export function usePaths(): PathsAdapter {
  return useShell().paths;
}

export function usePathPicker(): PathPicker {
  return useShell().pathPicker;
}

export function useAutostart(): AutostartAdapter {
  return useShell().autostart;
}

export function useSettingsWindow(): SettingsWindowAdapter {
  return useShell().settingsWindow;
}

export function useNetworkProbe(): NetworkProbeAdapter {
  return useShell().networkProbe;
}
