import { wallClockTime } from "./format";

// Centred divider for lifecycle moments (stage started, verify
// passed, job stopped, …). Tone colours the rule and label so the
// user's eye lands on bad / warn moments without having to read every
// divider.
export function LifecycleDivider({
  label,
  tone,
  ts,
}: {
  label: string;
  tone: "neutral" | "good" | "bad" | "warn";
  ts: number;
}) {
  const colour =
    tone === "good"
      ? "text-emerald-600 dark:text-emerald-400 border-emerald-500/30"
      : tone === "bad"
        ? "text-destructive border-destructive/30"
        : tone === "warn"
          ? "text-amber-600 dark:text-amber-400 border-amber-500/40"
          : "text-muted-foreground border-border/50";
  return (
    <li
      className={`flex items-center gap-2 py-1 font-mono text-[10px] uppercase tracking-wide ${colour}`}
    >
      <span className={`h-px flex-1 border-t ${colour}`} />
      <span>{label}</span>
      <span className="text-muted-foreground normal-case tracking-normal">
        {wallClockTime(ts)}
      </span>
      <span className={`h-px flex-1 border-t ${colour}`} />
    </li>
  );
}
