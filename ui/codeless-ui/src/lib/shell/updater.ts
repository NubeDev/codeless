// Auto-update is a desktop-only concern. Tauri's updater plugin
// downloads a signed bundle and relaunches the process; that has no
// counterpart in a browser tab (the page just reloads) or on mobile
// (the store handles it). Browser / mobile shells inject the no-op
// adapter and the updater UI disappears.
//
// The adapter is intentionally narrow: `useUpdater` owns the state
// machine (idle / checking / available / downloading / ready / error)
// and only delegates the side-effecting Tauri calls here. That keeps
// the React surface identical across shells.

export interface UpdateHandle {
  version: string;
  /** Optional release notes; the dialog falls back to a generic blurb. */
  body: string | null;
  downloadAndInstall(
    onProgress: (event: UpdateProgress) => void,
  ): Promise<void>;
}

export type UpdateProgress =
  | { event: "Started"; data: { contentLength: number | null } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };

export interface UpdaterAdapter {
  /** Returns `null` when no update is available, otherwise a handle
   *  whose `downloadAndInstall` triggers the bundle download. */
  check(): Promise<UpdateHandle | null>;
  /** Relaunch the application — called after a successful install. */
  relaunch(): Promise<void>;
  /** True when the shell supports updates; the UI hides itself when
   *  false (e.g. browser / mobile). */
  readonly supported: boolean;
}

export const noopUpdater: UpdaterAdapter = {
  supported: false,
  check: () => Promise.resolve(null),
  relaunch: () => Promise.resolve(),
};
