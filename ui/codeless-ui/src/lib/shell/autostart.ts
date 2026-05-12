// Launch-at-login is a desktop-only capability. Browser tabs and
// mobile apps can't opt themselves into auto-launch; the OS settings
// own that. The desktop shell wraps `@tauri-apps/plugin-autostart`;
// browser / mobile inject `noopAutostart` and the toggle in Settings
// reads as "not supported" → consumers hide the row.

export interface AutostartAdapter {
  readonly supported: boolean;
  isEnabled(): Promise<boolean>;
  enable(): Promise<void>;
  disable(): Promise<void>;
}

export const noopAutostart: AutostartAdapter = {
  supported: false,
  isEnabled: () => Promise.resolve(false),
  enable: () => Promise.resolve(),
  disable: () => Promise.resolve(),
};
