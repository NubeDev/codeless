import { cn } from "@/lib/utils";
import type { PreCheckOutcome, ReviewVerdict } from "@/lib/rpc";

// Surface A from `DOCS/SCOPE-MUTABLE-UI-DECISIONS.md` — the
// editor-facing summary of one REVIEW gate. Reads the most-recent
// `ReviewPreCheck` and `ReviewVerdict` events the parent has already
// folded out of the SSE stream and renders them as a compact panel:
// the pre-check outcome (path-by-path verified / missing), the
// verdict + reason, and (when the runtime advertises the
// scope-patch handover round-trip capability) a `Patches proposed: N`
// counter. The raw events stay in the timeline; this panel is the
// summary, not a duplicate (decision OQ#6).
//
// `patchCounterEnabled` is gated on `ServerInfo.feature_flags
// .scope_patch_handover_round_trip`. While the runtime cannot
// guarantee SCOPE-PATCH markers survive the next-stage handover,
// the row is omitted (not zeroed) per OQ#1 — a counter that lies
// once per missed proposal is worse than no counter.
interface Props {
  precheck: PreCheckOutcome | null;
  verdict: ReviewVerdict | null;
  patchesProposed: number;
  patchCounterEnabled: boolean;
}

export function ReviewGatePanel({
  precheck,
  verdict,
  patchesProposed,
  patchCounterEnabled,
}: Props) {
  // Empty panel before the first event arrives stays useful as a
  // placeholder — the editor knows the stage is a REVIEW gate even
  // before the runtime emits anything (e.g. the stage is queued).
  return (
    <div className="space-y-3">
      <PreCheckRow outcome={precheck} />
      <VerdictRow verdict={verdict} />
      {patchCounterEnabled && <PatchesRow count={patchesProposed} />}
    </div>
  );
}

// ------------------------------------------------------------------ pre-check

function PreCheckRow({ outcome }: { outcome: PreCheckOutcome | null }) {
  if (outcome === null) {
    return (
      <Row label="pre-check" tone="muted" glyph="○">
        <span className="text-muted-foreground">awaiting</span>
      </Row>
    );
  }
  switch (outcome.outcome) {
    case "pass":
      return (
        <Row label="pre-check" tone="ok" glyph="✓">
          <span>verified {outcome.verified.length} path{outcome.verified.length === 1 ? "" : "s"}</span>
          {outcome.verified.length > 0 && <PathList paths={outcome.verified} tone="muted" />}
        </Row>
      );
    case "fail":
      return (
        <Row label="pre-check" tone="bad" glyph="!">
          <span>missing {outcome.missing.length} path{outcome.missing.length === 1 ? "" : "s"}</span>
          {outcome.missing.length > 0 && <PathList paths={outcome.missing} tone="bad" />}
        </Row>
      );
    case "skipped":
      return (
        <Row label="pre-check" tone="muted" glyph="—">
          <span>skipped — no prior handover or worktree to verify against</span>
        </Row>
      );
    case "nothing-to-verify":
      return (
        <Row label="pre-check" tone="muted" glyph="—">
          <span>nothing to verify — prior handover named no path-shaped tokens</span>
        </Row>
      );
  }
}

// ------------------------------------------------------------------ verdict

function VerdictRow({ verdict }: { verdict: ReviewVerdict | null }) {
  if (verdict === null) {
    return (
      <Row label="verdict" tone="muted" glyph="○">
        <span className="text-muted-foreground">awaiting</span>
      </Row>
    );
  }
  switch (verdict.verdict) {
    case "pass":
      return (
        <Row label="verdict" tone="ok" glyph="✓">
          <span className="font-medium">PASS</span>
          {verdict.reason && <span className="text-muted-foreground"> — {verdict.reason}</span>}
        </Row>
      );
    case "fail":
      return (
        <Row label="verdict" tone="bad" glyph="!">
          <span className="font-medium">FAIL</span>
          {verdict.reason && <span className="text-muted-foreground"> — {verdict.reason}</span>}
        </Row>
      );
    case "auto-fail":
      return (
        <Row label="verdict" tone="bad" glyph="!">
          <span className="font-medium">AUTO-FAIL</span>
          {verdict.reason && <span className="text-muted-foreground"> — {verdict.reason}</span>}
        </Row>
      );
  }
}

// ------------------------------------------------------------------ patches

function PatchesRow({ count }: { count: number }) {
  return (
    <Row
      label="patches"
      tone={count > 0 ? "ok" : "muted"}
      glyph={count > 0 ? "✓" : "○"}
    >
      <span>{count} proposed</span>
    </Row>
  );
}

// ------------------------------------------------------------------ shared

type Tone = "ok" | "bad" | "muted";

function toneClass(tone: Tone): string {
  switch (tone) {
    case "ok":
      return "text-emerald-600 dark:text-emerald-400";
    case "bad":
      return "text-destructive";
    case "muted":
      return "text-muted-foreground";
  }
}

function Row({
  label,
  tone,
  glyph,
  children,
}: {
  label: string;
  tone: Tone;
  glyph: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1">
      <div className="flex items-baseline gap-2 text-xs">
        <span className="text-muted-foreground w-20 shrink-0 text-[10px] uppercase tracking-wider">
          {label}
        </span>
        <span className={cn("font-mono", toneClass(tone))}>{glyph}</span>
        <span className="min-w-0 flex-1">{children}</span>
      </div>
    </div>
  );
}

function PathList({ paths, tone }: { paths: string[]; tone: Tone }) {
  return (
    <ul className="ml-4 mt-1 list-disc space-y-0.5 text-[11px]">
      {paths.map((p, i) => (
        <li key={`${p}-${i}`} className={cn("font-mono", toneClass(tone))}>
          {p}
        </li>
      ))}
    </ul>
  );
}
