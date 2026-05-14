import { useCallback, useEffect, useRef, useState } from "react";

import { cn } from "@/lib/utils";
import { useEventStream, type EventEnvelope, type JobId } from "@/lib/rpc";

// ------------------------------------------------------------------ types

export type TabIndicator = "running" | "failed" | "review" | "paused" | null;

// Always-pinned system tabs. Order matches the spec's visual left-to-right.
export type SystemTabId = "CHAT" | "SPEC" | "Stages";

export interface StageTab {
  kind: "stage";
  stageId: string;
  // Display name (e.g. "auth") surfaced from the stage-started event or
  // the template YAML. Falls back to the stageId prefix.
  stageName: string;
  pinned: boolean;
}

export type ActiveTab = { kind: "system"; id: SystemTabId } | StageTab;

interface Props {
  jobId: JobId;
  active: ActiveTab;
  stageTabs: StageTab[];
  onActivate: (tab: ActiveTab) => void;
  // Close a stage tab. The Stages overview keeps working; the tab is
  // just removed from the strip. Pinned stage tabs do not receive this
  // call.
  onClose: (stageId: string) => void;
  // Toggle the pinned state of a stage tab.
  onTogglePin: (stageId: string) => void;
}

// ------------------------------------------------------------------ indicator derivation

// Per-tab indicator derived from the event stream. The indicator clears
// when the tab becomes active (cursor advances). Client-side only: no
// server round-trip needed because the events themselves carry the state.
//
// The CHAT tab shows ● when there is unread ai-token / ai-message-complete
// activity outside a stage context.
//
// Stage-N tabs show ! when the stage failed, ⟳ when awaiting review.
// The Stages tab shows ! when any stage has failed and the user is not
// already on it.

function useTabIndicators(
  jobId: JobId,
  active: ActiveTab,
): Map<string, TabIndicator> {
  // Map<tabKey, indicator>. tabKey is SystemTabId for system tabs or
  // stageId for stage tabs.
  const [indicators, setIndicators] = useState<Map<string, TabIndicator>>(
    new Map(),
  );

  // The active tab's key: while the user is on it, new events for that
  // tab don't accumulate an unread indicator.
  const activeKey = useRef<string>(activeTabKey(active));
  useEffect(() => {
    activeKey.current = activeTabKey(active);
    // Clear the indicator for whichever tab just became active.
    setIndicators((prev) => {
      const next = new Map(prev);
      next.delete(activeKey.current);
      return next;
    });
  }, [active]);

  const onEvent = useCallback((env: EventEnvelope) => {
    const e = env.event;
    const stageId = env.stage_id ?? ("stage_id" in e && typeof e.stage_id === "string" ? e.stage_id : null);
    const key = stageId ?? "CHAT";
    // Don't accumulate an indicator for the currently-active tab.
    if (key === activeKey.current) return;

    setIndicators((prev) => {
      const next = new Map(prev);
      switch (e.type) {
        case "ai-token":
        case "ai-message-complete":
          // Only mark CHAT as unread when the token belongs to no stage
          // (job-level chat or a standalone agent_chat session).
          if (!stageId && !prev.has("CHAT")) {
            next.set("CHAT", "running");
          }
          break;
        case "verify-failed":
        case "stage-completed": {
          const failed =
            e.type === "verify-failed" ||
            (e.type === "stage-completed" && e.status !== "passed");
          if (failed) {
            if (stageId) next.set(stageId, "failed");
            // Surface a ! on the Stages overview tab so the user can
            // navigate there to see which stage failed, even if they're
            // currently on a different tab.
            if (key !== "Stages") next.set("Stages", "failed");
          }
          break;
        }
        case "review-requested":
          if (stageId) next.set(stageId, "review");
          break;
        case "job-paused":
          // The CHAT tab is the most relevant surface when the job pauses;
          // stage tabs don't map cleanly to the pause event.
          if (!prev.has("CHAT")) next.set("CHAT", "paused");
          break;
        default:
          break;
      }
      return next;
    });
  }, []);

  useEventStream({ scope: "job", job_id: jobId }, onEvent);

  return indicators;
}

function activeTabKey(tab: ActiveTab): string {
  if (tab.kind === "system") return tab.id;
  return tab.stageId;
}

// ------------------------------------------------------------------ component

