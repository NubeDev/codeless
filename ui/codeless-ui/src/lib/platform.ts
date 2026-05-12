// OS detection from the user-agent. Works identically in browser, in a
// Tauri webview, and on mobile webviews — no Tauri plugin needed.
//
// Shell-level capabilities (e.g. "does this shell own the window
// chrome?") live in `src/lib/shell/`, not here. This file is for
// platform-derived constants only: keyboard modifier names, shortcut
// labels, and the `IS_*` booleans every module imports synchronously.

type Os = "macos" | "linux" | "windows" | "ios" | "android" | "unknown";

function detectOs(): Os {
  if (typeof navigator === "undefined") return "unknown";
  const ua = navigator.userAgent;
  if (/iPad|iPhone|iPod/.test(ua)) return "ios";
  if (/Android/.test(ua)) return "android";
  if (/Mac/.test(ua)) return "macos";
  if (/Win/.test(ua)) return "windows";
  if (/Linux/.test(ua)) return "linux";
  return "unknown";
}

const OS: Os = detectOs();

export const IS_MAC = OS === "macos";
export const IS_LINUX = OS === "linux";
export const IS_WINDOWS = OS === "windows";

export const MOD_KEY = IS_MAC ? "⌘" : "Ctrl";
/** KeyBinding property name for the platform's primary modifier. */
export const MOD_PROP: "meta" | "ctrl" = IS_MAC ? "meta" : "ctrl";
export const CTRL_KEY = IS_MAC ? "⌃" : "Ctrl";
export const ALT_KEY = IS_MAC ? "⌥" : "Alt";
export const SHIFT_KEY = IS_MAC ? "⇧" : "Shift";
export const TAB_KEY = IS_MAC ? "⇥" : "Tab";
export const ENTER_KEY = IS_MAC ? "↵" : "Enter";

export const KEY_SEP = IS_MAC ? "" : "+";

export function fmtShortcut(...parts: string[]): string {
  return parts.join(KEY_SEP);
}
