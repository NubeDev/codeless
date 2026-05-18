import { open } from "@tauri-apps/plugin-dialog";

import type { PathPicker } from "@/lib/shell";

// `plugin-dialog` returns the absolute OS path directly, so the
// desktop shell has no typed-input fallback — the user always
// confirms by clicking inside a native chooser. The caller still
// hands the result to `validate_workspace_path`; the picker does
// not pre-validate.
export const tauriPathPicker: PathPicker = {
  async pickDirectory({ startPath } = {}) {
    const picked = await open({
      directory: true,
      multiple: false,
      defaultPath: startPath,
    });
    if (picked == null) return null;
    return Array.isArray(picked) ? (picked[0] ?? null) : picked;
  },
};
