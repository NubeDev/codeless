import { useCallback, useEffect, useMemo, useRef } from "react";

import { useWorkspacesStore } from "@/modules/workspaces/store";

import type { Tab } from "./useTabs";

type Result = {
  explorerRoot: string | null;
  inheritedCwdForNewTab: () => string | undefined;
};

export function useWorkspaceCwd(
  activeTab: Tab | undefined,
  tabs: Tab[],
  home: string | null,
): Result {
  const lastTerminalCwd = useRef<string | null>(null);

  useEffect(() => {
    if (activeTab?.kind === "terminal" && activeTab.cwd) {
      lastTerminalCwd.current = activeTab.cwd;
    }
  }, [activeTab]);

  // After workspace-attach landed, the server jails every `fs.*` call
  // to the attached roots. The user's OS home (`/home/<user>`) is no
  // longer a valid fallback — the runtime returns PermissionDenied on
  // any path outside an attached workspace. Resolve the active
  // workspace's `fs_root` ahead of `home` so a fresh attach lights up
  // the explorer without the user opening a terminal first.
  const activeWorkspaceRoot = useWorkspacesStore((s) => {
    if (s.activeRepoId === null) return null;
    const ws = s.workspaces.find((w) => w.repo_id === s.activeRepoId);
    return ws?.fs_root ?? null;
  });

  const explorerRoot = useMemo<string | null>(() => {
    if (activeTab?.kind === "terminal" && activeTab.cwd) return activeTab.cwd;
    if (lastTerminalCwd.current) return lastTerminalCwd.current;
    const anyTerm = tabs.find((t) => t.kind === "terminal" && t.cwd);
    if (anyTerm?.kind === "terminal" && anyTerm.cwd) return anyTerm.cwd;
    if (activeWorkspaceRoot) return activeWorkspaceRoot;
    return home;
  }, [activeTab, tabs, activeWorkspaceRoot, home]);

  const inheritedCwdForNewTab = useCallback((): string | undefined => {
    if (activeTab?.kind === "terminal" && activeTab.cwd) return activeTab.cwd;
    // Editor tabs inherit the last terminal's cwd (or workspace root /
    // shell home), not the file's folder — opening a new terminal from
    // a file shouldn't hijack the user's working directory context.
    return (
      lastTerminalCwd.current ?? activeWorkspaceRoot ?? home ?? undefined
    );
  }, [activeTab, activeWorkspaceRoot, home]);

  return { explorerRoot, inheritedCwdForNewTab };
}
