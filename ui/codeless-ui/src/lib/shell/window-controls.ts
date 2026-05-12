// Capability adapter for native window chrome (min / max / close).
//
// Browser and mobile shells do not own a window — the host browser /
// OS draws the chrome and these calls are meaningless. Desktop (Tauri)
// shells inject a real implementation that drives the current webview
// window. Components depend on the interface; the shell decides what
// to plug in.

export interface WindowControlsAdapter {
  isMaximized(): Promise<boolean>;
  /** Subscribe to resize events; returns an unlisten. */
  onResized(cb: () => void): Promise<() => void>;
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  close(): Promise<void>;
}

// No-op adapter for shells that don't own a window. Returning a stable
// resolved promise keeps consumer call sites the same in browser /
// mobile as on desktop — they simply never fire.
export const noopWindowControls: WindowControlsAdapter = {
  isMaximized: () => Promise.resolve(false),
  onResized: () => Promise.resolve(() => {}),
  minimize: () => Promise.resolve(),
  toggleMaximize: () => Promise.resolve(),
  close: () => Promise.resolve(),
};
