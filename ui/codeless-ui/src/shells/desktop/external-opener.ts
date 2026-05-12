import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";

import type { ExternalOpenerAdapter } from "@/lib/shell";

export const tauriExternalOpener: ExternalOpenerAdapter = {
  openUrl: (url) => openUrl(url),
  revealPath: (path) => revealItemInDir(path),
};
