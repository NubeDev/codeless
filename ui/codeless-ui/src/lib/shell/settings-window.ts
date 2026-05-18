// Open the Settings surface. Every shell renders `<SettingsApp />`
// inline inside `<App />` as a full-screen overlay, driven by
// `useInlineSettingsStore` (see `inline-settings.ts`).

import { useInlineSettingsStore } from "./inline-settings";

export type SettingsTab =
  | "general"
  | "shortcuts"
  | "models"
  | "agents"
  | "workspaces"
  | "about";

export interface SettingsWindowAdapter {
  open(tab?: SettingsTab): Promise<void>;
}

export const browserSettingsWindow: SettingsWindowAdapter = {
  open: async (tab) => {
    useInlineSettingsStore.getState().show(tab);
  },
};