export function JobTabs({
  jobId,
  active,
  stageTabs,
  onActivate,
  onClose,
  onTogglePin,
}: Props) {
  const indicators = useTabIndicators(jobId, active);

  return (
    <div
      className="border-b border-border/50 flex items-center gap-0 overflow-x-auto"
      role="tablist"
    >
      {/* Always-pinned system tabs */}
      {(["CHAT", "SPEC", "Stages"] as SystemTabId[]).map((id) => {
        const isActive = active.kind === "system" && active.id === id;
        const indicator = indicators.get(id) ?? null;
        return (
          <SystemTabButton
            key={id}
            id={id}
            isActive={isActive}
            indicator={indicator}
            onClick={() => onActivate({ kind: "system", id })}
          />
        );
      })}

      {/* User-opened stage tabs */}
      {stageTabs.map((tab) => {
        const isActive =
          active.kind === "stage" && active.stageId === tab.stageId;
        const indicator = indicators.get(tab.stageId) ?? null;
        return (
          <StageTabButton
            key={tab.stageId}
            tab={tab}
            isActive={isActive}
            indicator={indicator}
            onClick={() => onActivate(tab)}
            onClose={() => onClose(tab.stageId)}
            onTogglePin={() => onTogglePin(tab.stageId)}
          />
        );
      })}
    </div>
  );
}

// ------------------------------------------------------------------ sub-components

function indicatorGlyph(indicator: TabIndicator): string | null {
  switch (indicator) {
    case "running":
      return "●";
    case "failed":
      return "!";
    case "review":
      return "⟳";
    case "paused":
      return "⏸";
    default:
      return null;
  }
}

function indicatorTone(indicator: TabIndicator): string {
  switch (indicator) {
    case "running":
      return "text-blue-500";
    case "failed":
      return "text-destructive";
    case "review":
      return "text-amber-500";
    case "paused":
      return "text-muted-foreground";
    default:
      return "";
  }
}

function SystemTabButton({
  id,
  isActive,
  indicator,
  onClick,
}: {
  id: SystemTabId;
  isActive: boolean;
  indicator: TabIndicator;
  onClick: () => void;
}) {
  const glyph = indicatorGlyph(indicator);
  return (
    <button
      role="tab"
      aria-selected={isActive}
      onClick={onClick}
      className={cn(
        "flex shrink-0 items-center gap-1 border-b-2 px-4 py-2.5 text-sm font-medium transition-colors",
        isActive
          ? "border-primary text-foreground"
          : "border-transparent text-muted-foreground hover:text-foreground",
      )}
    >
      {id}
      {glyph && (
        <span className={cn("text-[10px]", indicatorTone(indicator))}>
          {glyph}
        </span>
      )}
    </button>
  );
}

function StageTabButton({
  tab,
  isActive,
  indicator,
  onClick,
  onClose,
  onTogglePin,
}: {
  tab: StageTab;
  isActive: boolean;
  indicator: TabIndicator;
  onClick: () => void;
  onClose: () => void;
  onTogglePin: () => void;
}) {
  const glyph = indicatorGlyph(indicator);
  const label =
    tab.stageName.length > 18
      ? `${tab.stageName.slice(0, 16)}…`
      : tab.stageName;
  return (
    <div
      className={cn(
        "flex shrink-0 items-center gap-1 border-b-2 px-3 py-2.5 text-sm transition-colors",
        isActive
          ? "border-primary text-foreground"
          : "border-transparent text-muted-foreground hover:text-foreground",
      )}
    >
      <button role="tab" aria-selected={isActive} onClick={onClick} className="flex items-center gap-1">
        <span className="text-[10px] text-muted-foreground">Stage:</span>
        <span>{label}</span>
        {glyph && (
          <span className={cn("text-[10px]", indicatorTone(indicator))}>
            {glyph}
          </span>
        )}
      </button>
      {/* Pin toggle */}
      <button
        className={cn(
          "ml-0.5 text-[10px] leading-none",
          tab.pinned ? "text-foreground" : "text-muted-foreground/40 hover:text-muted-foreground",
        )}
        onClick={(e) => {
          e.stopPropagation();
          onTogglePin();
        }}
        title={tab.pinned ? "Unpin tab" : "Pin tab (survives reload)"}
        aria-label={tab.pinned ? "Unpin tab" : "Pin tab"}
      >
        &#128204;
      </button>
      {/* Close — hidden for pinned tabs */}
      {!tab.pinned && (
        <button
          className="ml-0.5 text-[10px] text-muted-foreground/40 hover:text-muted-foreground leading-none"
          onClick={(e) => {
            e.stopPropagation();
            onClose();
          }}
          aria-label={`Close Stage: ${tab.stageName}`}
        >
          ✕
        </button>
      )}
    </div>
  );
}
