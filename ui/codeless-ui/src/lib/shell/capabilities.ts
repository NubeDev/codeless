// Per-shell capability flags. The shell entry constructs these once
// at startup; components read via `useShell()`. New flags belong here
// when they describe *what the shell can do*, not *who is calling* —
// the UI must not branch on shell kind directly.

export interface ShellCapabilities {
  /** Shell renders its own min / max / close buttons (Tauri desktop on
   * non-macOS). macOS desktop uses native traffic lights, so this is
   * false there. Browser / mobile shells never draw window chrome. */
  customWindowControls: boolean;
}
