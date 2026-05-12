import { getName, getVersion } from "@tauri-apps/api/app";
import { arch, platform } from "@tauri-apps/plugin-os";

import { fallbackAppInfo, type AppInfo } from "@/lib/shell";

const PLATFORM_LABEL: Record<string, string> = {
  macos: "macOS",
  windows: "Windows",
  linux: "Linux",
  ios: "iOS",
  android: "Android",
  freebsd: "FreeBSD",
};

// Read once at startup. Tauri's `getName`/`getVersion` are async and
// `platform`/`arch` may throw outside a Tauri runtime; falling back to
// the shared `fallbackAppInfo` keeps the desktop entry resilient when
// the bundle is built without these values.
export async function readTauriAppInfo(): Promise<AppInfo> {
  try {
    const [name, version] = await Promise.all([getName(), getVersion()]);
    let buildLabel: string | null = null;
    try {
      const p = platform();
      const a = arch();
      buildLabel = `${PLATFORM_LABEL[p] ?? p} · ${a}`;
    } catch {
      buildLabel = null;
    }
    return { name, version, buildLabel };
  } catch {
    return fallbackAppInfo;
  }
}
