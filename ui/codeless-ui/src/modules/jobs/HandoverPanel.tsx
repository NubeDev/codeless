import { useCallback, useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { RpcError, useRpc, type Job } from "@/lib/rpc";
import type { Handover as HandoverShape } from "@/lib/rpc/wire";

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
//
// "missing" and "ready" both expose an editor affordance so the user
// can seed (or rewrite) the handover from the UI. Editing commits
// through `write_handover`, which is the only RPC that writes the
// per-run handover; the runner takes over after.
export function HandoverPanel({ job }: Props) {
  const rpc = useRpc();
  const [state, setState] = useState<State>({ kind: "idle" });
  const [editing, setEditing] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);

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
          const result = (await rpc.call("fs_read_file", {
            repo_id: job.repo_id,
            path,
          })) as unknown as
            | { kind: "text"; content: string }
            | { kind: "binary" }
            | { kind: "toolarge"; limit: number }
            | { content: string };
          if (cancelled) return;
          // The current server returns `{content: string}` (flat,
          // always text). The kind-tagged variants are for a future
          // build that classifies binary / over-limit separately;
          // until that ships we treat flat `content` as text.
          if (!("kind" in result)) {
            setState({ kind: "ready", path, content: result.content });
            return;
          }
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
  }, [job.id, job.worktree_path, rpc, reloadKey]);

  const seed = useMemo<HandoverShape>(() => {
    if (state.kind === "ready") {
      const parsed = parseHandover(state.content);
      return {
        done: parsed.done,
        next: parsed.next,
        what_you_need_to_know: parsed.what_you_need_to_know,
        open_questions: parsed.open_questions,
      };
    }
    return { done: [], next: [], what_you_need_to_know: [], open_questions: [] };
  }, [state]);

  if (editing) {
    return (
      <HandoverEditor
        jobId={job.id}
        seed={seed}
        canWrite={!!job.worktree_path}
        onCancel={() => setEditing(false)}
        onSaved={() => {
          setEditing(false);
          setReloadKey((k) => k + 1);
        }}
      />
    );
  }

  return (
    <ScrollArea className="flex-1">
      <div className="p-4">
        {state.kind === "idle" || state.kind === "loading" ? (
          <div className="text-muted-foreground text-sm">loading…</div>
        ) : state.kind === "missing" ? (
          <MissingNotice
            tried={state.tried}
            hasWorktree={!!job.worktree_path}
            onCreate={() => setEditing(true)}
          />
        ) : state.kind === "error" ? (
          <div className="text-destructive text-sm">{state.message}</div>
        ) : (
          <Handover
            content={state.content}
            path={state.path}
            onEdit={() => setEditing(true)}
          />
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

function Handover({
  content,
  path,
  onEdit,
}: {
  content: string;
  path: string;
  onEdit: () => void;
}) {
  const sections = parseHandover(content);
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <div className="text-muted-foreground font-mono text-[10px]" title={path}>
          {path}
        </div>
        <Button size="sm" variant="outline" className="h-6 px-2 text-xs" onClick={onEdit}>
          Edit
        </Button>
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
  onCreate,
}: {
  tried: string[];
  hasWorktree: boolean;
  onCreate: () => void;
}) {
  return (
    <div className="text-muted-foreground space-y-3 text-sm">
      <div className="flex items-center justify-between">
        <p>No handover yet for this job.</p>
        {hasWorktree && (
          <Button size="sm" variant="outline" onClick={onCreate}>
            Create handover
          </Button>
        )}
      </div>
      {!hasWorktree && (
        <p className="text-xs">
          The job has no worktree path on disk — handover files live under
          the worktree, so nothing to preview or seed until the runner
          provisions one.
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

interface EditorProps {
  jobId: string;
  seed: HandoverShape;
  canWrite: boolean;
  onCancel: () => void;
  onSaved: () => void;
}

// Structured handover editor: four bullet lists, one per JOB-MODEL.md
// section. Save round-trips through `write_handover`, which validates
// worktree presence server-side; the panel disables Save when the job
// has no worktree, but the runtime is the final gate.
function HandoverEditor({
  jobId,
  seed,
  canWrite,
  onCancel,
  onSaved,
}: EditorProps) {
  const rpc = useRpc();
  const [done, setDone] = useState<string>(seed.done.join("\n"));
  const [next, setNext] = useState<string>(seed.next.join("\n"));
  const [knowledge, setKnowledge] = useState<string>(
    seed.what_you_need_to_know.join("\n"),
  );
  const [questions, setQuestions] = useState<string>(seed.open_questions.join("\n"));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const onSave = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await rpc.call("write_handover", {
        job_id: jobId,
        handover: {
          done: linesToItems(done),
          next: linesToItems(next),
          what_you_need_to_know: linesToItems(knowledge),
          open_questions: linesToItems(questions),
        },
      });
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [rpc, jobId, done, next, knowledge, questions, onSaved]);

  return (
    <div className="flex h-full flex-col">
      <div className="border-border/40 flex items-center gap-2 border-b px-3 py-2 text-xs">
        <span className="font-mono">handover editor</span>
        <span className="text-muted-foreground">— one item per line</span>
        <div className="ml-auto flex gap-2">
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-xs"
            onClick={onCancel}
            disabled={busy}
          >
            Cancel
          </Button>
          <Button
            size="sm"
            className="h-6 px-2 text-xs"
            onClick={() => void onSave()}
            disabled={busy || !canWrite}
          >
            Save
          </Button>
        </div>
      </div>
      {error && (
        <div className="border-destructive/40 bg-destructive/10 text-destructive border-b px-3 py-2 text-xs">
          {error}
        </div>
      )}
      {!canWrite && (
        <div className="border-border/40 bg-muted/20 text-muted-foreground border-b px-3 py-2 text-xs">
          No worktree yet — Save is disabled until the runner provisions
          one.
        </div>
      )}
      <ScrollArea className="min-h-0 flex-1">
        <div className="space-y-4 p-4">
          <HandoverField label="Done" value={done} onChange={setDone} disabled={busy} />
          <HandoverField label="Next" value={next} onChange={setNext} disabled={busy} />
          <HandoverField
            label="What you need to know"
            value={knowledge}
            onChange={setKnowledge}
            disabled={busy}
          />
          <HandoverField
            label="Open questions"
            value={questions}
            onChange={setQuestions}
            disabled={busy}
          />
        </div>
      </ScrollArea>
    </div>
  );
}

function HandoverField({
  label,
  value,
  onChange,
  disabled,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  disabled: boolean;
}) {
  return (
    <div>
      <label className="text-muted-foreground mb-1 block text-[10px] uppercase">
        {label}
      </label>
      <textarea
        className="border-border/60 h-24 w-full resize-none rounded border bg-transparent p-2 text-xs leading-snug outline-none"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled}
        spellCheck={false}
        placeholder="One item per line."
      />
    </div>
  );
}

// Split a textarea body into bullet items. Blank lines drop; leading
// `- ` or `* ` is forgiven (so paste-from-markdown round-trips).
function linesToItems(s: string): string[] {
  return s
    .split(/\r?\n/)
    .map((line) => line.replace(/^\s*[-*]\s+/, "").trim())
    .filter((line) => line.length > 0);
}
