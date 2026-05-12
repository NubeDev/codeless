import { invoke } from "@tauri-apps/api/core";

import type { SettingsWindowAdapter } from "@/lib/shell";

export const tauriSettingsWindow: SettingsWindowAdapter = {
  open: (tab) => invoke<void>("open_settings_window", { tab: tab ?? null }),
};
