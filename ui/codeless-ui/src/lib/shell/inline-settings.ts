// Single source of truth for the in-app Settings overlay. Lives next
// to the shell-injection adapters because that's the seam: the desktop
// shell opens a separate Tauri window and never touches this store;
// browser / mobile shells flip the store to render `<SettingsApp />`
// inline inside `<App />`. App.tsx subscribes and renders the overlay
// when `open === true`.
//
// The store is a Zustand singleton because settings is a singleton —
// at most one settings panel can be open at a time, and the close
// button needs to talk to whatever opened it without prop-drilling.

import { create } from "zustand";

import type { SettingsTab } from "./settings-window";

interface InlineSettingsStore {
  open: boolean;
  tab: SettingsTab;
  /** Opens the panel, optionally switching to a specific tab. The tab
   *  defaults to whatever was last shown so re-opens land where the
   *  user left off, matching the Tauri-window behavior. */
  show(tab?: SettingsTab): void;
  hide(): void;
}

export const useInlineSettingsStore = create<InlineSettingsStore>((set) => ({
  open: false,
  tab: "general",
  show: (tab) =>
    set((s) => ({ open: true, tab: tab ?? s.tab })),
  hide: () => set({ open: false }),
}));
