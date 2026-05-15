import { cn } from "@/lib/utils";
import type { JobDetailTab, TabPatch } from "@/modules/tabs";
import type { JobId } from "@/lib/rpc";

import { JobPage } from "./JobPage";

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
    // Sibling wrappers must overlap rather than stack: the inactive
    // JobPage hides its inner content but its wrapper would still
    // consume `h-full` of the parent in normal flow, pushing every
    // later sibling out of the viewport. Mirror AiDiffStack /
    // EditorStack / PreviewStack: a positioned parent with each child
    // pinned via `absolute inset-0`.
    <div className="relative h-full w-full">
      {jobTabs.map((t) => {
        const visible = t.id === activeId;
        return (
          <div
            key={t.id}
            className={cn(
              "absolute inset-0",
              !visible && "invisible pointer-events-none",
            )}
            aria-hidden={!visible}
          >
            <JobPage jobId={t.jobId as JobId} active={visible} />
          </div>
        );
      })}
    </div>
  );
}

