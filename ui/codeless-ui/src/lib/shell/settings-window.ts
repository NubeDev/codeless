// Open the Settings surface. Desktop spawns a separate Tauri webview
// window via `invoke("open_settings_window", { tab })`; browser /
// mobile shells render `<SettingsApp />` inline inside `<App />` as a
// full-screen overlay, driven by `useInlineSettingsStore` (see
// `inline-settings.ts`). Callers don't branch on shell — they just
// call `open(tab)` and the adapter dispatches to the right surface.

import { useInlineSettingsStore } from "./inline-settings";

export type SettingsTab =
  | "general"
  | "shortcuts"
  | "models"
  | "agents"
  | "about";

export interface SettingsWindowAdapter {
  open(tab?: SettingsTab): Promise<void>;
}

export const browserSettingsWindow: SettingsWindowAdapter = {
  open: async (tab) => {
    useInlineSettingsStore.getState().show(tab);
  },
};
