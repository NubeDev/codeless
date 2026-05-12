import { useCallback } from "react";

import type { JobDetailTab, TabPatch } from "@/modules/tabs";
import type { JobId } from "@/lib/rpc";

import { JobPage } from "./JobPage";

interface Props {
  tabs: Array<{ id: number; kind: string; jobId?: string }>;
  activeId: number;
  // Open `absPath` in an editor tab. Provided by `App.tsx` which owns
  // the editor-tab opening machinery; passed down here so each
  // `JobPage` can wire its "Files changed" click handler without
  // knowing about tab plumbing.
  onOpenFile: (absPath: string) => void;
  // Update a tab's title in the strip when `JobPage` resolves a
  // friendlier label than the initial best-guess one the dashboard
  // opened the tab with.
  onUpdateTab: (id: number, patch: TabPatch) => void;
}

// Mirror of `AiDiffStack` / `PreviewStack`: render every job-detail
// tab simultaneously, but only the active one is visible. Keeping
// inactive tabs mounted means switching back is instant and the
// per-job event subscriptions don't tear down on tab change.
export function JobDetailStack({ tabs, activeId, onOpenFile, onUpdateTab }: Props) {
  const handleTitle = useCallback(
    (tabId: number, title: string) => {
      onUpdateTab(tabId, { title });
    },
    [onUpdateTab],
  );

  const jobTabs = tabs.filter(
    (t): t is JobDetailTab => t.kind === "job-detail" && t.jobId !== undefined,
  );

  return (
    <>
      {jobTabs.map((t) => (
        <JobPage
          key={t.id}
          jobId={t.jobId as JobId}
          active={t.id === activeId}
          onOpenFile={onOpenFile}
          onTitleResolved={(title) => handleTitle(t.id, title)}
        />
      ))}
    </>
  );
}
