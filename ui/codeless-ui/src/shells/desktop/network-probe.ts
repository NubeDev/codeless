import { invoke } from "@tauri-apps/api/core";

import type { NetworkProbeAdapter } from "@/lib/shell";

export const tauriNetworkProbe: NetworkProbeAdapter = {
  ping: (url) => invoke<number>("http_ping", { url }),
};
