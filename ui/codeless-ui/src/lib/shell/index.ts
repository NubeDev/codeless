export type { AppInfo } from "./app-info";
export { fallbackAppInfo } from "./app-info";
export type { AutostartAdapter } from "./autostart";
export { noopAutostart } from "./autostart";
export type { ShellCapabilities } from "./capabilities";
export type { CrossWindowEventsAdapter } from "./cross-window-events";
export {
  getCrossWindowEvents,
  inProcessCrossWindowEvents,
  registerCrossWindowEvents,
} from "./cross-window-events";
export type { ExternalOpenerAdapter } from "./external-opener";
export { browserExternalOpener } from "./external-opener";
export type { KVStoreAdapter, KVStoreFactory } from "./kv-store";
export {
  getStore,
  localStorageKVFactory,
  registerKVStoreFactory,
} from "./kv-store";
export type { NetworkProbeAdapter } from "./network-probe";
export { browserNetworkProbe } from "./network-probe";
export type { PathPicker } from "./path-picker";
export { browserPathPicker } from "./path-picker";
export type { PathsAdapter } from "./paths";
export { noopPaths } from "./paths";
export type { SettingsTab, SettingsWindowAdapter } from "./settings-window";
export { browserSettingsWindow } from "./settings-window";
export { useInlineSettingsStore } from "./inline-settings";
export type {
  UpdateHandle,
  UpdateProgress,
  UpdaterAdapter,
} from "./updater";
export { noopUpdater } from "./updater";
export type { WindowControlsAdapter } from "./window-controls";
export { noopWindowControls } from "./window-controls";
export {
  ShellProvider,
  useAppInfo,
  useAutostart,
  useExternalOpener,
  useNetworkProbe,
  usePathPicker,
  usePaths,
  useSettingsWindow,
  useShell,
  useShellCapabilities,
  useUpdaterAdapter,
  useWindowControls,
} from "./provider";
