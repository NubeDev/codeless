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

// Detect whether the browser exposes the File System Access API's
// directory picker. Chromium-family browsers do; Firefox and Safari
// do not. The injector falls back to a typed-input prompt in that
// case so the UI component above it never has to branch.
function hasShowDirectoryPicker(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof (window as unknown as { showDirectoryPicker?: unknown })
      .showDirectoryPicker === "function"
  );
}

// `showDirectoryPicker()` does not surface the underlying OS path —
// it returns a sandboxed handle. The user is supplying a path they
// already know (they have to, since the server stores the absolute
// path); the handle's `name` is the directory basename and acts as
// a hint pre-filled into the typed-input fallback. The caller is
// still responsible for shipping the full path to the validator.
export const browserPathPicker: PathPicker = {
  async pickDirectory({ startPath } = {}) {
    const initial = startPath ?? "";
    if (hasShowDirectoryPicker()) {
      try {
        const showPicker = (
          window as unknown as {
            showDirectoryPicker: (opts?: {
              mode?: "read" | "readwrite";
            }) => Promise<{ name: string }>;
          }
        ).showDirectoryPicker;
        const handle = await showPicker({ mode: "read" });
        const hint = handle.name;
        const typed = window.prompt(
          "Enter the absolute path to this directory on the server:",
          initial || (hint ? `/${hint}` : ""),
        );
        return typed?.trim() ? typed.trim() : null;
      } catch (e) {
        // User cancelled the native picker — surface as cancel, not
        // as a thrown error, so callers can treat the typed-input
        // fallback and the native picker identically.
        if (
          e instanceof DOMException &&
          (e.name === "AbortError" || e.name === "NotAllowedError")
        ) {
          return null;
        }
        // Any other failure falls through to the typed-input path.
      }
    }
    const typed = window.prompt(
      "Enter the absolute path to the workspace directory:",
      initial,
    );
    return typed?.trim() ? typed.trim() : null;
  },
};
