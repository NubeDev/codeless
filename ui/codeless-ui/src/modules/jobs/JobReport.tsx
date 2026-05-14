import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { useRpc, type JobId, type JobReportResult } from "@/lib/rpc";

type Props = { jobId: JobId };

export function JobReport({ jobId }: Props) {
  const rpc = useRpc();
  const [report, setReport] = useState<JobReportResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const fetchReport = useCallback(
    async (showSpinner: boolean) => {
      if (showSpinner) setLoading(true);
      else setRefreshing(true);
      setErr(null);
      try {
        const r = await rpc.call("job_report", { job_id: jobId });
        setReport(r);
      } catch (e) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
        setRefreshing(false);
      }
    },
    [rpc, jobId],
  );

  useEffect(() => {
    void fetchReport(true);
  }, [fetchReport]);

  if (loading) {
    return (
      <div className="text-muted-foreground p-3 text-xs">loading report…</div>
    );
  }
  if (err) {
    return (
      <div className="space-y-2 p-3 text-xs">
        <div className="text-destructive">{err}</div>
        <Button size="sm" variant="outline" onClick={() => fetchReport(true)}>
          retry
        </Button>
      </div>
    );
  }
  if (!report) return null;

  return (
    <div className="space-y-3 p-3 text-xs">
      <div className="flex items-center justify-between">
        <div className="text-muted-foreground text-[11px] uppercase tracking-wide">
          Job report
        </div>
        <Button
          size="sm"
          variant="ghost"
          disabled={refreshing}
          onClick={() => fetchReport(false)}
          className="h-6 px-2 text-[11px]"
        >
          {refreshing ? "refreshing…" : "refresh"}
        </Button>
      </div>

      <HeaderGrid r={report} />
      <StagesTable r={report} />
      <TurnsTable r={report} />
      <ToolCalls r={report} />
      <SpecChanges r={report} />
      <EventTally r={report} />
      <CopyMarkdownButton r={report} />
    </div>
  );
}

function HeaderGrid({ r }: { r: JobReportResult }) {
  const spend = fmtUsd(r.cost_cents);
  const cap = fmtUsd(r.cost_cap_cents);
  const pct = r.cost_cap_cents > 0
    ? Math.round((r.cost_cents / r.cost_cap_cents) * 100)
    : 0;
  return (
    <div className="grid grid-cols-2 gap-x-3 gap-y-1">
      <Cell label="Status" value={`${r.status}${r.stop_reason ? ` (${r.stop_reason})` : ""}`} />
      <Cell label="Wall clock" value={fmtDuration(r.wall_clock_ms)} />
      <Cell label="Spend" value={`${spend} of ${cap} (${pct}%)`} />
      <Cell label="Stages" value={String(r.stages.length)} />
    </div>
  );
}

function Cell({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col">
      <span className="text-muted-foreground text-[10px] uppercase tracking-wide">
        {label}
      </span>
      <span className="font-mono">{value}</span>
    </div>
  );
}

