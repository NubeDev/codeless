import type { JobDetailTab, TabPatch } from "@/modules/tabs";
import type { JobId } from "@/lib/rpc";

import { JobChatPage } from "./JobChatPage";

interface Props {
  tabs: Array<{ id: number; kind: string; jobId?: string }>;
  activeId: number;
  // Open `absPath` in an editor tab. Reserved for when "Files changed"
  // wires back into the editor stack from the chat-first sidebar.
  onOpenFile?: (absPath: string) => void;
  // Update a tab's title. Reserved for the future when JobChatPage
  // surfaces a derived friendly title back to the tab strip.
  onUpdateTab?: (id: number, patch: TabPatch) => void;
  // Open a new job-detail tab for a freshly created job (Re-run from
  // scratch). Reserved for when re-run is wired through the parent
  // tab system rather than navigating the URL.
  onOpenJobTab?: (jobId: JobId, initialTitle: string) => void;
}

// Mirror of `AiDiffStack` / `PreviewStack`: render every job-detail
// tab simultaneously and toggle visibility, so switching back is
// instant and per-job event subscriptions don't tear down.
export function JobDetailStack({ tabs, activeId }: Props) {
  const jobTabs = tabs.filter(
    (t): t is JobDetailTab => t.kind === "job-detail" && t.jobId !== undefined,
  );

  return (
    <>
      {jobTabs.map((t) => (
        <div
          key={t.id}
          className="h-full w-full"
          style={{ display: t.id === activeId ? "block" : "none" }}
        >
          <JobChatPage jobId={t.jobId as JobId} />
        </div>
      ))}
    </>
  );
}

