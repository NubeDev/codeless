import { emit, listen } from "@tauri-apps/api/event";

import type { CrossWindowEventsAdapter } from "@/lib/shell";

export const tauriCrossWindowEvents: CrossWindowEventsAdapter = {
  emit: (event, payload) => emit(event, payload),
  listen: async (event, cb) => {
    const un = await listen(event, (e) => cb(e.payload as never));
    return un;
  },
};
