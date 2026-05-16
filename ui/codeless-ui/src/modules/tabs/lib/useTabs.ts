import { useCallback, useRef, useState } from "react";
import {
  hasLeaf,
  leafIds,
  nextLeafId,
  removeLeaf,
  setLeafCwd as setLeafCwdInTree,
  siblingLeafOf,
  splitLeaf,
  type PaneNode,
  type SplitDir,
} from "@/modules/terminal/lib/panes";

// Browsers cap WebGL contexts at ~16; one xterm renderer per leaf.
export const MAX_PANES_PER_TAB = 8;

export type TerminalTab = {
  id: number;
  kind: "terminal";
  title: string;
  cwd?: string;
  paneTree: PaneNode;
  activeLeafId: number;
};

export type EditorTab = {
  id: number;
  kind: "editor";
  title: string;
  path: string;
  dirty: boolean;
  /**
   * True while the tab is in the transient "preview" state — opened by a
   * single-click in the explorer and not yet pinned by the user. A preview tab
   * is replaced by the next single-click rather than accumulating.
   */
  preview: boolean;
};

export type PreviewTab = {
  id: number;
  kind: "preview";
  title: string;
  url: string;
};

export type AiDiffStatus = "pending" | "approved" | "rejected";

export type AiDiffTab = {
  id: number;
  kind: "ai-diff";
  title: string;
  path: string;
  /** "" for newly created files. */
  originalContent: string;
  proposedContent: string;
  /** Tool-call approval id used to resolve the AI SDK approval. */
  approvalId: string;
  status: AiDiffStatus;
  isNewFile: boolean;
};

export type JobsTab = {
  id: number;
  kind: "jobs";
  title: string;
};

// The `/assistant` surface is a singleton tab — like `JobsTab`, the
// underlying page is the global cross-thread rail, so a second tab
// would render identical content. Threads are not modelled as their
// own tab kind in stage 6; the rail handles selection in-page.
export type AssistantTab = {
  id: number;
  kind: "assistant";
  title: string;
};

// Surface C from `DOCS/SCOPE-MUTABLE-UI.md` — the cross-workspace
// patch worklist at `/patches`. Singleton for the same reason as
// `JobsTab` / `AssistantTab`: the page is a global view across every
// repo, so a second tab would render identical content.
export type PatchesTab = {
  id: number;
  kind: "patches";
  title: string;
};

// Per-job workspace tab. Distinct from `JobsTab` (the global list) so
// the user can have several jobs open in parallel — the natural read
// of "I want to watch run A finish while I drive run B". Title is
// derived from the job's prompt or its template name; the tab carries
// the id so `JobPage` can subscribe to that specific job's events.
export type JobDetailTab = {
  id: number;
  kind: "job-detail";
  title: string;
  jobId: string;
};

export type Tab =
  | TerminalTab
  | EditorTab
  | PreviewTab
  | AiDiffTab
  | JobsTab
  | JobDetailTab
  | AssistantTab
  | PatchesTab;

export type TabPatch = Partial<{
  title: string;
  cwd: string;
  path: string;
  dirty: boolean;
  url: string;
}>;

function basename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : path;
}

function titleFromUrl(url: string): string {
  try {
    const u = new URL(url);
    return u.host || url;
  } catch {
    return url || "preview";
  }
}

const JOBS_TABS_LS_KEY = "codeless-open-job-tabs-v3";

interface PersistedTab {
  kind: "jobs" | "job-detail" | "patches";
  jobId?: string;
  title: string;
}

function readPersistedTabs(): PersistedTab[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(JOBS_TABS_LS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (t): t is PersistedTab =>
        t &&
        typeof t.title === "string" &&
        (t.kind === "jobs" ||
          t.kind === "patches" ||
          (t.kind === "job-detail" && typeof t.jobId === "string")),
    );
  } catch {
    return [];
  }
}

function writePersistedTabs(tabs: Tab[]): void {
  if (typeof window === "undefined") return;
  const persisted: PersistedTab[] = [];
  for (const t of tabs) {
    if (t.kind === "jobs") {
      persisted.push({ kind: "jobs", title: t.title });
    } else if (t.kind === "patches") {
      persisted.push({ kind: "patches", title: t.title });
    } else if (t.kind === "job-detail") {
      persisted.push({ kind: "job-detail", jobId: t.jobId, title: t.title });
    }
  }
  try {
    window.localStorage.setItem(JOBS_TABS_LS_KEY, JSON.stringify(persisted));
  } catch {
    // Quota or disabled — best effort.
  }
}

