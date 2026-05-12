import { openUrl } from "@tauri-apps/plugin-opener";

import type { ExternalOpenerAdapter } from "@/lib/shell";

export const tauriExternalOpener: ExternalOpenerAdapter = {
  openUrl: (url) => openUrl(url),
};
