import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Streamdown } from "streamdown";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import {
  useEventStream,
  useRpc,
  type EventEnvelope,
  type Job,
  type JobDiffResult,
  type JobId,
  type JobStatus,
  type StageRollup,
} from "@/lib/rpc";

import { setGlobalDocs } from "./spec/mutateTemplate";

type JobDiffFile = JobDiffResult["files"][number];

import { summariseToolArgs } from "./eventFormat";

// The RunPane is the canonical "is it running, can I make it run"
// surface. Identity + cost stay in the page header; here is where the
// user looks at lifecycle, presses [run] / [stop] / [re-run], and
// watches stages produce tool calls + assistant prose live.
//
// Lifecycle = vertical Draft → Queued → Running → Done timeline. Each
// node carries a tone (active = primary, past = success/error tone of
// the terminal status, future = muted) and an animated connector
// that draws in as the job advances. Stages nest under Running with
// status pill, duration, cost, recent tool-call ribbon, and an
// assistant message bubble fed from streaming ai-token deltas.

type Phase = "draft" | "queued" | "running" | "terminal";

interface PhaseSpec {
  id: Phase;
  label: string;
}

const PHASES: PhaseSpec[] = [
  { id: "draft", label: "Draft" },
  { id: "queued", label: "Queued" },
  { id: "running", label: "Running" },
  { id: "terminal", label: "Done" },
];

function phaseOf(status: JobStatus): Phase {
  switch (status) {
    case "draft":
      return "draft";
    case "queued":
      return "queued";
    case "running":
    case "awaiting-review":
      return "running";
    case "completed":
    case "failed":
    case "stopped":
      return "terminal";
  }
}

function phaseIndex(p: Phase): number {
  return PHASES.findIndex((s) => s.id === p);
}

// Terminal-state tone — only used once the job reaches `terminal`. We
// re-use this on the final node so success/failure read at a glance,
// instead of every job ending on the same grey checkmark.
function terminalTone(status: JobStatus): {
  ring: string;
  dot: string;
  text: string;
  bg: string;
} {
  switch (status) {
    case "completed":
      return {
        ring: "border-emerald-500",
        dot: "bg-emerald-500",
        text: "text-emerald-600 dark:text-emerald-400",
        bg: "bg-emerald-500/10",
      };
    case "failed":
      return {
        ring: "border-red-500",
        dot: "bg-red-500",
        text: "text-red-600 dark:text-red-400",
        bg: "bg-red-500/10",
      };
    case "stopped":
      return {
        ring: "border-zinc-500",
        dot: "bg-zinc-500",
        text: "text-zinc-600 dark:text-zinc-400",
        bg: "bg-zinc-500/10",
      };
    default:
      return {
        ring: "border-blue-500",
        dot: "bg-blue-500",
        text: "text-blue-600 dark:text-blue-400",
        bg: "bg-blue-500/10",
      };
  }
}

interface Props {
  job: Job;
  refetchJob: () => void;
  onOpenJobTab?: (jobId: JobId, initialTitle: string) => void;
  // Jump to the SPEC pane so the user can edit template.yaml / docs.
  // Called from the "no stages defined" / "add stages" CTAs in this
  // pane.
  onEditSpec?: () => void;
}

export function RunPane({ job, refetchJob, onOpenJobTab, onEditSpec }: Props) {
  const current = phaseOf(job.status);
  const currentIdx = phaseIndex(current);
  const tone = terminalTone(job.status);
  // Chat needs a session that can run against a worktree. Hide it
  // for pre-run phases (draft/queued) where there's nothing to ask
  // about and `agent_chat` would have no cwd. Surface it as soon as
  // the runner takes over.
  const showChat = current === "running" || current === "terminal";

  return (
    <ScrollArea className="h-full">
      <div className="space-y-4 p-4 lg:p-6">
        <RunHeader job={job} onEditSpec={onEditSpec} />

        <PhaseStepper currentIdx={currentIdx} job={job} tone={tone} />

        <div
          className={cn(
            "grid gap-4",
            showChat && "lg:grid-cols-[minmax(0,1fr)_minmax(360px,420px)]",
          )}
        >
          <div className="min-w-0 space-y-3">
            <PhaseBody
              phase={PHASES[currentIdx]}
              job={job}
              isActive
              isPast={false}
              refetchJob={refetchJob}
              onOpenJobTab={onOpenJobTab}
            />
          </div>
          {showChat && (
            <aside className="min-w-0 lg:sticky lg:top-4 lg:self-start">
              <JobChat job={job} onOpenJobTab={onOpenJobTab} />
            </aside>
          )}
        </div>
      </div>
    </ScrollArea>
  );
}

// Compact orientation strip used by the chat-first job page: the same
// goal/runner/cost/wallclock card and 4-node lifecycle stepper
// `RunPane` shows, but standalone so it can sit above the chat
// without dragging the rest of the run surface in.
export function RunOverview({
  job,
  onEditSpec,
}: {
  job: Job;
  onEditSpec?: () => void;
}) {
  const current = phaseOf(job.status);
  const currentIdx = phaseIndex(current);
  const tone = terminalTone(job.status);
  return (
    <div className="space-y-4">
      <RunHeader job={job} onEditSpec={onEditSpec} />
      <PhaseStepper currentIdx={currentIdx} job={job} tone={tone} />
    </div>
  );
}

// One-line strip for the chat-page header. Phase dots + runner +
// model + cost + time, sized to sit next to the title without
// forcing a second row. Click any segment to jump to the matching
// sidebar tab / spec view.
export function RunStrip({
  job,
  onEditSpec,
}: {
  job: Job;
  onEditSpec?: () => void;
}) {
  const currentIdx = phaseIndex(phaseOf(job.status));
  const tone = terminalTone(job.status);
  const costPct =
    job.cost_cap_cents > 0
      ? Math.min(100, (job.cost_cents / job.cost_cap_cents) * 100)
      : 0;
  const wallMs =
    job.started_at && job.ended_at
      ? new Date(job.ended_at).getTime() - new Date(job.started_at).getTime()
      : job.started_at
        ? Date.now() - new Date(job.started_at).getTime()
        : 0;
  const wallPct =
    job.wall_clock_cap_ms > 0
      ? Math.min(100, (wallMs / job.wall_clock_cap_ms) * 100)
      : 0;
  return (
    <div className="text-muted-foreground flex items-center gap-3 text-[11px]">
      <ol className="flex items-center gap-1" aria-label="lifecycle">
        {PHASES.map((p, i) => {
          const isPast = i < currentIdx;
          const isActive = i === currentIdx;
          const isTerm = p.id === "terminal";
          const reached = isPast || isActive;
          const dotClass = isTerm && reached
            ? tone.dot
            : reached
              ? "bg-blue-500"
              : "bg-muted-foreground/30";
          return (
            <li key={p.id} className="flex items-center gap-1" title={p.label}>
              <span className={cn("h-1.5 w-1.5 rounded-full", dotClass)} />
              {i < PHASES.length - 1 && (
                <span
                  className={cn(
                    "h-px w-3",
                    isPast ? (isTerm ? tone.dot : "bg-blue-500") : "bg-border",
                  )}
                />
              )}
            </li>
          );
        })}
      </ol>
      <span className="truncate">
        <span className="text-foreground font-medium">{job.runner}</span>
        {job.model && <span className="ml-1">· {job.model}</span>}
      </span>
      {job.cost_cap_cents > 0 && (
        <span className="flex items-center gap-1" title={`cost $${(job.cost_cents / 100).toFixed(2)} of $${(job.cost_cap_cents / 100).toFixed(2)}`}>
          <span className="bg-border relative h-1 w-10 overflow-hidden rounded-full">
            <span
              className={cn(
                "absolute inset-y-0 left-0",
                costPct >= 90 ? "bg-red-500" : "bg-blue-500",
              )}
              style={{ width: `${costPct}%` }}
            />
          </span>
          <span className="font-mono">${(job.cost_cents / 100).toFixed(2)}</span>
        </span>
      )}
      {job.wall_clock_cap_ms > 0 && (
        <span className="flex items-center gap-1" title={`time ${formatMs(wallMs)} of ${formatMs(job.wall_clock_cap_ms)}`}>
          <span className="bg-border relative h-1 w-10 overflow-hidden rounded-full">
            <span
              className={cn(
                "absolute inset-y-0 left-0",
                wallPct >= 90 ? "bg-red-500" : "bg-blue-500",
              )}
              style={{ width: `${wallPct}%` }}
            />
          </span>
          <span className="font-mono">{formatMs(wallMs)}</span>
        </span>
      )}
      {onEditSpec && (
        <button
          type="button"
          onClick={onEditSpec}
          className="hover:text-foreground underline"
          title="Edit template.yaml in Spec"
        >
          spec
        </button>
      )}
    </div>
  );
}

