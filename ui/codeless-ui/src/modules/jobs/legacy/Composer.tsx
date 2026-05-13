// State-driven action surface pinned to the bottom of the
// ConversationPane — the chat-as-control idea from JOBS-UX.md Phase 3.
// What the primary button does depends entirely on the job's current
// status; the position of the button is fixed so muscle memory holds.
//
// This slice is "controls only": the textarea is here for visual
// continuity but does not yet send. Sending while running depends on
// A2 (`add_job_note` + folding), which is the next runtime change
// after A0 (intra-stage resume). The button row is the load-bearing
// part — it makes the composer the canonical place for run / stop /
// resume / re-run regardless of which section the user is in.

import { useState } from "react";

import { Button } from "@/components/ui/button";
import { navigate } from "@/lib/route";
import { useRpc, type Job, type JobId } from "@/lib/rpc";

interface Props {
  job: Job;
  refetchJob: () => void;
  // Re-run mints a new job; the parent decides whether to open it as
  // a fresh tab (dashboard) or navigate the current tab. `null` means
  // "navigate this tab" so the composer doesn't have to know.
  onOpenJobTab?: (jobId: JobId, initialTitle: string) => void;
}

type Busy = "start" | "stop" | "resume" | "rerun" | null;

// Pre-canned cap bumps for the most common resume case: a cost-cap
// fired and the user wants another N dollars on the clock. The
// custom path (an arbitrary number) is intentionally not in this
// slice — keep the UX one click for the dominant case, add a
// "custom…" affordance in 3c when the visual real-estate budget
// for it is real.
const COST_BUMP_PRESETS_CENTS = [500, 1000, 2500, 5000];

