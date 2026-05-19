// Pick a directory on disk. The picker is the only piece of the
// workspace-attach flow that legitimately differs by shell — every
// other Tauri-vs-browser branch is hidden behind `RpcClient`. The
// contract is deliberately weak: the returned string is *not*
// trusted, even on desktop, because the caller hands it back to
// `validate_workspace_path` on the server before anything attaches.
// This keeps the same code path for the typed-input fallback (where
// the user supplies a path manually) and the OS-native picker.

export interface PathPicker {
  // Returns the path the user picked (or typed). `null` means the
  // user cancelled. The string is not canonicalised; the caller must
  // pass it through `validate_workspace_path` to resolve symlinks and
  // run the policy checks.
  pickDirectory(opts?: { startPath?: string }): Promise<string | null>;
}

// The File System Access API's `showDirectoryPicker()` is deliberately
// not used here. It returns a sandboxed handle whose only path-shaped
// field is `handle.name` — the directory basename, never the absolute
// OS path. Round-tripping that as `/${basename}` faked an absolute
// path the server then rejected (e.g. picking `/home/me/code/rubix`
// landed in the prompt as `/rubix`). The honest browser-shell flow is
// a single typed prompt, paired with the dialog's live
// `validate_workspace_path` feedback on the surrounding input. The
// caller is still responsible for shipping the full path to the
// validator before anything attaches.
export const browserPathPicker: PathPicker = {
  async pickDirectory({ startPath } = {}) {
    const typed = window.prompt(
      "Enter the absolute path to the workspace directory:",
      startPath ?? "",
    );
    return typed?.trim() ? typed.trim() : null;
  },
};
