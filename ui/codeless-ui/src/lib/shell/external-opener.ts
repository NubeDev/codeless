// Open a URL outside the app — the system browser on desktop, a new
// tab on web. Tauri webviews block `window.open` by default so the
// desktop shell routes through `@tauri-apps/plugin-opener`; the
// browser shell uses `window.open` directly.

export interface ExternalOpenerAdapter {
  openUrl(url: string): Promise<void>;
}

export const browserExternalOpener: ExternalOpenerAdapter = {
  openUrl: async (url) => {
    window.open(url, "_blank", "noopener,noreferrer");
  },
};