export function Composer({ job, refetchJob, onOpenJobTab }: Props) {
  const rpc = useRpc();
  const [busy, setBusy] = useState<Busy>(null);
  const [err, setErr] = useState<string | null>(null);
  const [showResumeForm, setShowResumeForm] = useState(false);

  const isCostCapped = job.stop_reason === "cost-cap";
  const isWallClockCapped = job.stop_reason === "wall-clock";
  const isResumable = job.status === "stopped" || job.status === "failed";

  // The textarea always renders so the composer's shape stays
  // stable across status transitions. Real send wiring lands in
  // Slice 3b once A2 (`add_job_note`) is real; today it's a
  // placeholder that surfaces what *will* happen when the user
  // hits send in each state.
  const [draft, setDraft] = useState("");
  const sendPlaceholder = sendIntent(job);

  const action = async (kind: Exclude<Busy, null>, fn: () => Promise<unknown>) => {
    setBusy(kind);
    setErr(null);
    try {
      await fn();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const start = () =>
    action("start", async () => {
      await rpc.call("start_job", { job_id: job.id });
      refetchJob();
    });

  const stop = () =>
    action("stop", async () => {
      await rpc.call("stop_job", { job_id: job.id });
      refetchJob();
    });

  const resume = (costBump: number | null, wallBump: number | null) =>
    action("resume", async () => {
      await rpc.call("resume_job", {
        job_id: job.id,
        additional_cost_cap_cents: costBump,
        additional_wall_clock_cap_ms: wallBump,
      });
      setShowResumeForm(false);
      refetchJob();
    });

  const rerun = () =>
    action("rerun", async () => {
      const fresh = await rpc.call("rerun_job", { source_job_id: job.id });
      const title = `Job ${fresh.id.slice(-6).toUpperCase()}`;
      if (onOpenJobTab) onOpenJobTab(fresh.id, title);
      else navigate(`/jobs/${fresh.id}`);
    });

  return (
    <div className="border-border/50 shrink-0 border-t">
      {err && (
        <div className="text-destructive border-border/40 border-b px-4 py-1 text-xs">
          {err}
        </div>
      )}
      {showResumeForm && isCostCapped && (
        <CapBumpRow
          status={job}
          onCancel={() => setShowResumeForm(false)}
          onResume={(c, w) => resume(c, w)}
          busy={busy === "resume"}
        />
      )}
      <div className="mx-auto flex max-w-[820px] flex-col gap-2 px-4 py-2.5">
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={sendPlaceholder}
          rows={2}
          className="border-border/50 bg-background min-h-[44px] resize-y rounded-md border px-2.5 py-1.5 text-[12px] focus:outline-none focus-visible:ring-1 focus-visible:ring-blue-500/40"
        />
        <div className="flex items-center justify-between gap-2">
          <div className="text-muted-foreground text-[10px]">
            <SendHint job={job} draftLength={draft.length} />
          </div>
          <div className="flex items-center gap-2">
            <PrimaryActions
              job={job}
              busy={busy}
              isResumable={isResumable}
              isCostCapped={isCostCapped}
              isWallClockCapped={isWallClockCapped}
              onStart={start}
              onStop={stop}
              onRerun={rerun}
              onOpenResumeForm={() => setShowResumeForm(true)}
              onResumeUnchanged={() => resume(null, null)}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

// The primary-button strip. State-driven; one button is highlighted
// (the "do the thing the user is most likely to want next"), the
// rest are secondary. Keeps the strip short — no more than three
// buttons in any state — so the user's eye lands on the right one
// without having to read each label.
function PrimaryActions({
  job,
  busy,
  isResumable,
  isCostCapped,
  isWallClockCapped,
  onStart,
  onStop,
  onRerun,
  onOpenResumeForm,
  onResumeUnchanged,
}: {
  job: Job;
  busy: Busy;
  isResumable: boolean;
  isCostCapped: boolean;
  isWallClockCapped: boolean;
  onStart: () => void;
  onStop: () => void;
  onRerun: () => void;
  onOpenResumeForm: () => void;
  onResumeUnchanged: () => void;
}) {
  const status = job.status;
  const disabled = busy !== null;

  if (status === "draft") {
    return (
      <Button
        size="sm"
        onClick={onStart}
        disabled={disabled}
        className="bg-blue-600 hover:bg-blue-700 text-white"
      >
        {busy === "start" ? "starting…" : "run ▶"}
      </Button>
    );
  }

  if (status === "queued") {
    return (
      <Button
        size="sm"
        variant="outline"
        onClick={onStop}
        disabled={disabled}
      >
        {busy === "stop" ? "stopping…" : "cancel"}
      </Button>
    );
  }

  if (status === "running" || status === "awaiting-review") {
    return (
      <Button
        size="sm"
        variant="outline"
        onClick={onStop}
        disabled={disabled}
      >
        {busy === "stop" ? "stopping…" : "stop ■"}
      </Button>
    );
  }

  // Terminal: stopped / failed / completed.
  return (
    <>
      {isResumable && (isCostCapped || isWallClockCapped) && (
        <Button
          size="sm"
          onClick={onOpenResumeForm}
          disabled={disabled}
          className="bg-emerald-600 hover:bg-emerald-700 text-white"
          title={
            isCostCapped
              ? "Resume from the captured session id, with a higher cost cap."
              : "Resume from the captured session id, with a higher wall-clock budget."
          }
        >
          {busy === "resume" ? "resuming…" : "resume ▶ …"}
        </Button>
      )}
      {isResumable && !(isCostCapped || isWallClockCapped) && (
        <Button
          size="sm"
          onClick={onResumeUnchanged}
          disabled={disabled}
          className="bg-emerald-600 hover:bg-emerald-700 text-white"
          title="Resume with the same caps; the captured session id continues the same claude conversation."
        >
          {busy === "resume" ? "resuming…" : "resume ▶"}
        </Button>
      )}
      <Button
        size="sm"
        variant="outline"
        onClick={onRerun}
        disabled={disabled}
        title="Clone the spec into a fresh job. Doesn't continue the previous session."
      >
        {busy === "rerun" ? "queuing…" : "re-run"}
      </Button>
    </>
  );
}

// Inline cap-bump form for cost-cap pauses. The dominant resume
// case: a cap fired, the user wants to add $5/$10/$25 and keep
// going. The presets cover ~90% of the use case; an arbitrary
// custom-amount field is a Slice 3c addition once we know which
// number people actually reach for.
function CapBumpRow({
  status,
  onCancel,
  onResume,
  busy,
}: {
  status: Job;
  onCancel: () => void;
  onResume: (cost: number | null, wall: number | null) => void;
  busy: boolean;
}) {
  const currentCap = formatCents(status.cost_cap_cents);
  const spent = formatCents(status.cost_cents);
  return (
    <div className="border-border/40 bg-muted/20 border-b px-4 py-2">
      <div className="mx-auto flex max-w-[820px] flex-wrap items-center gap-2 text-[11px]">
        <span className="text-muted-foreground">
          spent {spent} of {currentCap} cap. Add:
        </span>
        {COST_BUMP_PRESETS_CENTS.map((cents) => (
          <Button
            key={cents}
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => onResume(cents, null)}
            className="h-7 px-2 text-[11px]"
          >
            +{formatCents(cents)}
          </Button>
        ))}
        <span className="text-muted-foreground ml-1">or</span>
        <Button
          size="sm"
          variant="ghost"
          disabled={busy}
          onClick={() => onResume(null, null)}
          className="h-7 px-2 text-[11px]"
          title="Resume without raising the cap. The job will trip the same cap again unless something has changed."
        >
          resume unchanged
        </Button>
        <span className="ml-auto" />
        <Button
          size="sm"
          variant="ghost"
          disabled={busy}
          onClick={onCancel}
          className="h-7 px-2 text-[11px]"
        >
          cancel
        </Button>
      </div>
    </div>
  );
}

// Placeholder + help-line copy. The textarea is a stub for Slice
// 3b — telling the user *now* what the eventual send will do
// keeps the UI honest while the wiring is incomplete.
function sendIntent(job: Job): string {
  switch (job.status) {
    case "draft":
      return "edit the spec first, then click run.";
    case "queued":
      return "waiting in the queue…";
    case "running":
    case "awaiting-review":
      return "ask a question — queued for the next session (A2).";
    case "stopped":
    case "failed":
      return "describe what to change, then resume.";
    case "completed":
      return "re-run with a follow-up message.";
  }
}

function SendHint({ job, draftLength }: { job: Job; draftLength: number }) {
  if (draftLength === 0) return null;
  if (job.status === "running" || job.status === "awaiting-review") {
    return (
      <span>
        send queues your message for the next session (needs A2 to land
        live).
      </span>
    );
  }
  if (job.status === "stopped" || job.status === "failed") {
    return <span>send folds your message into the resume prompt.</span>;
  }
  if (job.status === "completed") {
    return <span>send opens a re-run with this as the feedback note.</span>;
  }
  return null;
}

function formatCents(c: number): string {
  return `$${(c / 100).toFixed(2)}`;
}