function formatMs(ms: number): string {
  if (ms < 60_000) return `${Math.round(ms / 1000)}s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.round((ms % 60_000) / 1000);
  return s === 0 ? `${m}m` : `${m}m${s}s`;
}


// Compact horizontal stepper. Replaces the old vertical lifecycle
// timeline (4 nodes \u00d7 ~80px of vertical space). Past phases collapse
// to a small filled dot; the active phase pulses; future phases are
// muted. Labels sit underneath. The active phase's body renders
// below this strip in the main RunPane layout, so this surface is
// orientation-only \u2014 you don't have to scroll to know where the job
// is in its lifecycle.
function PhaseStepper({
  currentIdx,
  job,
  tone,
}: {
  currentIdx: number;
  job: Job;
  tone: ReturnType<typeof terminalTone>;
}) {
  return (
    <ol className="flex items-center gap-0">
      {PHASES.map((p, i) => {
        const isPast = i < currentIdx;
        const isActive = i === currentIdx;
        const isTerminalNode = p.id === "terminal";
        const reached = isPast || isActive;
        const isLast = i === PHASES.length - 1;
        return (
          <li key={p.id} className="flex flex-1 items-center last:flex-none">
            <div className="flex flex-col items-center gap-1">
              <div className="relative">
                <motion.div
                  initial={false}
                  animate={{ scale: isActive ? 1.05 : 1 }}
                  transition={{ duration: 0.3 }}
                  className={cn(
                    "h-3 w-3 rounded-full border-2 transition-colors",
                    isTerminalNode && reached
                      ? cn(tone.ring, tone.dot)
                      : reached
                        ? "border-blue-500 bg-blue-500"
                        : "border-muted-foreground/30 bg-background",
                  )}
                />
                {isActive && !isTerminalNode && (
                  <motion.div
                    aria-hidden
                    initial={{ opacity: 0.5, scale: 1 }}
                    animate={{ opacity: 0, scale: 2.4 }}
                    transition={{ duration: 1.6, repeat: Infinity, ease: "easeOut" }}
                    className="bg-blue-500 absolute left-0 top-0 h-3 w-3 rounded-full"
                  />
                )}
              </div>
              <span
                className={cn(
                  "text-[11px] font-medium tracking-tight",
                  isActive
                    ? isTerminalNode
                      ? tone.text
                      : "text-foreground"
                    : reached
                      ? "text-foreground/70"
                      : "text-muted-foreground",
                )}
              >
                {isTerminalNode && reached && job.status !== "completed"
                  ? job.status
                  : p.label}
              </span>
            </div>
            {!isLast && (
              <div className="bg-border mx-2 h-px flex-1 overflow-hidden">
                <motion.div
                  initial={{ scaleX: 0 }}
                  animate={{ scaleX: isPast ? 1 : 0 }}
                  transition={{ duration: 0.5, ease: "easeOut" }}
                  style={{ transformOrigin: "left" }}
                  className={cn(
                    "h-full",
                    PHASES[currentIdx].id === "terminal"
                      ? tone.dot
                      : "bg-blue-500",
                  )}
                />
              </div>
            )}
          </li>
        );
      })}
    </ol>
  );
}

// Compact orientation strip above the timeline: prompt summary,
// runner config chips, cost + wall-clock progress bars. Lets the user
// see "what's this job, how much budget is left" without scrolling
// out to the page header.
function RunHeader({
  job,
  onEditSpec,
}: {
  job: Job;
  onEditSpec?: () => void;
}) {
  const promptLine = (job.prompt ?? "").split("\n")[0]?.trim() || null;
  const tplStages = templateStageTitles(job.template_yaml);
  const goal = templateGoal(job.template_yaml);
  return (
    <div className="border-border/60 bg-card/40 rounded-lg border p-4">
      <div className="space-y-3">
        {goal ? (
          <div>
            <div className="text-muted-foreground mb-1 text-[10px] uppercase tracking-wide">
              goal
            </div>
            <p className="text-sm leading-snug line-clamp-2">{goal}</p>
          </div>
        ) : promptLine ? (
          <div>
            <div className="text-muted-foreground mb-1 text-[10px] uppercase tracking-wide">
              prompt
            </div>
            <p className="text-sm leading-snug line-clamp-2">{promptLine}</p>
          </div>
        ) : (
          <p className="text-muted-foreground text-sm italic">
            no prompt set — open Spec → Files to give the runner something to do
          </p>
        )}

        <div className="flex flex-wrap items-center gap-1.5">
          <RunnerChip label="runner" value={job.runner} accent />
          {job.model && <RunnerChip label="model" value={job.model} />}
          {job.effort && <RunnerChip label="effort" value={job.effort} />}
          {job.permission_mode && (
            <RunnerChip label="perm" value={job.permission_mode} />
          )}
          {tplStages.length > 0 && onEditSpec && (
            <button
              type="button"
              onClick={onEditSpec}
              className="text-muted-foreground hover:text-foreground ml-auto text-[11px] underline"
              title="Edit template.yaml in the Spec pane"
            >
              {tplStages.length} stages · edit
            </button>
          )}
        </div>

        {tplStages.length === 0 && (
          <NoStagesHint onEditSpec={onEditSpec} />
        )}

        <div className="grid grid-cols-2 gap-3">
          <CapBar
            label="cost"
            value={job.cost_cents}
            cap={job.cost_cap_cents}
            format={(c) => `$${(c / 100).toFixed(2)}`}
          />
          <WallClockBar job={job} />
        </div>
      </div>
    </div>
  );
}

function NoStagesHint({ onEditSpec }: { onEditSpec?: () => void }) {
  return (
    <p className="text-muted-foreground text-[11px] italic">
      no stages in template.yaml — re-runs of this job will do nothing.
      {onEditSpec && (
        <>
          {" "}
          <button
            type="button"
            onClick={onEditSpec}
            className="hover:text-foreground underline"
          >
            open Spec → Files
          </button>
          {" "}and add `stages:` entries.
        </>
      )}
    </p>
  );
}

// Pull the goal: "..." line and the stages: list from template.yaml
// without bringing in a YAML parser. The structure codeless emits is
// known and small, so two focused regexes are enough; non-conforming
// YAML drops back to empty results and the prompt fallback kicks in.
function templateGoal(tpl: string | null): string | null {
  if (!tpl) return null;
  const m = /^goal:\s*(?:"([^"]*)"|'([^']*)'|(.+))$/m.exec(tpl);
  if (!m) return null;
  return (m[1] ?? m[2] ?? m[3] ?? "").trim() || null;
}

function templateStageTitles(tpl: string | null): string[] {
  if (!tpl) return [];
  const start = tpl.search(/^stages\s*:\s*$/m);
  if (start < 0) return [];
  const body = tpl.slice(start).split("\n").slice(1);
  const out: string[] = [];
  for (const raw of body) {
    if (/^\S/.test(raw) && raw.trim() !== "") break;
    const m = /^\s*-\s*(?:"([^"]*)"|'([^']*)'|(.+?))\s*$/.exec(raw);
    if (m) {
      const title = (m[1] ?? m[2] ?? m[3] ?? "").trim();
      if (title) out.push(title);
      continue;
    }
    // Nested mapping form: `- name: "..."` lines also count; pick up
    // the name and continue scanning siblings.
    const nm = /^\s*-?\s*name\s*:\s*(?:"([^"]*)"|'([^']*)'|(.+))$/.exec(raw);
    if (nm) {
      const title = (nm[1] ?? nm[2] ?? nm[3] ?? "").trim();
      if (title) out.push(title);
    }
  }
  return out;
}

function RunnerChip({
  label,
  value,
  accent,
}: {
  label: string;
  value: string;
  accent?: boolean;
}) {
  return (
    <div
      className={cn(
        "border-border/60 inline-flex items-baseline gap-1 rounded-md border px-1.5 py-0.5",
        accent && "border-blue-500/40 bg-blue-500/10",
      )}
    >
      <span className="text-muted-foreground text-[9px] uppercase tracking-wide">
        {label}
      </span>
      <span
        className={cn(
          "font-mono text-[11px]",
          accent && "text-blue-700 dark:text-blue-300",
        )}
      >
        {value}
      </span>
    </div>
  );
}

function CapBar({
  label,
  value,
  cap,
  format,
}: {
  label: string;
  value: number;
  cap: number;
  format: (n: number) => string;
}) {
  const pct = cap > 0 ? Math.min(100, (value / cap) * 100) : 0;
  const hot = pct > 80;
  return (
    <div>
      <div className="mb-0.5 flex items-baseline justify-between">
        <span className="text-muted-foreground text-[10px] uppercase tracking-wide">
          {label}
        </span>
        <span className="font-mono text-[10px]">
          <span className={cn(hot && "text-amber-600 dark:text-amber-400")}>
            {format(value)}
          </span>
          <span className="text-muted-foreground"> / {format(cap)}</span>
        </span>
      </div>
      <div className="bg-muted/60 h-1.5 overflow-hidden rounded-full">
        <motion.div
          initial={{ width: 0 }}
          animate={{ width: `${pct}%` }}
          transition={{ duration: 0.5, ease: "easeOut" }}
          className={cn(
            "h-full rounded-full",
            hot ? "bg-amber-500" : "bg-blue-500",
          )}
        />
      </div>
    </div>
  );
}

function WallClockBar({ job }: { job: Job }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (job.ended_at !== null) return;
    if (job.started_at === null) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [job.started_at, job.ended_at]);
  const elapsed =
    job.started_at === null ? 0 : (job.ended_at ?? now) - job.started_at;
  return (
    <CapBar
      label="time"
      value={elapsed}
      cap={job.wall_clock_cap_ms}
      format={formatDurationMs}
    />
  );
}

function PhaseBody({
  phase,
  job,
  isActive,
  isPast,
  refetchJob,
  onOpenJobTab,
}: {
  phase: PhaseSpec;
  job: Job;
  isActive: boolean;
  isPast: boolean;
  refetchJob: () => void;
  onOpenJobTab?: (jobId: JobId, initialTitle: string) => void;
}) {
  if (phase.id === "draft") {
    return <DraftBody job={job} isActive={isActive} refetchJob={refetchJob} />;
  }
  if (phase.id === "queued") {
    return <QueuedBody isActive={isActive} isPast={isPast} />;
  }
  if (phase.id === "running") {
    return (
      <RunningBody
        job={job}
        isActive={isActive}
        isPast={isPast}
        refetchJob={refetchJob}
      />
    );
  }
  return <TerminalBody job={job} isActive={isActive} onOpenJobTab={onOpenJobTab} />;
}

function DraftBody({
  job,
  isActive,
  refetchJob,
}: {
  job: Job;
  isActive: boolean;
  refetchJob: () => void;
}) {
  const rpc = useRpc();
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const start = async () => {
    setBusy(true);
    setErr(null);
    try {
      await rpc.call("start_job", { job_id: job.id });
      refetchJob();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!isActive) return null;

  return (
    <div className="space-y-2">
      <p className="text-muted-foreground text-xs">
        ready to run — edit the spec from the SPEC pane, or kick it off now.
      </p>
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          onClick={start}
          disabled={busy}
          className="bg-blue-600 text-white hover:bg-blue-700"
        >
          {busy ? "starting…" : "run ▶"}
        </Button>
        <span className="text-muted-foreground text-[11px]">
          promotes Draft → Queued; the driver picks it up next tick.
        </span>
      </div>
      {err && <div className="text-destructive text-xs">{err}</div>}
    </div>
  );
}

function QueuedBody({
  isActive,
  isPast,
}: {
  isActive: boolean;
  isPast: boolean;
}) {
  if (!isActive && !isPast) return null;
  if (isPast) {
    return (
      <p className="text-muted-foreground text-xs">
        driver picked the job up.
      </p>
    );
  }
  return (
    <div className="flex items-center gap-2">
      <PulseDot color="bg-amber-500" />
      <p className="text-amber-700 dark:text-amber-400 text-xs italic">
        waiting for the driver to allocate a worktree…
      </p>
    </div>
  );
}

function RunningBody({
  job,
  isActive,
  isPast,
}: {
  job: Job;
  isActive: boolean;
  isPast: boolean;
  // Kept for prop-shape symmetry with the other phase bodies; not
  // used here — the stop button now lives in `JobHeader`.
  refetchJob: () => void;
}) {
  if (!isActive && !isPast) return null;
  return (
    <div className="space-y-3">
      <LiveStageCards jobId={job.id} />
      {isActive && job.status === "awaiting-review" && (
        <Badge
          variant="outline"
          className="border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-300"
        >
          awaiting review
        </Badge>
      )}
    </div>
  );
}

function TerminalBody({
  job,
  isActive,
}: {
  job: Job;
  isActive: boolean;
  // Kept on the Props signature so callers (PhaseBody) can pass it
  // uniformly with other phase bodies; the actual re-run button now
  // lives in `JobHeader` so any section of the page can trigger it.
  onOpenJobTab?: (jobId: JobId, initialTitle: string) => void;
}) {
  const tone = terminalTone(job.status);
  if (!isActive) return null;
  return <FinalRollup job={job} tone={tone} />;
}

// Per-job chat. One persistent thread per job: every turn (user
// message + assistant reply) appended to `.codeless/jobs/<name>/CHAT.md`
// so history survives tab refreshes, server restarts, and is visible
// to the next regular run (CHAT.md is auto-registered in the
// template's `docs:` block on first message, so the runner sees the
// full conversation as context).
//
// The runtime side is `agent_chat(runner, prompt, session_id, cwd)`:
// a one-shot claude turn whose tokens stream back through the event
// bus, scoped by session_id. We mint the session id from the job id
// so every turn of the same job's chat lands on the same SSE filter
// and accumulates in this panel. The cwd is the job's worktree so
// questions like "how many rows in the csv" can read files that only
// exist on the job's branch.
//
// "Send" and "Run" are the same button: the model decides whether to
// answer or to do work. If it writes / edits / commits, that's fine —
// the worktree captures the change and the regular Done rollup
// surfaces it. The explicit [re-run ▾] button next to this panel is
// for "start the whole template pipeline over", which is a different
// kind of action.
const CHAT_FILE = "CHAT.md";

interface ChatMessage {
  role: "user" | "assistant";
  text: string;
  ts: string;
}

export function JobChat({
  job,
  uiLocation,
  onOpenJobTab: _onOpenJobTab,
}: {
  job: Job;
  /**
   * Where the user is in the app when they hit send (e.g.
   * `jobs/<id>`). Forwarded to the runtime via `ChatContext.ui_location`
   * so the model can ground answers to the active surface. Optional —
   * omit on call sites where the location is already implicit.
   */
  uiLocation?: string;
  onOpenJobTab?: (jobId: JobId, initialTitle: string) => void;
}) {
  const rpc = useRpc();
  const [history, setHistory] = useState<ChatMessage[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [streaming, setStreaming] = useState<{
    sessionId: JobId;
    taskId: string;
    text: string;
  } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const sinceCursor = useRef<number>(0);
  // Attachments staged for the next send. Each entry is the result
  // of an `upload_chat_attachment` RPC: the bytes already live in
  // the worktree and we hand the runtime references on send. Cleared
  // once the turn is dispatched so a follow-up question is not
  // accidentally re-attached.
  const [attachments, setAttachments] = useState<
    Array<{ filename: string; relativePath: string; mimeType?: string }>
  >([]);
  const [uploading, setUploading] = useState(0);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [dragOver, setDragOver] = useState(false);

  // Load prior chat on mount / job change.
  useEffect(() => {
    let cancelled = false;
    setHistory([]);
    setLoaded(false);
    rpc
      .call("read_job_file", { job_id: job.id, filename: CHAT_FILE })
      .then((r) => {
        if (cancelled) return;
        setHistory(parseChatMarkdown(r.content));
        setLoaded(true);
      })
      .catch(() => {
        if (!cancelled) setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, job.id]);

  // Subscribe to the chat session's event stream. We use the job's
  // own id as session_id so every chat turn for this job shares the
  // same filter; the streaming-text accumulator distinguishes turns
  // by task_id (one task per agent_chat call).
  useEventStream(
    { scope: "job", job_id: job.id },
    useCallback(
      (env) => {
        const e = env.event;
        if (!streaming) return;
        if (env.task_id !== streaming.taskId) return;
        if (e.type === "ai-token") {
          setStreaming((s) =>
            s && s.taskId === env.task_id ? { ...s, text: s.text + e.delta } : s,
          );
        }
      },
      [streaming],
    ),
    sinceCursor.current,
  );

  const send = async () => {
    const text = input.trim();
    if ((!text && attachments.length === 0) || busy) return;
    setBusy(true);
    setErr(null);
    const ts = isoNow();
    const userMsg: ChatMessage = {
      role: "user",
      text: renderUserMessageText(text, attachments),
      ts,
    };
    const optimistic = [...history, userMsg];
    const sentAttachments = attachments;
    setHistory(optimistic);
    setInput("");
    setAttachments([]);
    setStreaming({ sessionId: job.id, taskId: "pending", text: "" });

    try {
      // The chat runs in the job's worktree so it can read files
      // produced on the job branch. Fall back to no override only
      // when the job never provisioned a worktree (very early
      // failure path).
      const cwd = job.worktree_path ?? null;
      const result = await rpc.call("agent_chat", {
        runner: cliRunnerFor(job.runner),
        prompt: buildChatPrompt(optimistic, text || "(see attached files)"),
        session_id: job.id,
        cwd,
        context: {
          attachments: sentAttachments.map((a) => ({
            relative_path: a.relativePath,
            mime_type: a.mimeType ?? null,
          })),
          ui_location: uiLocation ?? `jobs/${job.id}`,
          selection: null,
          user_prompts: [],
        },
      });

      setStreaming({ sessionId: result.session_id, taskId: result.task_id, text: "" });

      // Wait for the assistant turn to complete. We poll the
      // streaming bubble's text and resolve when an ai-message-complete
      // event arrives. Simpler than a second listener: set up a one-shot
      // resolver inline via a Promise + the event stream.
      const assistantText = await waitForCompletion(
        rpc,
        job.id,
        result.task_id,
      );

      const assistantMsg: ChatMessage = {
        role: "assistant",
        text: assistantText,
        ts: isoNow(),
      };
      const updated = [...optimistic, assistantMsg];
      setHistory(updated);
      setStreaming(null);

      // Persist transcript.
      await rpc.call("write_job_file", {
        job_id: job.id,
        filename: CHAT_FILE,
        content: renderChatMarkdown(updated),
      });

      // First turn: register CHAT.md in the template's docs so the
      // next regular run sees the conversation.
      if (history.length === 0 && job.template_yaml) {
        const docs = extractGlobalDocs(job.template_yaml);
        if (!docs.includes(CHAT_FILE)) {
          const nextYaml = setGlobalDocs(job.template_yaml, [
            ...docs,
            CHAT_FILE,
          ]);
          try {
            await rpc.call("update_job_template", {
              job_id: job.id,
              template_yaml: nextYaml,
            });
          } catch {
            // Non-fatal: template mutation can fail on hand-edited
            // YAML the surgical mutator doesn't recognise. The chat
            // file still persists; only the auto-fold into the next
            // run's prompt is lost.
          }
        }
      }
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setStreaming(null);
      setHistory(history);
      // Restore the staged attachments so the user can retry without
      // re-picking files. The bytes still live in the worktree from
      // the original upload; we only re-stage the references.
      setAttachments(sentAttachments);
    } finally {
      setBusy(false);
    }
  };

  // Push files through `upload_chat_attachment` and stage their
  // returned worktree-relative paths. Errors land in `err` and the
  // file is dropped from the staged list — the user retries.
  const uploadFiles = useCallback(
    async (files: FileList | File[]) => {
      const list = Array.from(files);
      if (list.length === 0) return;
      if (!job.worktree_path) {
        setErr(
          "this job has no worktree yet — submit/run it before attaching files",
        );
        return;
      }
      setUploading((n) => n + list.length);
      for (const file of list) {
        try {
          const buf = await file.arrayBuffer();
          const b64 = arrayBufferToBase64(buf);
          const result = await rpc.call("upload_chat_attachment", {
            job_id: job.id,
            filename: file.name || "untitled",
            content_b64: b64,
          });
          setAttachments((prev) => [
            ...prev,
            {
              filename: file.name || "untitled",
              relativePath: result.relative_path,
              mimeType: file.type || undefined,
            },
          ]);
        } catch (e) {
          setErr(e instanceof Error ? e.message : String(e));
        } finally {
          setUploading((n) => n - 1);
        }
      }
    },
    [rpc, job.id, job.worktree_path],
  );

  return (
    <div
      className={cn(
        "border-border/60 bg-card/40 flex h-full min-h-0 min-w-0 flex-col gap-2 overflow-hidden rounded-md border p-3",
        dragOver && "border-blue-500/70 ring-1 ring-blue-500/40",
      )}
      onDragOver={(e) => {
        if (!e.dataTransfer.types.includes("Files")) return;
        e.preventDefault();
        setDragOver(true);
      }}
      onDragLeave={(e) => {
        // The drag leaves the root only when the cursor exits the
        // container, not on every child boundary. relatedTarget is
        // null when the pointer leaves the window.
        if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
        setDragOver(false);
      }}
      onDrop={(e) => {
        if (!e.dataTransfer.files || e.dataTransfer.files.length === 0) return;
        e.preventDefault();
        setDragOver(false);
        void uploadFiles(e.dataTransfer.files);
      }}
    >
      <div className="flex shrink-0 items-baseline justify-between gap-2">
        <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
          chat with this job
        </div>
        <span className="text-muted-foreground font-mono text-[10px]">
          {loaded ? `${history.length} message${history.length === 1 ? "" : "s"}` : "loading…"}
        </span>
      </div>

      {history.length === 0 && !streaming && loaded && (
        <p className="text-muted-foreground shrink-0 text-[11px] italic">
          ask anything — "how many rows in the csv?", "where did you put the
          files?", "now add a column for reactive power". the chat runs in
          this job's worktree so claude can read what it made. drop files,
          paste images, or use the attach button to share context.
        </p>
      )}

      <ul className="min-h-0 flex-1 space-y-2 overflow-y-auto pr-1">
        {history.map((m, i) => (
          <ChatBubble key={i} message={m} />
        ))}
        {streaming && (
          <ChatBubble
            message={{
              role: "assistant",
              text: streaming.text || "…",
              ts: "",
            }}
            streaming
          />
        )}
      </ul>

      {(attachments.length > 0 || uploading > 0) && (
        <div className="flex shrink-0 flex-wrap gap-1">
          {attachments.map((a, i) => (
            <span
              key={`${a.relativePath}-${i}`}
              className="border-border/60 bg-muted/40 flex max-w-full items-center gap-1 rounded border px-1.5 py-0.5 text-[10px]"
              title={a.relativePath}
            >
              <span className="truncate">{a.filename}</span>
              <button
                type="button"
                onClick={() =>
                  setAttachments((prev) => prev.filter((_, j) => j !== i))
                }
                className="text-muted-foreground hover:text-foreground shrink-0"
                aria-label={`remove ${a.filename}`}
              >
                ×
              </button>
            </span>
          ))}
          {uploading > 0 && (
            <span className="text-muted-foreground text-[10px] italic">
              uploading {uploading}…
            </span>
          )}
        </div>
      )}

      <input
        ref={fileInputRef}
        type="file"
        multiple
        className="hidden"
        onChange={(e) => {
          if (e.target.files) void uploadFiles(e.target.files);
          e.target.value = "";
        }}
      />

      <textarea
        value={input}
        onChange={(e) => setInput(e.target.value)}
        rows={2}
        placeholder="message claude about this job…"
        className="border-border/60 bg-background w-full resize-none rounded border px-2 py-1.5 text-xs"
        disabled={busy}
        onPaste={(e) => {
          // Pull files (typically clipboard images) out of the paste.
          // Text pastes still flow into the textarea normally because
          // we don't preventDefault unless files are present.
          const files: File[] = [];
          for (const item of e.clipboardData.items) {
            if (item.kind === "file") {
              const f = item.getAsFile();
              if (f) files.push(f);
            }
          }
          if (files.length > 0) {
            e.preventDefault();
            void uploadFiles(files);
          }
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void send();
          }
        }}
      />
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          variant="outline"
          onClick={() => fileInputRef.current?.click()}
          disabled={busy || !job.worktree_path}
          title={
            job.worktree_path
              ? "attach files (also: drop or paste)"
              : "no worktree yet — submit/run the job first"
          }
        >
          attach
        </Button>
        <Button
          size="sm"
          onClick={send}
          disabled={busy || (!input.trim() && attachments.length === 0)}
          className="bg-blue-600 text-white hover:bg-blue-700"
        >
          {busy ? "thinking…" : "send ▶"}
        </Button>
        <span className="text-muted-foreground text-[10px]">
          persisted to <code className="bg-muted/40 rounded px-1">{CHAT_FILE}</code>
          {" · "}runs in {job.worktree_path ? "worktree" : "repo root"}{" · ⌘/Ctrl+Enter"}
        </span>
      </div>
      {err && <div className="text-destructive text-xs">{err}</div>}
    </div>
  );
}

function ChatBubble({
  message,
  streaming,
}: {
  message: ChatMessage;
  streaming?: boolean;
}) {
  const isUser = message.role === "user";
  return (
    <li
      className={cn(
        "rounded-md border px-2.5 py-2",
        isUser
          ? "border-zinc-500/30 bg-zinc-500/5"
          : "border-blue-500/30 bg-blue-500/5",
      )}
    >
      <div className="text-muted-foreground mb-1 flex items-center justify-between gap-1.5 text-[9px] uppercase tracking-wide">
        <span className={isUser ? "" : "text-blue-700 dark:text-blue-300"}>
          {isUser ? "you" : "assistant"}
        </span>
        {streaming && <PulseDot color="bg-blue-500" />}
        {!streaming && message.ts && (
          <span className="font-mono normal-case tracking-normal">
            {shortTime(message.ts)}
          </span>
        )}
      </div>
      <div className="prose prose-sm dark:prose-invert max-w-none text-[12px] break-words [&_pre]:my-1.5 [&_pre]:bg-background/60 [&_pre]:p-2 [&_pre]:text-[11px] [&_pre]:whitespace-pre-wrap [&_pre]:break-words [&_pre]:overflow-x-auto [&_code]:bg-background/60 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-[11px] [&_code]:break-all [&_h1]:text-sm [&_h2]:text-sm [&_h3]:text-[13px] [&_h1]:font-semibold [&_h2]:font-semibold [&_h3]:font-semibold [&_p]:my-1 [&_ul]:my-1 [&_ol]:my-1 [&_li]:my-0">
        <Streamdown>{message.text}</Streamdown>
      </div>
    </li>
  );
}

function shortTime(iso: string): string {
  return iso.replace("T", " ").replace("Z", "");
}

function isoNow(): string {
  return new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
}

// Encode a binary blob as standard base64. Done in chunks because
// `String.fromCharCode(...big array)` blows the call-stack limit on
// large pasted images; 32k is well under the practical limit and
// keeps the loop cheap for typical attachments.
function arrayBufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  const chunk = 0x8000;
  let binary = "";
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(
      ...bytes.subarray(i, Math.min(i + chunk, bytes.length)),
    );
  }
  return btoa(binary);
}

// What we persist to CHAT.md for a turn that included attachments.
// Keeps the worktree-relative paths visible so reopening the chat
// later still shows what the user shared. The runtime's preamble is
// not duplicated here — the model receives it via `ChatContext` on
// the wire, and CHAT.md is the user-facing transcript.
function renderUserMessageText(
  text: string,
  attachments: Array<{ filename: string; relativePath: string }>,
): string {
  if (attachments.length === 0) return text;
  const list = attachments
    .map((a) => `- ${a.filename} (\`${a.relativePath}\`)`)
    .join("\n");
  return text ? `${text}\n\n_attached:_\n${list}` : `_attached:_\n${list}`;
}