// Compute the initial state once so `tabs` and `activeId` agree on the
// first render. Doing this inside useTabs's useState initializers would
// duplicate work and risk drift between the two; lifting it out keeps
// the URL → active-tab decision in one place.
function buildInitialState(
  initial: Partial<TerminalTab> | undefined,
): { tabs: Tab[]; activeId: number } {
  const shellTabId = 1;
  const leafId = 2;
  const shellTab: Tab = {
    id: shellTabId,
    kind: "terminal",
    title: initial?.title ?? "shell",
    cwd: initial?.cwd,
    paneTree: { kind: "leaf", id: leafId, cwd: initial?.cwd },
    activeLeafId: leafId,
  };
  const persisted = readPersistedTabs();
  let nextId = 3;
  const restored: Tab[] = [];
  for (const p of persisted) {
    if (p.kind === "jobs") {
      restored.push({ id: nextId++, kind: "jobs", title: p.title });
    } else if (p.kind === "patches") {
      restored.push({ id: nextId++, kind: "patches", title: p.title });
    } else if (p.kind === "job-detail" && p.jobId) {
      restored.push({
        id: nextId++,
        kind: "job-detail",
        title: p.title,
        jobId: p.jobId,
      });
    }
  }
  const tabs = [shellTab, ...restored];

  // Honour the URL when picking the initial active tab so a reload at
  // `/jobs/:id` lands on the job-detail tab, not the shell. The first
  // render then matches the URL and the URL-mirror effect never has to
  // rewrite the path away from what the user reloaded.
  let activeId = shellTabId;
  if (typeof window !== "undefined") {
    const path = window.location.pathname;
    const detailMatch = /^\/jobs\/([^/]+)\/?$/.exec(path);
    if (detailMatch) {
      const jobId = detailMatch[1];
      const t = tabs.find(
        (x): x is JobDetailTab => x.kind === "job-detail" && x.jobId === jobId,
      );
      if (t) activeId = t.id;
    } else if (path === "/jobs" || path.startsWith("/jobs?")) {
      const t = tabs.find((x): x is JobsTab => x.kind === "jobs");
      if (t) activeId = t.id;
    } else if (path === "/patches" || path.startsWith("/patches?")) {
      const existing = tabs.find((x): x is PatchesTab => x.kind === "patches");
      if (existing) {
        activeId = existing.id;
      } else {
        // The patches route is opt-in (no auto-create on first boot),
        // but a deep-link reload to `/patches` should land on the
        // worklist instead of silently falling back to the shell. The
        // tab list above only includes restored tabs; appending here
        // is the cheapest way to honour the URL without threading a
        // "create-on-deeplink" branch through `useTabs`.
        const id = nextId++;
        tabs.push({ id, kind: "patches", title: "Patches" });
        activeId = id;
      }
    }
  }
  return { tabs, activeId };
}

