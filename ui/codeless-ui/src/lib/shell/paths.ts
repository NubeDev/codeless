// Filesystem path resolution. Today the only consumer is App.tsx, which
// asks for the user's home directory at startup so the file-explorer
// root has a sensible default. Browser/mobile shells return null
// because the host has no concept of a user-owned path; the App falls
// back to its workspace heuristics.

export interface PathsAdapter {
  /** Forward-slash form, no trailing slash. Returns null when the
   *  shell has no notion of a home directory. */
  homeDir(): Promise<string | null>;
}

export const noopPaths: PathsAdapter = {
  homeDir: () => Promise.resolve(null),
};
