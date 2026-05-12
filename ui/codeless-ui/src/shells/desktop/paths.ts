import { homeDir } from "@tauri-apps/api/path";

import type { PathsAdapter } from "@/lib/shell";

export const tauriPaths: PathsAdapter = {
  homeDir: () => homeDir(),
};