export function useTabs(initial?: Partial<TerminalTab>) {
  const initialState = useRef<{ tabs: Tab[]; activeId: number } | null>(null);
  if (initialState.current === null) {
    initialState.current = buildInitialState(initial);
  }
  const [tabs, setTabsRaw] = useState<Tab[]>(initialState.current.tabs);

  // Every mutation goes through this wrapper so localStorage stays in
  // lock-step with the in-memory tab list. No effects, no closures —
  // the persisted snapshot is computed from the value React just stored.
  const setTabs: typeof setTabsRaw = useCallback((updater) => {
    setTabsRaw((curr) => {
      const next = typeof updater === "function" ? updater(curr) : updater;
      writePersistedTabs(next);
      return next;
    });
  }, []);

  const [activeId, setActiveId] = useState(initialState.current.activeId);
  // nextIdRef starts past any restored tab ids so new tabs never collide.
  const nextIdRef = useRef(
    Math.max(1, ...tabs.map((t) => t.id)) + 1,
  );

  const newTab = useCallback((cwd?: string) => {
    const tabId = nextIdRef.current++;
    const leafId = nextIdRef.current++;
    setTabs((t) => [
      ...t,
      {
        id: tabId,
        kind: "terminal",
        title: "shell",
        cwd,
        paneTree: { kind: "leaf", id: leafId, cwd },
        activeLeafId: leafId,
      },
    ]);
    setActiveId(tabId);
    return tabId;
  }, []);

  /**
   * Opens a file in an editor tab.
   *
   * - `pin = true` (default) — opens or activates a **persistent** tab.
   *   If the path is currently in the preview slot it is promoted in-place.
   *   Use this for programmatic opens (AI diff, New File dialog, etc.).
   * - `pin = false` — VSCode-style **preview** tab. A single shared slot is
   *   reused: if a persistent tab for the path already exists it is activated;
   *   otherwise the current preview slot is replaced with the new path.
   */
  const openFileTab = useCallback((path: string, pin = true) => {
    let targetId: number | null = null;
    setTabs((curr) => {
      if (pin) {
        // Persistent open: find any existing editor tab, pin it if needed.
        const existing = curr.find(
          (t) => t.kind === "editor" && t.path === path,
        );
        if (existing) {
          targetId = existing.id;
          if ((existing as EditorTab).preview) {
            return curr.map((t) =>
              t.id === existing.id ? { ...t, preview: false } : t,
            );
          }
          return curr;
        }
        const id = nextIdRef.current++;
        targetId = id;
        return [
          ...curr,
          {
            id,
            kind: "editor",
            title: basename(path),
            path,
            dirty: false,
            preview: false,
          } satisfies EditorTab,
        ];
      } else {
        // Preview open: persistent tab for this path takes priority.
        const persistent = curr.find(
          (t) => t.kind === "editor" && t.path === path && !(t as EditorTab).preview,
        );
        if (persistent) {
          targetId = persistent.id;
          return curr;
        }
        // Reuse the slot if it already shows the same path.
        const existingPreview = curr.find(
          (t) => t.kind === "editor" && t.path === path && (t as EditorTab).preview,
        );
        if (existingPreview) {
          targetId = existingPreview.id;
          return curr;
        }
        // Replace the current preview slot, or append a new one.
        const previewIdx = curr.findIndex(
          (t) => t.kind === "editor" && (t as EditorTab).preview,
        );
        const id = nextIdRef.current++;
        targetId = id;
        const tab: EditorTab = {
          id,
          kind: "editor",
          title: basename(path),
          path,
          dirty: false,
          preview: true,
        };
        if (previewIdx === -1) return [...curr, tab];
        const next = [...curr];
        next[previewIdx] = tab;
        return next;
      }
    });
    if (targetId !== null) setActiveId(targetId);
    return targetId as number | null;
  }, []);

  /**
   * Promotes a preview tab to a persistent one. Called on double-click of the
   * tab title in the tab bar. Dirty edits also auto-promote (see `updateTab`).
   */
  const pinTab = useCallback((id: number) => {
    setTabs((curr) =>
      curr.map((t) =>
        t.id === id && t.kind === "editor" ? { ...t, preview: false } : t,
      ),
    );
  }, []);

  const openAiDiffTab = useCallback(
    (input: {
      path: string;
      originalContent: string;
      proposedContent: string;
      approvalId: string;
      isNewFile: boolean;
    }) => {
      let targetId: number | null = null;
      setTabs((curr) => {
        const existing = curr.find(
          (t) => t.kind === "ai-diff" && t.approvalId === input.approvalId,
        );
        if (existing) {
          targetId = existing.id;
          return curr;
        }
        const id = nextIdRef.current++;
        targetId = id;
        const title = `${basename(input.path)} (AI diff)`;
        return [
          ...curr,
          {
            id,
            kind: "ai-diff",
            title,
            path: input.path,
            originalContent: input.originalContent,
            proposedContent: input.proposedContent,
            approvalId: input.approvalId,
            status: "pending",
            isNewFile: input.isNewFile,
          },
        ];
      });
      if (targetId !== null) setActiveId(targetId);
      return targetId as number | null;
    },
    [],
  );

  const setAiDiffStatus = useCallback(
    (approvalId: string, status: AiDiffStatus) => {
      setTabs((curr) =>
        curr.map((t) =>
          t.kind === "ai-diff" && t.approvalId === approvalId
            ? { ...t, status }
            : t,
        ),
      );
    },
    [],
  );

  const newPreviewTab = useCallback((url: string) => {
    const id = nextIdRef.current++;
    setTabs((t) => [
      ...t,
      { id, kind: "preview", title: titleFromUrl(url), url },
    ]);
    setActiveId(id);
    return id;
  }, []);

  // Singleton: <JobsDashboard /> renders the global jobs view across all
  // repos — opening a second tab would duplicate identical content. If a
  // jobs tab is already open, focus it instead of appending.
  const newJobsTab = useCallback(() => {
    let targetId = -1;
    setTabs((curr) => {
      const existing = curr.find((t) => t.kind === "jobs");
      if (existing) {
        targetId = existing.id;
        return curr;
      }
      const id = nextIdRef.current++;
      targetId = id;
      return [...curr, { id, kind: "jobs", title: "Jobs" } satisfies JobsTab];
    });
    setActiveId(targetId);
    return targetId;
  }, []);

  // Singleton: <PatchesPage /> renders the cross-workspace patch
  // worklist (Surface C from `DOCS/SCOPE-MUTABLE-UI.md`). One tab is
  // enough; a second would duplicate the worklist view. Focuses the
  // existing tab if present.
  const newPatchesTab = useCallback(() => {
    let targetId = -1;
    setTabs((curr) => {
      const existing = curr.find((t) => t.kind === "patches");
      if (existing) {
        targetId = existing.id;
        return curr;
      }
      const id = nextIdRef.current++;
      targetId = id;
      return [
        ...curr,
        { id, kind: "patches", title: "Patches" } satisfies PatchesTab,
      ];
    });
    setActiveId(targetId);
    return targetId;
  }, []);

  // Per-job tab. Unlike the singleton jobs dashboard, each job-detail
  // tab is keyed by `jobId` — opening the same job twice focuses the
  // existing tab rather than appending a duplicate. The `title` is
  // re-derived from the job by `JobPage` after fetch; the initial
  // value the caller passes is what shows up until that lands.
  // Singleton: the `/assistant` rail is global across threads, so a
  // second tab would duplicate identical content. Mirrors `newJobsTab`.
  const newAssistantTab = useCallback(() => {
    let targetId = -1;
    setTabs((curr) => {
      const existing = curr.find((t) => t.kind === "assistant");
      if (existing) {
        targetId = existing.id;
        return curr;
      }
      const id = nextIdRef.current++;
      targetId = id;
      return [
        ...curr,
        { id, kind: "assistant", title: "Assistant" } satisfies AssistantTab,
      ];
    });
    setActiveId(targetId);
    return targetId;
  }, []);

  const newJobDetailTab = useCallback((jobId: string, initialTitle: string) => {
    let targetId = -1;
    setTabs((curr) => {
      const existing = curr.find(
        (t): t is JobDetailTab => t.kind === "job-detail" && t.jobId === jobId,
      );
      if (existing) {
        targetId = existing.id;
        return curr;
      }
      const id = nextIdRef.current++;
      targetId = id;
      return [
        ...curr,
        {
          id,
          kind: "job-detail",
          title: initialTitle,
          jobId,
        } satisfies JobDetailTab,
      ];
    });
    setActiveId(targetId);
    return targetId;
  }, []);

  const closeTab = useCallback((id: number) => {
    setTabs((curr) => {
      if (curr.length <= 1) return curr;
      const idx = curr.findIndex((t) => t.id === id);
      const next = curr.filter((t) => t.id !== id);
      setActiveId((active) =>
        id === active ? next[Math.max(0, idx - 1)].id : active,
      );
      return next;
    });
  }, []);

  const updateTab = useCallback((id: number, patch: TabPatch) => {
    setTabs((t) =>
      t.map((x) => {
        if (x.id !== id) return x;
        if (x.kind === "terminal") {
          return {
            ...x,
            ...(patch.title !== undefined && { title: patch.title }),
            ...(patch.cwd !== undefined && { cwd: patch.cwd }),
          };
        }
        if (x.kind === "preview") {
          return {
            ...x,
            ...(patch.title !== undefined && { title: patch.title }),
            ...(patch.url !== undefined && {
              url: patch.url,
              title: patch.title ?? titleFromUrl(patch.url),
            }),
          };
        }
        // editor tab: auto-promote from preview the moment the file becomes dirty.
        const autoPin =
          patch.dirty === true && (x as EditorTab).preview
            ? { preview: false }
            : {};
        return {
          ...x,
          ...autoPin,
          ...(patch.title !== undefined && { title: patch.title }),
          ...(patch.dirty !== undefined && { dirty: patch.dirty }),
          ...(patch.path !== undefined && { path: patch.path }),
        };
      }),
    );
  }, []);

  const selectByIndex = useCallback(
    (idx: number) => {
      const t = tabs[idx];
      if (t) setActiveId(t.id);
    },
    [tabs],
  );

  /** Update a leaf's cwd; mirror to the tab's `cwd` when the leaf is active. */
  const setLeafCwd = useCallback((leafId: number, cwd: string) => {
    setTabs((curr) =>
      curr.map((t) => {
        if (t.kind !== "terminal") return t;
        if (!hasLeaf(t.paneTree, leafId)) return t;
        const paneTree = setLeafCwdInTree(t.paneTree, leafId, cwd);
        const isActive = t.activeLeafId === leafId;
        return { ...t, paneTree, ...(isActive && { cwd }) };
      }),
    );
  }, []);

  const focusPane = useCallback((tabId: number, leafId: number) => {
    setTabs((curr) =>
      curr.map((t) => {
        if (t.id !== tabId || t.kind !== "terminal") return t;
        if (!hasLeaf(t.paneTree, leafId)) return t;
        if (t.activeLeafId === leafId) return t;
        return { ...t, activeLeafId: leafId };
      }),
    );
  }, []);

  const focusNextPaneInTab = useCallback(
    (tabId: number, delta: 1 | -1) => {
      setTabs((curr) =>
        curr.map((t) => {
          if (t.id !== tabId || t.kind !== "terminal") return t;
          const next = nextLeafId(t.paneTree, t.activeLeafId, delta);
          if (next === t.activeLeafId) return t;
          return { ...t, activeLeafId: next };
        }),
      );
    },
    [],
  );

  /** Split the active leaf of `tabId` along `dir`. Returns the new leaf id. */
  const splitActivePane = useCallback(
    (tabId: number, dir: SplitDir): number | null => {
      let newLeafId: number | null = null;
      setTabs((curr) =>
        curr.map((t) => {
          if (t.id !== tabId || t.kind !== "terminal") return t;
          if (leafIds(t.paneTree).length >= MAX_PANES_PER_TAB) return t;
          const splitId = nextIdRef.current++;
          const leafId = nextIdRef.current++;
          newLeafId = leafId;
          const paneTree = splitLeaf(
            t.paneTree,
            t.activeLeafId,
            splitId,
            leafId,
            dir,
            t.cwd,
          );
          return { ...t, paneTree, activeLeafId: leafId };
        }),
      );
      return newLeafId;
    },
    [],
  );

  const closePaneByLeaf = useCallback((leafId: number): void => {
    setTabs((curr) => {
      const tab = curr.find(
        (t) => t.kind === "terminal" && hasLeaf(t.paneTree, leafId),
      );
      if (!tab || tab.kind !== "terminal") return curr;
      const newTree = removeLeaf(tab.paneTree, leafId);
      if (newTree === null) {
        if (curr.length <= 1) return curr;
        const idx = curr.findIndex((x) => x.id === tab.id);
        const next = curr.filter((x) => x.id !== tab.id);
        setActiveId((active) =>
          active === tab.id ? next[Math.max(0, idx - 1)].id : active,
        );
        return next;
      }
      const remaining = leafIds(newTree);
      let newActive = tab.activeLeafId;
      if (tab.activeLeafId === leafId) {
        const sib = siblingLeafOf(tab.paneTree, leafId);
        newActive = sib && remaining.includes(sib) ? sib : remaining[0];
      }
      return curr.map((x) =>
        x.id === tab.id
          ? { ...x, paneTree: newTree, activeLeafId: newActive }
          : x,
      );
    });
  }, []);

  const closeActivePane = useCallback((tabId: number): boolean => {
    let closedTab = false;
    setTabs((curr) => {
      const t = curr.find((x) => x.id === tabId);
      if (!t || t.kind !== "terminal") return curr;
      const target = t.activeLeafId;
      const newTree = removeLeaf(t.paneTree, target);
      if (newTree === null) {
        if (curr.length <= 1) return curr;
        const idx = curr.findIndex((x) => x.id === tabId);
        const next = curr.filter((x) => x.id !== tabId);
        setActiveId((active) =>
          active === tabId ? next[Math.max(0, idx - 1)].id : active,
        );
        closedTab = true;
        return next;
      }
      const remaining = leafIds(newTree);
      const sib = siblingLeafOf(t.paneTree, target);
      const newActive =
        sib && remaining.includes(sib) ? sib : remaining[0];
      return curr.map((x) =>
        x.id === tabId
          ? { ...x, paneTree: newTree, activeLeafId: newActive }
          : x,
      );
    });
    return closedTab;
  }, []);

  return {
    tabs,
    activeId,
    setActiveId,
    newTab,
    openFileTab,
    pinTab,
    newPreviewTab,
    newJobsTab,
    newAssistantTab,
    newPatchesTab,
    newJobDetailTab,
    openAiDiffTab,
    setAiDiffStatus,
    closeTab,
    updateTab,
    selectByIndex,
    setLeafCwd,
    focusPane,
    focusNextPaneInTab,
    splitActivePane,
    closeActivePane,
    closePaneByLeaf,
  };
}
