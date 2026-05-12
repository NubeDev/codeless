import { useEffect, useState } from "react";

import { ScrollArea } from "@/components/ui/scroll-area";
import { RpcError, useRpc, type Job } from "@/lib/rpc";

interface Props {
  job: Job;
}

type State =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "missing"; tried: string[] }
  | { kind: "ready"; path: string; content: string }
  | { kind: "error"; message: string };

// Best-effort preview of a job's `handover.md` (the JOB-MODEL.md
// session contract — see DOCS/JOB-MODEL.md). The runtime does not
// yet write these automatically for every job, so the panel must
// degrade cleanly to "not found" rather than treating absence as
// an error. Two locations are probed in priority order:
//   1. <worktree>/runs/<job_id>/handover.md  — the canonical layout
//   2. <worktree>/handover.md                — ad-hoc demos at the root
// `fs_read_file` returns `not_found` if the path is missing; any
// other error (binary content, transport, permissions) surfaces
// verbatim so the operator can tell what went wrong.
export function HandoverPanel({ job }: Props) {
  const rpc = useRpc();
  const [state, setState] = useState<State>({ kind: "idle" });

  useEffect(() => {
    if (!job.worktree_path) {
      setState({ kind: "missing", tried: [] });
      return;
    }
    let cancelled = false;
    setState({ kind: "loading" });
    const candidates = [
      `${job.worktree_path}/runs/${job.id}/handover.md`,
      `${job.worktree_path}/handover.md`,
    ];
    void (async () => {
      let lastError: string | null = null;
      for (const path of candidates) {
        try {
          const result = await rpc.call("fs_read_file", {
            path,
            byte_limit: null,
          });
          if (cancelled) return;
          if (result.kind === "text") {
            setState({ kind: "ready", path, content: result.content });
            return;
          }
          if (result.kind === "binary") {
            lastError = "handover.md is binary, not utf-8 text";
            continue;
          }
          if (result.kind === "toolarge") {
            lastError = `handover.md exceeds the ${result.limit}-byte read limit`;
            continue;
          }
        } catch (e) {
          if (cancelled) return;
          if (e instanceof RpcError && e.kind === "not_found") continue;
          lastError = e instanceof Error ? e.message : String(e);
        }
      }
      if (cancelled) return;
      if (lastError) {
        setState({ kind: "error", message: lastError });
      } else {
        setState({ kind: "missing", tried: candidates });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [job.id, job.worktree_path, rpc]);

  return (
    <ScrollArea className="flex-1">
      <div className="p-4">
        {state.kind === "idle" || state.kind === "loading" ? (
          <div className="text-muted-foreground text-sm">loading…</div>
        ) : state.kind === "missing" ? (
          <MissingNotice tried={state.tried} hasWorktree={!!job.worktree_path} />
        ) : state.kind === "error" ? (
          <div className="text-destructive text-sm">{state.message}</div>
        ) : (
          <Handover content={state.content} path={state.path} />
        )}
      </div>
    </ScrollArea>
  );
}

// Mirror of `codeless_types::Handover::from_markdown` in TS. Kept
// inline (rather than a shared lib) because the only consumer today
// is this panel; if a second consumer shows up, factor it out then.
// Unknown sections and prose between bullets are silently dropped —
// the four canonical sections are the contract, anything else is
// noise we tolerate from a partial run.
type Section =
  | "done"
  | "next"
  | "what_you_need_to_know"
  | "open_questions";

const SECTION_TITLE: Record<Section, string> = {
  done: "Done",
  next: "Next",
  what_you_need_to_know: "What you need to know",
  open_questions: "Open questions",
};

const SECTION_ORDER: Section[] = [
  "done",
  "next",
  "what_you_need_to_know",
  "open_questions",
];

function parseHandover(md: string): Record<Section, string[]> {
  const out: Record<Section, string[]> = {
    done: [],
    next: [],
    what_you_need_to_know: [],
    open_questions: [],
  };
  let current: Section | null = null;
  for (const raw of md.split(/\r?\n/)) {
    const line = raw.trimEnd();
    const heading = line.match(/^#{1,3}\s+(.+?)\s*$/);
    if (heading) {
      const name = heading[1].toLowerCase();
      if (name === "done") current = "done";
      else if (name === "next") current = "next";
      else if (name === "what you need to know")
        current = "what_you_need_to_know";
      else if (name === "open questions") current = "open_questions";
      else current = null;
      continue;
    }
    if (current === null) continue;
    const bullet = line.match(/^\s*[-*]\s+(.*)$/);
    if (!bullet) continue;
    const item = bullet[1].trim();
    if (!item || item === "(none)") continue;
    out[current].push(item);
  }
  return out;
}

function Handover({ content, path }: { content: string; path: string }) {
  const sections = parseHandover(content);
  return (
    <div className="space-y-3">
      <div className="text-muted-foreground font-mono text-[10px]" title={path}>
        {path}
      </div>
      {SECTION_ORDER.map((s) => (
        <HandoverSection
          key={s}
          title={SECTION_TITLE[s]}
          items={sections[s]}
        />
      ))}
    </div>
  );
}

function HandoverSection({
  title,
  items,
}: {
  title: string;
  items: string[];
}) {
  return (
    <section>
      <h3 className="text-foreground mb-1.5 text-xs font-semibold uppercase tracking-wide">
        {title}
      </h3>
      {items.length === 0 ? (
        <p className="text-muted-foreground/70 text-xs italic">none</p>
      ) : (
        <ul className="text-foreground/90 list-disc space-y-0.5 pl-5 text-sm leading-snug">
          {items.map((item, i) => (
            <li key={i} className="whitespace-pre-wrap">
              {item}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function MissingNotice({
  tried,
  hasWorktree,
}: {
  tried: string[];
  hasWorktree: boolean;
}) {
  return (
    <div className="text-muted-foreground space-y-2 text-sm">
      <p>No handover yet for this job.</p>
      {!hasWorktree && (
        <p className="text-xs">
          The job has no worktree path on disk — handover files live under
          the worktree, so nothing to preview until the runner provisions one.
        </p>
      )}
      {tried.length > 0 && (
        <div className="text-xs">
          <p>Looked under:</p>
          <ul className="mt-1 list-disc pl-5">
            {tried.map((p) => (
              <li key={p} className="font-mono text-[11px]">
                {p}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
