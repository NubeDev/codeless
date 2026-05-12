// App identity strings shown in Settings → About. Tauri reads from the
// bundle manifest at runtime; browser / mobile shells read from build-
// time Vite defines or hard-code their own values. All fields are
// synchronous after construction — the desktop shell awaits Tauri's
// async getters once at startup and freezes the result.

export interface AppInfo {
  name: string;
  version: string;
  /** Display string like "macOS · aarch64", or null when the shell
   *  can't determine a meaningful one (browser). */
  buildLabel: string | null;
}

export const fallbackAppInfo: AppInfo = {
  name: "Codeless",
  version: "",
  buildLabel: null,
};