// Anthropic / OpenAI / Copilot / etc map to CLI runner ids the
// agent_chat RPC accepts. The job's `runner` field may be a REST id
// ("anthropic"); pick a sensible CLI alternative because agent_chat
// only accepts CLI runners.
function cliRunnerFor(jobRunner: string): string {
  if (jobRunner === "claude") return "claude";
  if (jobRunner === "copilot") return "copilot";
  if (jobRunner === "codex") return "codex";
  // anthropic / openai / mock all fall through to claude — agent_chat
  // rejects REST runner ids, and mock doesn't make sense here.
  return "claude";
}

// Build the prompt sent to agent_chat: full prior transcript so each
// one-shot call has conversational context (agent_chat itself is
// stateless per v1 — no --continue), then the new user message.
function buildChatPrompt(history: ChatMessage[], _latest: string): string {
  if (history.length === 1) return history[0].text;
  const lines: string[] = [];
  lines.push("This is a chat about a code job. Prior turns follow.\n");
  for (const m of history.slice(0, -1)) {
    lines.push(`### ${m.role === "user" ? "User" : "Assistant"}`);
    lines.push(m.text);
    lines.push("");
  }
  lines.push("### User");
  lines.push(history[history.length - 1].text);
  lines.push("");
  lines.push(
    "Reply directly to the latest user message. If a question, answer it; if a task, do it.",
  );
  return lines.join("\n");
}