function StagesTable({ r }: { r: JobReportResult }) {
  if (r.stages.length === 0) return null;
  return (
    <div className="space-y-1">
      <div className="text-muted-foreground text-[11px] uppercase tracking-wide">
        Stages ({r.stages.length})
      </div>
      <table className="w-full text-[11px]">
        <thead className="text-muted-foreground">
          <tr className="text-left">
            <th className="py-0.5 pr-2 font-medium">#</th>
            <th className="py-0.5 pr-2 font-medium">status</th>
            <th className="py-0.5 pr-2 font-medium">session</th>
            <th className="py-0.5 pr-2 font-medium">cost</th>
            <th className="py-0.5 pr-2 font-medium">dur</th>
          </tr>
        </thead>
        <tbody>
          {r.stages.map((s, i) => (
            <tr key={i} className="font-mono">
              <td className="py-0.5 pr-2">
                {s.ordinal}
                {s.attempt > 0 ? `·${s.attempt}` : ""}
              </td>
              <td className="py-0.5 pr-2">{s.status}</td>
              <td className="py-0.5 pr-2 opacity-70">
                {s.session_id ? s.session_id.slice(0, 8) : "—"}
              </td>
              <td className="py-0.5 pr-2">{fmtUsd(s.cost_cents)}</td>
              <td className="py-0.5 pr-2">{fmtDuration(s.duration_ms)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function TurnsTable({ r }: { r: JobReportResult }) {
  if (r.turns.length === 0) return null;
  const total = r.turns.reduce((sum, t) => sum + t.cost_cents, 0);
  return (
    <div className="space-y-1">
      <div className="text-muted-foreground text-[11px] uppercase tracking-wide">
        Claude turns ({r.turns.length}) — total {fmtUsd(total)}
      </div>
      <table className="w-full text-[11px]">
        <thead className="text-muted-foreground">
          <tr className="text-left">
            <th className="py-0.5 pr-2 font-medium">task</th>
            <th className="py-0.5 pr-2 font-medium">stage</th>
            <th className="py-0.5 pr-2 font-medium">cost</th>
            <th className="py-0.5 pr-2 font-medium">tok in/out</th>
          </tr>
        </thead>
        <tbody>
          {r.turns.map((t) => (
            <tr key={t.task_id} className="font-mono">
              <td className="py-0.5 pr-2 opacity-70">{t.task_id.slice(0, 12)}</td>
              <td className="py-0.5 pr-2">
                {t.stage_ordinal == null ? "chat" : `s${t.stage_ordinal}`}
              </td>
              <td className="py-0.5 pr-2">{fmtUsd(t.cost_cents)}</td>
              <td className="py-0.5 pr-2 opacity-70">
                {t.input_tokens}/{t.output_tokens}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ToolCalls({ r }: { r: JobReportResult }) {
  if (r.tool_calls.length === 0) return null;
  const total = r.tool_calls.reduce((s, t) => s + t.count, 0);
  return (
    <div className="space-y-1">
      <div className="text-muted-foreground text-[11px] uppercase tracking-wide">
        Tool calls ({total})
      </div>
      <div className="flex flex-wrap gap-x-3 gap-y-0.5 font-mono text-[11px]">
        {r.tool_calls.map((t) => (
          <span key={t.tool}>
            {t.tool || "<unknown>"}: {t.count}
          </span>
        ))}
      </div>
    </div>
  );
}

function SpecChanges({ r }: { r: JobReportResult }) {
  if (r.spec_changes.length === 0) return null;
  const total = r.spec_changes.reduce((s, c) => s + c.count, 0);
  return (
    <div className="space-y-1">
      <div className="text-muted-foreground text-[11px] uppercase tracking-wide">
        Spec changes ({total})
      </div>
      <div className="flex flex-wrap gap-x-3 gap-y-0.5 font-mono text-[11px]">
        {r.spec_changes.map((c, i) => {
          const label =
            c.kind === "template"
              ? "template.yaml"
              : (c.filename ?? "<unknown>");
          return (
            <span key={`${c.kind}:${c.filename ?? ""}:${i}`}>
              {label}: {c.count}
            </span>
          );
        })}
      </div>
    </div>
  );
}

function EventTally({ r }: { r: JobReportResult }) {
  if (r.event_tally.length === 0) return null;
  return (
    <details className="text-[11px]">
      <summary className="text-muted-foreground cursor-pointer uppercase tracking-wide">
        Event tally ({r.event_tally.reduce((s, e) => s + e.count, 0)} events)
      </summary>
      <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 font-mono">
        {r.event_tally.map((e) => (
          <span key={e.kind}>
            {e.kind}: {e.count}
          </span>
        ))}
      </div>
    </details>
  );
}

function CopyMarkdownButton({ r }: { r: JobReportResult }) {
  const [copied, setCopied] = useState(false);
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(toMarkdown(r));
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // clipboard may be denied; ignore silently — the markdown is
      // still inspectable via the underlying RPC for the operator
      // who really needs it.
    }
  };
  return (
    <div>
      <Button
        size="sm"
        variant="outline"
        onClick={onCopy}
        className="h-6 px-2 text-[11px]"
      >
        {copied ? "copied!" : "copy as markdown"}
      </Button>
    </div>
  );
}

function fmtUsd(cents: number): string {
  return `$${(cents / 100).toFixed(2)}`;
}

function fmtDuration(ms: number | null): string {
  if (ms == null) return "—";
  const total = Math.floor(ms / 1000);
  const min = Math.floor(total / 60);
  const sec = total % 60;
  return min > 0 ? `${min}m ${String(sec).padStart(2, "0")}s` : `${sec}s`;
}

function toMarkdown(r: JobReportResult): string {
  const lines: string[] = [];
  lines.push(`# Job report — ${r.job_id}`);
  lines.push("");
  lines.push(`- Status: **${r.status}**${r.stop_reason ? ` (${r.stop_reason})` : ""}`);
  lines.push(`- Spend: **${fmtUsd(r.cost_cents)}** of ${fmtUsd(r.cost_cap_cents)}`);
  lines.push(`- Wall clock: ${fmtDuration(r.wall_clock_ms)}`);
  lines.push("");
  if (r.stages.length > 0) {
    lines.push("## Stages");
    lines.push("");
    lines.push("| # | status | session | cost | duration |");
    lines.push("|---|---|---|---|---|");
    for (const s of r.stages) {
      const num = s.attempt > 0 ? `${s.ordinal}·${s.attempt}` : String(s.ordinal);
      const session = s.session_id ? s.session_id.slice(0, 8) : "—";
      lines.push(
        `| ${num} | ${s.status} | \`${session}\` | ${fmtUsd(s.cost_cents)} | ${fmtDuration(s.duration_ms)} |`,
      );
    }
    lines.push("");
  }
  if (r.turns.length > 0) {
    const total = r.turns.reduce((sum, t) => sum + t.cost_cents, 0);
    lines.push(`## Claude turns (${r.turns.length}, total ${fmtUsd(total)})`);
    lines.push("");
    lines.push("| task | stage | cost | tok in/out |");
    lines.push("|---|---|---|---|");
    for (const t of r.turns) {
      const stage = t.stage_ordinal == null ? "chat" : `s${t.stage_ordinal}`;
      lines.push(
        `| \`${t.task_id.slice(0, 12)}\` | ${stage} | ${fmtUsd(t.cost_cents)} | ${t.input_tokens}/${t.output_tokens} |`,
      );
    }
    lines.push("");
  }
  if (r.tool_calls.length > 0) {
    const total = r.tool_calls.reduce((s, t) => s + t.count, 0);
    lines.push(`## Tool calls (${total})`);
    lines.push("");
    for (const t of r.tool_calls) {
      lines.push(`- \`${t.tool || "<unknown>"}\`: ${t.count}`);
    }
    lines.push("");
  }
  if (r.spec_changes.length > 0) {
    const total = r.spec_changes.reduce((s, c) => s + c.count, 0);
    lines.push(`## Spec changes (${total})`);
    lines.push("");
    for (const c of r.spec_changes) {
      const label =
        c.kind === "template" ? "template.yaml" : (c.filename ?? "<unknown>");
      lines.push(`- \`${label}\`: ${c.count}`);
    }
    lines.push("");
  }
  if (r.event_tally.length > 0) {
    lines.push("## Event tally");
    lines.push("");
    for (const e of r.event_tally) {
      lines.push(`- \`${e.kind}\`: ${e.count}`);
    }
  }
  return lines.join("\n");
}
