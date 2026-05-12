import { getCurrentWindow } from "@tauri-apps/api/window";

import type { WindowControlsAdapter } from "@/lib/shell";

// Tauri-backed `WindowControlsAdapter`. The only `@tauri-apps/api/*`
// import path outside `src/shells/desktop/` is forbidden by R2;
// keeping this in the desktop shell directory is the legitimate seam.

export const tauriWindowControls: WindowControlsAdapter = {
  isMaximized: () => getCurrentWindow().isMaximized(),
  onResized: (cb) =>
    getCurrentWindow().onResized(() => cb()).then((un) => un),
  minimize: () => getCurrentWindow().minimize(),
  toggleMaximize: () => getCurrentWindow().toggleMaximize(),
  close: () => getCurrentWindow().close(),
};