// Subscribe-and-resolve: returns when ai-message-complete arrives for
// `taskId`, with the accumulated ai-token text. We open our own
// subscription rather than reading from the shared one because the
// caller wants a single Promise to await; the shared subscription
// dedup keeps the connection count to one.
function waitForCompletion(
  rpc: ReturnType<typeof useRpc>,
  jobId: JobId,
  taskId: string,
): Promise<string> {
  return new Promise((resolve, reject) => {
    let text = "";
    let done = false;
    const stream = rpc.subscribe({ scope: "job", job_id: jobId });
    const iter = stream[Symbol.asyncIterator]();
    (async () => {
      try {
        while (!done) {
          const r = await iter.next();
          if (r.done) {
            if (!done) reject(new Error("event stream closed before completion"));
            return;
          }
          const env = r.value;
          if (env.task_id !== taskId) continue;
          const e = env.event;
          if (e.type === "ai-token") {
            text += e.delta;
          } else if (e.type === "ai-message-complete") {
            done = true;
            iter.return?.();
            resolve(text);
            return;
          }
        }
      } catch (e) {
        if (!done) reject(e instanceof Error ? e : new Error(String(e)));
      }
    })();
  });
}

function parseChatMarkdown(src: string): ChatMessage[] {
  // Round-trips renderChatMarkdown's `## role @ ts` headings.
  const out: ChatMessage[] = [];
  const lines = src.split("\n");
  let current: ChatMessage | null = null;
  let buf: string[] = [];
  const flush = () => {
    if (current) {
      current.text = buf.join("\n").trim();
      if (current.text) out.push(current);
    }
    current = null;
    buf = [];
  };
  for (const raw of lines) {
    const m = /^##\s+(user|assistant)\s+@\s+(.+)$/i.exec(raw.trim());
    if (m) {
      flush();
      current = {
        role: m[1].toLowerCase() === "user" ? "user" : "assistant",
        ts: m[2].trim(),
        text: "",
      };
      continue;
    }
    if (current) buf.push(raw);
  }
  flush();
  return out;
}

