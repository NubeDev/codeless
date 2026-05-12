import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";

import type { UpdateHandle, UpdaterAdapter } from "@/lib/shell";

// Wraps `@tauri-apps/plugin-updater`'s `Update` object in the
// shell-agnostic `UpdateHandle` shape. The plugin emits
// `contentLength` as `number | undefined` on the `Started` event; the
// shell-side type uses `number | null`, so the progress callback is
// adapted here so consumers don't carry that variance.

export const tauriUpdater: UpdaterAdapter = {
  supported: true,
  check: async () => {
    const update = await check();
    if (!update) return null;
    const handle: UpdateHandle = {
      version: update.version,
      body: update.body ?? null,
      downloadAndInstall: (onProgress) =>
        update.downloadAndInstall((event) => {
          if (event.event === "Started") {
            onProgress({
              event: "Started",
              data: { contentLength: event.data.contentLength ?? null },
            });
          } else {
            onProgress(event);
          }
        }),
    };
    return handle;
  },
  relaunch: () => relaunch(),
};
