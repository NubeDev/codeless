import { useCallback, useEffect, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import {
  useEventStream,
  useRpc,
  type EventEnvelope,
  type JobId,
  type PreCheckOutcome,
  type ReviewVerdict,
  type ScopePatchId,
  type StageRollup,
} from "@/lib/rpc";

import { ReviewGatePanel } from "./ReviewGatePanel";
import { StageChat } from "./StageChat";

// ------------------------------------------------------------------ types

type StageStatus = "pending" | "running" | "passed" | "failed";
type TaskStatus = "queued" | "running" | "passed" | "failed";
type VerifyStepStatus = "running" | "passed" | "failed" | "skipped";

interface TaskRow {
  kind: "task";
  taskId: string;
  ordinal: number;
  status: TaskStatus;
}

interface VerifyStepRow {
  kind: "verify-step";
  stepIndex: number;
  name: string;
  status: VerifyStepStatus;
  durationMs: number | null;
  tail: string | null;
  exitCode: number | null;
}

type ChildRow = TaskRow | VerifyStepRow;

interface StageState {
  // null until the list_stages call returns
  rollup: StageRollup | null;
  // Overrides the rollup status as events arrive, so the UI reflects
  // in-flight transitions without waiting for a database write.
  liveStatus: StageStatus | null;
  children: ChildRow[];
  // Most recent REVIEW-gate diagnostics for this stage, populated
  // for stages that emit them. Null when no event has arrived yet
  // (or for non-REVIEW stages, which never emit either).
  precheck: PreCheckOutcome | null;
  verdict: ReviewVerdict | null;
  // Set of patch ids the runtime has emitted a `ScopePatchProposed`
  // event for on this stage. Stored as a set so SSE replays after a
  // reconnect cannot double-count the same proposal.
  patchIds: Set<ScopePatchId>;
}

// ------------------------------------------------------------------ reducer

// Reduce a single event into the children array for this stage.
// Only called when the event's stage_id matches the target; the
// caller filters before passing events here.
function applyEvent(state: StageState, env: EventEnvelope): StageState {
  const e = env.event;

  switch (e.type) {
    case "stage-started":
      return { ...state, liveStatus: "running" };

    case "stage-completed":
      return {
        ...state,
        liveStatus: e.status === "passed" ? "passed" : "failed",
      };

    case "verify-failed":
      return { ...state, liveStatus: "failed" };

    case "task-enqueued": {
      const already = state.children.some(
        (c) => c.kind === "task" && c.taskId === e.task_id,
      );
      if (already) return state;
      const ordinal =
        state.children.filter((c) => c.kind === "task").length + 1;
      const row: TaskRow = {
        kind: "task",
        taskId: e.task_id,
        ordinal,
        status: "queued",
      };
      return { ...state, children: [...state.children, row] };
    }

    case "task-started": {
      const has = state.children.some(
        (c) => c.kind === "task" && c.taskId === e.task_id,
      );
      if (has) {
        return {
          ...state,
          children: state.children.map((c) =>
            c.kind === "task" && c.taskId === e.task_id
              ? { ...c, status: "running" as TaskStatus }
              : c,
          ),
        };
      }
      // Synthesise a row when task-started arrives before task-enqueued.
      const ordinal =
        state.children.filter((c) => c.kind === "task").length + 1;
      return {
        ...state,
        children: [
          ...state.children,
          {
            kind: "task",
            taskId: e.task_id,
            ordinal,
            status: "running" as TaskStatus,
          },
        ],
      };
    }

    case "task-completed": {
      const termStatus: TaskStatus =
        e.status === "completed" ? "passed" : "failed";
      return {
        ...state,
        children: state.children.map((c) =>
          c.kind === "task" && c.taskId === e.task_id
            ? { ...c, status: termStatus }
            : c,
        ),
      };
    }

    case "verify-step-started": {
      const already = state.children.some(
        (c) => c.kind === "verify-step" && c.stepIndex === e.step_index,
      );
      if (already) {
        return {
          ...state,
          children: state.children.map((c) =>
            c.kind === "verify-step" && c.stepIndex === e.step_index
              ? { ...c, status: "running" as VerifyStepStatus }
              : c,
          ),
        };
      }
      const row: VerifyStepRow = {
        kind: "verify-step",
        stepIndex: e.step_index,
        name: e.name,
        status: "running",
        durationMs: null,
        tail: null,
        exitCode: null,
      };
      return { ...state, children: [...state.children, row] };
    }

    case "verify-step-passed":
      return {
        ...state,
        children: state.children.map((c) =>
          c.kind === "verify-step" && c.stepIndex === e.step_index
            ? { ...c, status: "passed" as VerifyStepStatus, durationMs: e.duration_ms }
            : c,
        ),
      };

    case "verify-step-failed": {
      const has = state.children.some(
        (c) => c.kind === "verify-step" && c.stepIndex === e.step_index,
      );
      if (has) {
        return {
          ...state,
          children: state.children.map((c) =>
            c.kind === "verify-step" && c.stepIndex === e.step_index
              ? {
                  ...c,
                  status: "failed" as VerifyStepStatus,
                  exitCode: e.exit_code,
                  tail: e.tail,
                }
              : c,
          ),
        };
      }
      // Synthesise when verify-step-failed arrives without a prior started.
      return {
        ...state,
        children: [
          ...state.children,
          {
            kind: "verify-step",
            stepIndex: e.step_index,
            name: e.name,
            status: "failed" as VerifyStepStatus,
            durationMs: null,
            tail: e.tail,
            exitCode: e.exit_code,
          },
        ],
      };
    }

    case "verify-step-skipped": {
      const already = state.children.some(
        (c) => c.kind === "verify-step" && c.stepIndex === e.step_index,
      );
      if (already) {
        return {
          ...state,
          children: state.children.map((c) =>
            c.kind === "verify-step" && c.stepIndex === e.step_index
              ? { ...c, status: "skipped" as VerifyStepStatus }
              : c,
          ),
        };
      }
      return {
        ...state,
        children: [
          ...state.children,
          {
            kind: "verify-step",
            stepIndex: e.step_index,
            name: e.name,
            status: "skipped" as VerifyStepStatus,
            durationMs: null,
            tail: null,
            exitCode: null,
          },
        ],
      };
    }

    case "review-pre-check":
      return { ...state, precheck: e.outcome };

    case "review-verdict":
      return { ...state, verdict: e.verdict };

    case "scope-patch-proposed": {
      // Dedup on patch_id so an SSE replay cannot inflate the count.
      // The `Set` is cloned (not mutated) so React notices the change.
      if (state.patchIds.has(e.patch_id)) return state;
      const next = new Set(state.patchIds);
      next.add(e.patch_id);
      return { ...state, patchIds: next };
    }

    default:
      return state;
  }
}

// ------------------------------------------------------------------ helpers

function resolvedStatus(s: StageState): StageStatus {
  if (s.liveStatus !== null) return s.liveStatus;
  if (!s.rollup) return "pending";
  const rs = s.rollup.stage.status;
  if (rs === "passed") return "passed";
  if (rs === "failed") return "failed";
  if (rs === "running" || rs === "awaiting-review") return "running";
  return "pending";
}

function glyphFor(status: StageStatus | TaskStatus | VerifyStepStatus): {
  char: string;
  tone: string;
  label: string;
} {
  switch (status) {
    case "passed":
      return {
        char: "✓",
        tone: "text-emerald-600 dark:text-emerald-400",
        label: "passed",
      };
    case "running":
      return { char: "●", tone: "text-blue-500", label: "running" };
    case "failed":
      return { char: "!", tone: "text-destructive", label: "failed" };
    case "queued":
    case "pending":
      return { char: "○", tone: "text-muted-foreground", label: "queued" };
    case "skipped":
      return { char: "—", tone: "text-muted-foreground", label: "skipped" };
    default:
      return { char: "?", tone: "text-muted-foreground", label: "unknown" };
  }
}

// ------------------------------------------------------------------ component

interface Props {
  jobId: JobId;
  stageId: string;
  stageName: string;
  // Called when the stage's chat transitions between idle and streaming.
  // The parent uses this to drive the tab's ● indicator.
  onChatActive?: (active: boolean) => void;
}

// Full detail view for one stage tab. Shows the stage goal (name),
// a compact TICKS strip of each task and verify step, a FAILURE block
// when any verify step failed, three action buttons, and a live chat
// panel wired to the stage's warm session.
//
// Data model: seeds from list_stages (authoritative for completed stages)
// then layers live events on top. The children list (ticks / verify steps)
// is built purely from events because the list_stages rollup does not
// carry per-step rows.
export function StageDetail({ jobId, stageId, stageName, onChatActive }: Props) {
  const rpc = useRpc();

  const [stage, setStage] = useState<StageState>({
    rollup: null,
    liveStatus: null,
    children: [],
    precheck: null,
    verdict: null,
    patchIds: new Set<ScopePatchId>(),
  });
  const [fetchError, setFetchError] = useState<string | null>(null);
  // `feature_flags.scope_patch_handover_round_trip` from `ServerInfo`.
  // Until Step 2 of the scope-mutable-ui ramp lands the handover-schema
  // fix and the runtime flips this to `true`, the `Patches proposed: N`
  // counter row stays omitted (decision OQ#1 — never ship a counter
  // gated on a half-built capability).
  const [patchCounterEnabled, setPatchCounterEnabled] = useState(false);

  // Seed rollup from persisted data on mount / stageId change.
  useEffect(() => {
    let cancelled = false;
    setStage({
      rollup: null,
      liveStatus: null,
      children: [],
      precheck: null,
      verdict: null,
      patchIds: new Set<ScopePatchId>(),
    });
    setFetchError(null);
    rpc
      .call("list_stages", { job_id: jobId })
      .then((res) => {
        if (cancelled) return;
        const found = res.stages.find((s) => s.stage.id === stageId) ?? null;
        if (found) {
          setStage((prev) => ({ ...prev, rollup: found }));
        }
      })
      .catch((err) => {
        if (cancelled) return;
        setFetchError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, jobId, stageId]);

  // Subscribe to live events, filtering to this stage only.
  const onEvent = useCallback(
    (env: EventEnvelope) => {
      const e = env.event;
      const evtStageId =
        env.stage_id ??
        ("stage_id" in e && typeof e.stage_id === "string"
          ? e.stage_id
          : null);
      // task events carry task_id only; the envelope's stage_id column
      // is the authoritative join, so fall through to it.
      if (evtStageId !== stageId) return;
      setStage((prev) => applyEvent(prev, env));
    },
    [stageId],
  );

  useEventStream({ scope: "job", job_id: jobId }, onEvent);

  // Read the patch-counter feature flag once. `serverInfo` is a boot
  // snapshot — the flag never flips at runtime, so a single fetch per
  // mount is enough; cache misses fall through silently to the
  // "counter omitted" default rather than blocking the panel.
  useEffect(() => {
    let cancelled = false;
    rpc
      .serverInfo()
      .then((info) => {
        if (cancelled) return;
        setPatchCounterEnabled(
          info.feature_flags?.scope_patch_handover_round_trip === true,
        );
      })
      .catch(() => {
        // Silent: the panel still renders, just without the counter row.
      });
    return () => {
      cancelled = true;
    };
  }, [rpc]);

  const status = resolvedStatus(stage);
  const rollup = stage.rollup;
  const displayName =
    rollup?.stage.name || stageName || `Stage ${(rollup?.stage.ordinal ?? 0) + 1}`;
  const ordinalLabel =
    rollup
      ? `Stage ${rollup.stage.ordinal + 1}`
      : stageName
        ? `Stage: ${stageName}`
        : "Stage";

  const failedVerifySteps = stage.children.filter(
    (c): c is VerifyStepRow => c.kind === "verify-step" && c.status === "failed",
  );

  const capturedSessionId = rollup?.stage.session_id ?? null;

  // The runtime tags REVIEW stages by prefixing the stored stage
  // name with `REVIEW ` (see `template_runner` `name_for_event`).
  // Falling back to the chat-tab `stageName` covers the brief window
  // before `list_stages` returns; the second condition catches stages
  // that have already emitted a REVIEW event (defensive — the prefix
  // should always be present once `rollup` lands).
  const reviewByName = (rollup?.stage.name ?? stageName ?? "")
    .startsWith("REVIEW ");
  const reviewByEvent = stage.precheck !== null || stage.verdict !== null;
  const isReview = reviewByName || reviewByEvent;

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* Stage detail — scrollable, capped at half the panel height so
          the chat always has room even for stages with long failure output. */}
      <ScrollArea className="max-h-[50%] shrink-0">
        <div className="space-y-5 p-5">
          {/* Header */}
          <div className="flex items-start justify-between gap-3">
            <div>
              <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
                {ordinalLabel}
              </div>
              <h2 className="mt-0.5 text-base font-semibold leading-tight">
                {displayName}
              </h2>
            </div>
            <StatusBadge status={status} />
          </div>

          {fetchError && (
            <div className="text-destructive text-xs">{fetchError}</div>
          )}

          {/* GOAL */}
          <Section label="Goal">
            <p className="text-sm">
              {rollup?.stage.name || stageName || (
                <span className="text-muted-foreground italic">
                  no goal recorded
                </span>
              )}
            </p>
          </Section>

          {/* REVIEW gate panel — Surface A. Summary of the most-recent
              `ReviewPreCheck` and `ReviewVerdict` events for this
              stage; the raw events stay in the timeline (decision
              OQ#6). Patch counter omitted unless the runtime
              advertises the handover round-trip capability. */}
          {isReview && (
            <Section label="Review gate">
              <ReviewGatePanel
                precheck={stage.precheck}
                verdict={stage.verdict}
                patchesProposed={stage.patchIds.size}
                patchCounterEnabled={patchCounterEnabled}
              />
            </Section>
          )}

          {/* TICKS */}
          {stage.children.length > 0 && (
            <Section label="Ticks">
              <TicksStrip children={stage.children} />
            </Section>
          )}

          {/* FAILURE */}
          {status === "failed" && failedVerifySteps.length > 0 && (
            <Section label="Failure">
              <FailureBlock steps={failedVerifySteps} />
            </Section>
          )}

          {/* Action buttons */}
          <ActionBar jobId={jobId} hasWarmSession={capturedSessionId !== null} />
        </div>
      </ScrollArea>

      {/* Live chat — takes the remaining vertical space so the user can
          talk to the agent that ran this stage without leaving the tab. */}
      <div className="min-h-0 flex-1 px-5 pb-5">
        <StageChat
          jobId={jobId}
          stageId={stageId}
          stageName={displayName}
          capturedSessionId={capturedSessionId}
          onChatActive={onChatActive}
        />
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ sub-components

function Section({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <div className="text-muted-foreground text-[10px] font-semibold uppercase tracking-wider">
        {label}
      </div>
      {children}
    </div>
  );
}

// Compact horizontal strip showing one glyph+label chip per child row.
// Tasks render as "tick N", verify steps render by their step name.
function TicksStrip({ children }: { children: ChildRow[] }) {
  return (
    <div className="flex flex-wrap gap-x-3 gap-y-1.5">
      {children.map((c, i) => {
        const status = c.status;
        const g = glyphFor(status);
        const label =
          c.kind === "task" ? `tick ${c.ordinal}` : c.name || "test";
        return (
          <span
            key={c.kind === "task" ? `task-${c.taskId}` : `vs-${c.stepIndex}-${i}`}
            className="flex items-baseline gap-1 text-xs"
            title={g.label}
          >
            <span className={cn("font-mono", g.tone)}>{g.char}</span>
            <span
              className={cn(
                status === "failed" ? g.tone : "text-muted-foreground",
              )}
            >
              {label}
            </span>
          </span>
        );
      })}
    </div>
  );
}

// One card per failed verify step, showing the step name, exit code,
// and the captured tail output. Only rendered when the stage failed
// and at least one verify step has a failed status.
function FailureBlock({ steps }: { steps: VerifyStepRow[] }) {
  return (
    <div className="space-y-3">
      {steps.map((step) => (
        <div key={step.stepIndex} className="space-y-1.5">
          <div className="flex items-baseline gap-2 text-xs">
            <span className="text-destructive font-mono">!</span>
            <span className="font-medium">{step.name || "verify"}</span>
            {step.exitCode !== null && (
              <span className="text-muted-foreground">
                exit {step.exitCode}
              </span>
            )}
          </div>
          {step.tail !== null && step.tail.length > 0 && (
            <pre className="bg-muted/40 text-muted-foreground overflow-x-auto rounded px-3 py-2 text-[10px] leading-snug">
              {step.tail}
            </pre>
          )}
        </div>
      ))}
    </div>
  );
}

// The three action buttons from JOB-UI.md "The Stage-N tab".
//
//   rerun now            — resumes the job via the captured session_id
//                          so the runner passes --continue on next tick.
//   new session+handover — creates a fresh job copy seeded from handover.md.
//                          Prompts once when the stage has a warm session so
//                          the user knows the session will be discarded.
//   stop                 — terminates the job; stage stays at failed.
function ActionBar({
  jobId,
  hasWarmSession,
}: {
  jobId: JobId;
  hasWarmSession: boolean;
}) {
  const rpc = useRpc();
  const [busy, setBusy] = useState<"rerun" | "new-session" | "stop" | null>(
    null,
  );
  const [err, setErr] = useState<string | null>(null);
  // True while waiting for the user to confirm discarding the warm session.
  // Resets to false once the user confirms or cancels.
  const [confirmDiscard, setConfirmDiscard] = useState(false);

  const rerunNow = async () => {
    setBusy("rerun");
    setErr(null);
    try {
      await rpc.call("resume_job", { job_id: jobId });
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const newSession = async () => {
    setBusy("new-session");
    setErr(null);
    setConfirmDiscard(false);
    try {
      await rpc.call("rerun_job", { source_job_id: jobId });
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const stopJob = async () => {
    setBusy("stop");
    setErr(null);
    try {
      await rpc.call("stop_job", { job_id: jobId });
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-1.5">
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="default"
          className="h-7 px-3 text-xs"
          onClick={() => void rerunNow()}
          disabled={busy !== null}
        >
          {busy === "rerun" ? "queuing…" : "rerun now"}
        </Button>
        <Button
          size="sm"
          variant="outline"
          className="h-7 px-3 text-xs"
          onClick={() => {
            if (hasWarmSession && !confirmDiscard) {
              setConfirmDiscard(true);
            } else {
              void newSession();
            }
          }}
          disabled={busy !== null}
        >
          {busy === "new-session" ? "creating…" : "new session + handover"}
        </Button>
        <Button
          size="sm"
          variant="outline"
          className="h-7 px-3 text-xs text-destructive hover:text-destructive"
          onClick={() => void stopJob()}
          disabled={busy !== null}
        >
          {busy === "stop" ? "stopping…" : "stop"}
        </Button>
      </div>
      {confirmDiscard && (
        <div className="flex items-center gap-2 text-[11px]">
          <span className="text-muted-foreground">
            this will discard the current session — continue?
          </span>
          <button
            className="text-destructive underline"
            onClick={() => void newSession()}
          >
            yes
          </button>
          <button
            className="text-muted-foreground underline"
            onClick={() => setConfirmDiscard(false)}
          >
            cancel
          </button>
        </div>
      )}
      {err !== null && (
        <div className="text-destructive text-[11px]">{err}</div>
      )}
    </div>
  );
}

function StatusBadge({ status }: { status: StageStatus }) {
  const tone =
    status === "passed"
      ? "border-emerald-500/40 text-emerald-600 dark:text-emerald-400"
      : status === "failed"
        ? "border-destructive/40 text-destructive"
        : status === "running"
          ? "border-blue-500/40 text-blue-500"
          : "border-border text-muted-foreground";
  return (
    <Badge variant="outline" className={cn("shrink-0 text-[10px]", tone)}>
      {status}
    </Badge>
  );
}