function renderChatMarkdown(messages: ChatMessage[]): string {
  const out: string[] = ["# Chat for this job", ""];
  for (const m of messages) {
    out.push(`## ${m.role} @ ${m.ts}`);
    out.push("");
    out.push(m.text);
    out.push("");
  }
  return out.join("\n");
}

function extractGlobalDocs(yaml: string): string[] {
  const out: string[] = [];
  const m = /^docs\s*:\s*$/m.exec(yaml);
  if (!m) {
    const flow = /^docs\s*:\s*\[(.*)\]\s*$/m.exec(yaml);
    if (flow) {
      return flow[1]
        .split(",")
        .map((s) => s.trim().replace(/^["']|["']$/g, ""))
        .filter(Boolean);
    }
    return out;
  }
  const after = yaml.slice(m.index + m[0].length).split("\n").slice(1);
  for (const raw of after) {
    if (/^\S/.test(raw) && raw.trim() !== "") break;
    const item = /^\s*-\s*(?:"([^"]*)"|'([^']*)'|(.+?))\s*$/.exec(raw);
    if (item) {
      const name = (item[1] ?? item[2] ?? item[3] ?? "").trim();
      if (name) out.push(name);
    }
  }
  return out;
}

// Per-stage rollup for finished jobs. Pulls from list_stages so we can
// show the spine of what actually ran without leaving the Run pane.
function FinalRollup({
  job,
  tone,
}: {
  job: Job;
  tone: ReturnType<typeof terminalTone>;
}) {
  const rpc = useRpc();
  const [stages, setStages] = useState<StageRollup[] | null>(null);
  useEffect(() => {
    let cancelled = false;
    rpc
      .call("list_stages", { job_id: job.id })
      .then((res) => {
        if (!cancelled) setStages(res.stages);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [rpc, job.id]);

  const totalCost = stages?.reduce((a, s) => a + s.cost_cents, 0) ?? 0;
  const totalTasks = stages?.reduce((a, s) => a + s.task_count, 0) ?? 0;

  const headline =
    job.status === "completed"
      ? "finished cleanly"
      : job.status === "failed"
        ? "exited with an error"
        : "stopped before completion";

  return (
    <div className={cn("rounded-md border p-3", tone.bg)}>
      <p className={cn("text-xs font-medium", tone.text)}>{headline}</p>
      <div className="mt-2 grid grid-cols-3 gap-3">
        <MiniStat
          label="duration"
          value={
            job.started_at && job.ended_at
              ? formatDurationMs(job.ended_at - job.started_at)
              : "—"
          }
        />
        <MiniStat
          label="cost"
          value={`$${((totalCost || job.cost_cents) / 100).toFixed(2)}`}
        />
        <MiniStat
          label="tasks"
          value={stages ? String(totalTasks) : "…"}
        />
      </div>
      {stages && stages.length > 0 && (
        <ul className="mt-3 space-y-1">
          {stages.map((s) => (
            <li
              key={s.stage.id}
              className="bg-background/40 flex items-center justify-between gap-2 rounded px-2 py-1 text-[11px]"
            >
              <div className="flex min-w-0 items-center gap-2">
                <StageStatusDot status={s.stage.status} />
                <span className="text-muted-foreground font-mono text-[10px] tabular-nums">
                  {String(s.stage.ordinal + 1).padStart(2, "0")}
                </span>
                <span className="truncate">{s.stage.name || "unnamed"}</span>
              </div>
              <div className="text-muted-foreground flex shrink-0 items-center gap-2 font-mono text-[10px]">
                <span>{stageDuration(s)}</span>
                <span>${(s.cost_cents / 100).toFixed(2)}</span>
              </div>
            </li>
          ))}
        </ul>
      )}

      <FilesProduced jobId={job.id} />
    </div>
  );
}

// Inline files-changed strip on the Done node. Users were missing
// the "Files changed" sidebar item entirely; this surfaces the diff
// summary right where they look for "what did it actually make".
function FilesProduced({ jobId }: { jobId: JobId }) {
  const rpc = useRpc();
  const [state, setState] = useState<
    | { kind: "loading" }
    | { kind: "ready"; files: JobDiffFile[]; base: string; head: string }
    | { kind: "error" }
  >({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    setState({ kind: "loading" });
    rpc
      .call("job_diff", { job_id: jobId })
      .then((diff) => {
        if (cancelled) return;
        setState({
          kind: "ready",
          files: diff.files,
          base: diff.base,
          head: diff.head,
        });
      })
      .catch(() => {
        if (cancelled) return;
        setState({ kind: "error" });
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, jobId]);

  if (state.kind !== "ready" || state.files.length === 0) return null;

  return (
    <div className="mt-3">
      <div className="text-muted-foreground mb-1 flex items-center justify-between gap-2 text-[10px] uppercase tracking-wide">
        <span>
          files changed
          <span className="ml-1 font-mono normal-case tracking-normal">
            ({state.files.length})
          </span>
        </span>
        <span className="text-muted-foreground font-mono normal-case tracking-normal text-[10px]">
          vs {shortRef(state.base)}
        </span>
      </div>
      <ul className="space-y-0.5">
        {state.files.slice(0, 8).map((f) => (
          <li
            key={f.path}
            className="bg-background/40 flex items-center justify-between gap-2 rounded px-2 py-1 text-[11px]"
          >
            <div className="flex min-w-0 items-center gap-2">
              <StatusLetter status={f.status} />
              <span className="truncate font-mono">{f.path}</span>
            </div>
            <div className="text-muted-foreground flex shrink-0 items-center gap-2 font-mono text-[10px] tabular-nums">
              {f.additions > 0 && (
                <span className="text-emerald-600 dark:text-emerald-400">
                  +{f.additions}
                </span>
              )}
              {f.deletions > 0 && (
                <span className="text-red-600 dark:text-red-400">
                  -{f.deletions}
                </span>
              )}
            </div>
          </li>
        ))}
        {state.files.length > 8 && (
          <li className="text-muted-foreground px-2 py-1 text-[10px] italic">
            … and {state.files.length - 8} more — see the Files changed pane
          </li>
        )}
      </ul>
    </div>
  );
}

function StatusLetter({ status }: { status: string }) {
  const map: Record<string, { label: string; cls: string }> = {
    A: { label: "A", cls: "bg-emerald-500/20 text-emerald-700 dark:text-emerald-300" },
    M: { label: "M", cls: "bg-amber-500/20 text-amber-700 dark:text-amber-300" },
    D: { label: "D", cls: "bg-red-500/20 text-red-700 dark:text-red-300" },
    R: { label: "R", cls: "bg-blue-500/20 text-blue-700 dark:text-blue-300" },
  };
  const m = map[status] ?? { label: status, cls: "bg-muted text-muted-foreground" };
  return (
    <span
      className={cn(
        "inline-flex h-4 w-4 shrink-0 items-center justify-center rounded font-mono text-[9px] font-bold",
        m.cls,
      )}
    >
      {m.label}
    </span>
  );
}

function shortRef(ref: string): string {
  // "refs/heads/main" → "main"
  return ref.replace(/^refs\/(heads|remotes\/origin)\//, "");
}

function MiniStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-background/40 rounded px-2 py-1.5">
      <div className="text-sm font-medium tabular-nums">{value}</div>
      <div className="text-muted-foreground text-[9px] uppercase tracking-wide">
        {label}
      </div>
    </div>
  );
}

function StageStatusDot({ status }: { status: StageRollup["stage"]["status"] }) {
  const cls =
    status === "passed"
      ? "bg-emerald-500"
      : status === "failed"
        ? "bg-red-500"
        : status === "running"
          ? "bg-blue-500"
          : status === "awaiting-review"
            ? "bg-amber-500"
            : "bg-muted-foreground/40";
  return <span className={cn("h-2 w-2 shrink-0 rounded-full", cls)} />;
}

function stageDuration(s: StageRollup): string {
  const start = s.stage.started_at;
  const end = s.stage.ended_at;
  if (!start) return "—";
  return formatDurationMs((end ?? Date.now()) - start);
}

// Live stage cards: subscribe to the per-job SSE stream and bucket
// events by stage_id. Each card shows the stage name + ordinal, an
// animated status pill, duration ticking up while running, the
// running cost, a tool-call ribbon with the last 6 calls, and a
// streaming assistant-message bubble fed by ai-token deltas. This is
// the meat of the "is anything happening" question.
interface LiveStage {
  stageId: string;
  ordinal: number | null;
  name: string | null;
  status: "running" | "passed" | "failed" | "awaiting-review";
  startedAt: number;
  endedAt: number | null;
  tools: ToolCall[];
  assistantText: string;
  cost_cents: number;
  task_count: number;
}

interface ToolCall {
  key: string;
  tool: string;
  summary: string;
  approval: boolean;
}

function LiveStageCards({ jobId }: { jobId: JobId }) {
  const [stages, setStages] = useState<Record<string, LiveStage>>({});
  const [order, setOrder] = useState<string[]>([]);
  const counter = useRef(0);
  // task_id → stage_id, plus the most recently started stage. Most
  // mid-stage envelopes (tool-call, ai-token, ai-message-complete,
  // task-*) only carry task_id, not stage_id — `task-enqueued` (and
  // `stage-started`, which carries the kickoff task) are the only
  // events that bind the two. The linear executor only runs one
  // stage at a time, so unbound envelopes route to that stage.
  const taskToStage = useRef<Map<string, string>>(new Map());
  const activeStage = useRef<string | null>(null);

  useEffect(() => {
    setStages({});
    setOrder([]);
    counter.current = 0;
    taskToStage.current = new Map();
    activeStage.current = null;
  }, [jobId]);

  useEventStream(
    { scope: "job", job_id: jobId },
    useCallback((env: EventEnvelope) => {
      const e = env.event;
      // Bucket envelopes by stage. Resolve the stage this envelope
      // belongs to: Prefer the
      // server-stamped stage_id; fall back to task→stage map; final
      // fallback is the currently active stage (linear executor only
      // runs one at a time).
      const resolveStage = (): string | null => {
        if (env.stage_id) return env.stage_id;
        if (env.task_id && taskToStage.current.has(env.task_id)) {
          return taskToStage.current.get(env.task_id)!;
        }
        return activeStage.current;
      };

      if (e.type === "stage-started") {
        activeStage.current = e.stage_id;
        if (env.task_id) {
          taskToStage.current.set(env.task_id, e.stage_id);
        }
        setStages((prev) => {
          if (prev[e.stage_id]) return prev;
          return {
            ...prev,
            [e.stage_id]: {
              stageId: e.stage_id,
              ordinal: e.ordinal ?? null,
              name: e.name ?? null,
              status: "running",
              startedAt: env.created_at,
              endedAt: null,
              tools: [],
              assistantText: "",
              cost_cents: 0,
              task_count: 0,
            },
          };
        });
        setOrder((prev) =>
          prev.includes(e.stage_id) ? prev : [...prev, e.stage_id],
        );
        return;
      }

      if (e.type === "stage-completed") {
        if (activeStage.current === e.stage_id) activeStage.current = null;
        setStages((prev) => {
          const s = prev[e.stage_id];
          if (!s) return prev;
          return {
            ...prev,
            [e.stage_id]: {
              ...s,
              status: e.status as LiveStage["status"],
              endedAt: env.created_at,
            },
          };
        });
        return;
      }

      if (e.type === "task-enqueued") {
        taskToStage.current.set(e.task_id, e.stage_id);
        return;
      }

      const stageId = resolveStage();
      if (!stageId) return;

      if (e.type === "tool-call" || e.type === "tool-approval-requested") {
        const summary = summariseToolArgs(e.tool, e.args_json);
        const key = `${env.cursor}-${counter.current++}`;
        setStages((prev) => {
          const s = prev[stageId];
          if (!s) return prev;
          const nextTools = [
            ...s.tools,
            {
              key,
              tool: e.tool,
              summary,
              approval: e.type === "tool-approval-requested",
            },
          ].slice(-6);
          return { ...prev, [stageId]: { ...s, tools: nextTools } };
        });
        return;
      }

      if (e.type === "ai-token") {
        setStages((prev) => {
          const s = prev[stageId];
          if (!s) return prev;
          return {
            ...prev,
            [stageId]: { ...s, assistantText: s.assistantText + e.delta },
          };
        });
        return;
      }

      if (e.type === "ai-message-complete") {
        setStages((prev) => {
          const s = prev[stageId];
          if (!s) return prev;
          return {
            ...prev,
            [stageId]: { ...s, cost_cents: s.cost_cents + e.cost_cents },
          };
        });
        return;
      }

      if (e.type === "task-started") {
        setStages((prev) => {
          const s = prev[stageId];
          if (!s) return prev;
          return { ...prev, [stageId]: { ...s, task_count: s.task_count + 1 } };
        });
      }
    }, []),
  );

  const ordered = useMemo(
    () => order.map((id) => stages[id]).filter(Boolean) as LiveStage[],
    [order, stages],
  );

  if (ordered.length === 0) {
    return (
      <div className="border-border/60 bg-muted/20 flex items-center gap-2 rounded-md border border-dashed px-3 py-2">
        <PulseDot color="bg-blue-500" />
        <p className="text-muted-foreground text-xs italic">
          waiting for the first stage…
        </p>
      </div>
    );
  }

  return (
    <ul className="space-y-2">
      <AnimatePresence initial={false}>
        {ordered.map((s) => (
          <motion.li
            key={s.stageId}
            initial={{ opacity: 0, y: -6, height: 0 }}
            animate={{ opacity: 1, y: 0, height: "auto" }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.3, ease: "easeOut" }}
          >
            <StageCard stage={s} />
          </motion.li>
        ))}
      </AnimatePresence>
    </ul>
  );
}

function StageCard({ stage }: { stage: LiveStage }) {
  const isRunning = stage.status === "running";
  const tone =
    stage.status === "passed"
      ? "border-emerald-500/40 bg-emerald-500/5"
      : stage.status === "failed"
        ? "border-red-500/40 bg-red-500/5"
        : stage.status === "awaiting-review"
          ? "border-amber-500/40 bg-amber-500/5"
          : "border-blue-500/40 bg-blue-500/5";

  return (
    <div className={cn("rounded-md border p-3", tone)}>
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <StageStatusDot status={stage.status} />
          {stage.ordinal !== null && (
            <span className="text-muted-foreground font-mono text-[10px] tabular-nums">
              {String(stage.ordinal + 1).padStart(2, "0")}
            </span>
          )}
          <span className="truncate text-sm font-medium">
            {stage.name ?? "stage"}
          </span>
          {isRunning && <PulseDot color="bg-blue-500" />}
        </div>
        <div className="text-muted-foreground flex shrink-0 items-center gap-2 font-mono text-[10px] tabular-nums">
          <StageClock stage={stage} />
          {stage.cost_cents > 0 && (
            <span>${(stage.cost_cents / 100).toFixed(2)}</span>
          )}
        </div>
      </div>

      {stage.tools.length > 0 && <ToolRibbon tools={stage.tools} />}

      {stage.assistantText && (
        <AssistantBubble text={stage.assistantText} streaming={isRunning} />
      )}
    </div>
  );
}

function StageClock({ stage }: { stage: LiveStage }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (stage.endedAt !== null) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [stage.endedAt]);
  const end = stage.endedAt ?? now;
  return <span>{formatDurationMs(end - stage.startedAt)}</span>;
}

function ToolRibbon({ tools }: { tools: ToolCall[] }) {
  return (
    <ul className="border-border/40 mt-2 space-y-0.5 border-l-2 pl-2">
      <AnimatePresence initial={false}>
        {tools.map((t) => (
          <motion.li
            key={t.key}
            initial={{ opacity: 0, x: -6 }}
            animate={{ opacity: 1, x: 0 }}
            transition={{ duration: 0.2 }}
            className="flex items-baseline gap-1.5 font-mono text-[11px]"
          >
            {t.approval && (
              <span className="text-amber-600 dark:text-amber-400">⏵</span>
            )}
            <span
              className={cn(
                "shrink-0 font-medium",
                toolColor(t.tool),
              )}
            >
              {t.tool}
            </span>
            <span className="text-muted-foreground truncate">
              {t.summary || "—"}
            </span>
          </motion.li>
        ))}
      </AnimatePresence>
    </ul>
  );
}

function toolColor(tool: string): string {
  switch (tool) {
    case "Bash":
      return "text-purple-600 dark:text-purple-400";
    case "Read":
    case "Glob":
    case "Grep":
      return "text-blue-600 dark:text-blue-400";
    case "Write":
    case "Edit":
    case "MultiEdit":
    case "NotebookEdit":
      return "text-emerald-600 dark:text-emerald-400";
    case "TodoWrite":
      return "text-amber-600 dark:text-amber-400";
    default:
      return "text-foreground";
  }
}

function AssistantBubble({
  text,
  streaming,
}: {
  text: string;
  streaming: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  // Collapse very long completions by default so a 5KB message
  // doesn't shove the rest of the page off-screen. Streamdown renders
  // claude's prose with headings / fences / lists intact; the raw
  // Timeline section is still the escape hatch for the source view.
  const LIMIT = 1200;
  const tooLong = text.length > LIMIT;
  const shown = !tooLong || expanded ? text : `${text.slice(0, LIMIT)}…`;
  return (
    <div className="mt-2 rounded border border-blue-500/30 bg-blue-500/5 px-2.5 py-2">
      <div className="text-muted-foreground mb-1 flex items-center gap-1.5 text-[9px] uppercase tracking-wide">
        <span className="text-blue-700 dark:text-blue-300">assistant</span>
        {streaming && <PulseDot color="bg-blue-500" />}
        <span className="ml-auto font-mono normal-case tracking-normal text-[10px]">
          {text.length} chars
        </span>
      </div>
      <div className="prose prose-sm dark:prose-invert max-w-none text-[12px] break-words [&_pre]:my-1.5 [&_pre]:bg-background/60 [&_pre]:p-2 [&_pre]:text-[11px] [&_pre]:whitespace-pre-wrap [&_pre]:break-words [&_pre]:overflow-x-auto [&_code]:bg-background/60 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-[11px] [&_code]:break-all [&_h1]:text-sm [&_h2]:text-sm [&_h3]:text-[13px] [&_h1]:font-semibold [&_h2]:font-semibold [&_h3]:font-semibold [&_h1]:mt-2 [&_h2]:mt-2 [&_h3]:mt-2 [&_h1]:mb-1 [&_h2]:mb-1 [&_h3]:mb-1 [&_p]:my-1 [&_ul]:my-1 [&_ol]:my-1 [&_li]:my-0">
        <Streamdown>{shown}</Streamdown>
        {streaming && (
          <motion.span
            aria-hidden
            animate={{ opacity: [1, 0, 1] }}
            transition={{ duration: 1, repeat: Infinity }}
            className="bg-foreground -mt-3 ml-0.5 inline-block h-3 w-1 align-text-bottom"
          />
        )}
      </div>
      {tooLong && (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="text-muted-foreground hover:text-foreground mt-1 text-[10px] underline"
        >
          {expanded ? "show less" : `show all ${text.length} chars`}
        </button>
      )}
    </div>
  );
}

function PulseDot({ color }: { color: string }) {
  return (
    <span className="relative inline-block h-2 w-2">
      <motion.span
        aria-hidden
        animate={{ opacity: [0.4, 1, 0.4], scale: [1, 1.2, 1] }}
        transition={{ duration: 1.4, repeat: Infinity }}
        className={cn("absolute inset-0 rounded-full", color)}
      />
    </span>
  );
}

function formatDurationMs(ms: number): string {
  if (ms < 0) return "0s";
  if (ms < 1000) return `${ms}ms`;
  const totalSec = Math.floor(ms / 1000);
  if (totalSec < 60) return `${totalSec}s`;
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  if (m < 60) return `${m}m ${String(s).padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  const remM = m % 60;
  return `${h}h ${String(remM).padStart(2, "0")}m`;
}
