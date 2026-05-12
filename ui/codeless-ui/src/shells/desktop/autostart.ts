import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";

import type { AutostartAdapter } from "@/lib/shell";

export const tauriAutostart: AutostartAdapter = {
  supported: true,
  isEnabled: () => isEnabled(),
  enable: () => enable(),
  disable: () => disable(),
};
