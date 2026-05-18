import { useMemo, useState } from "react";

import { prettyJson, wallClockTime } from "./format";

// One tool call, collapsed by default. Shows tool name + a one-line
// argument summary (`Edit …/stage.rs`, `Bash <cmd>`) so the user can
// scan a long run without expanding every card. Click to reveal a
// Copilot-style preview: a +/- diff for Edit/MultiEdit, the new file
// body for Write, and the raw pretty-printed args JSON as a fallback
// for everything else.
export function ToolCallCard({
  tool,
  argsJson,
  ts,
}: {
  tool: string;
  argsJson: string;
  ts: number;
}) {
  const [expanded, setExpanded] = useState(false);
  const summary = toolCallSummary(tool, argsJson);
  const preview = useMemo(
    () => editPreviewFromArgs(tool, argsJson),
    [tool, argsJson],
  );
  return (
    <li>
      <button
        type="button"
        onClick={() => setExpanded((e) => !e)}
        className="border-border/40 bg-muted/20 hover:bg-muted/40 flex w-full items-center justify-between gap-2 rounded border px-2 py-1.5 text-left text-[11px] transition-colors"
      >
        <span className="min-w-0 flex-1 truncate">
          <span className="text-muted-foreground mr-1.5">tool</span>
          <span className="font-mono font-medium">{tool}</span>
          {summary && (
            <span className="text-muted-foreground ml-2 truncate">
              {summary}
            </span>
          )}
          {preview && preview.totals && (
            <span className="ml-2 font-mono text-[10px]">
              <span className="text-emerald-500">+{preview.totals.adds}</span>{" "}
              <span className="text-destructive">−{preview.totals.dels}</span>
            </span>
          )}
        </span>
        <span className="text-muted-foreground shrink-0 font-mono text-[10px]">
          {wallClockTime(ts)}
        </span>
      </button>
      {expanded && preview && <EditPreviewBody preview={preview} />}
      {expanded && !preview && (
        <pre className="border-border/40 bg-background/60 mt-1 max-h-72 overflow-auto rounded border px-2 py-1.5 font-mono text-[10px] whitespace-pre-wrap break-all">
          {prettyJson(argsJson)}
        </pre>
      )}
    </li>
  );
}

// Parsed shape of an edit-style tool call ready for inline render.
// `hunks` is one entry per edit; `Write` / `NotebookEdit` produce a
// single hunk with `dels = []`. Returns null for tools whose args
// don't fit this shape (Bash, Read, Grep, …) so the card falls back
// to the raw-JSON view.
interface EditPreview {
  path: string | null;
  hunks: Array<{ dels: string[]; adds: string[] }>;
  totals: { adds: number; dels: number } | null;
}

function editPreviewFromArgs(
  tool: string,
  argsJson: string,
): EditPreview | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(argsJson);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== "object") return null;
  const args = parsed as Record<string, unknown>;
  const path =
    (typeof args.file_path === "string" && args.file_path) ||
    (typeof args.path === "string" && args.path) ||
    (typeof args.notebook_path === "string" && args.notebook_path) ||
    null;
  const displayPath = path ? relativiseWorktreePath(path) : null;

  const splitLines = (s: string): string[] =>
    s === "" ? [] : s.replace(/\n$/, "").split("\n");

  switch (tool) {
    case "Edit":
    case "NotebookEdit": {
      const oldS = typeof args.old_string === "string" ? args.old_string : null;
      const newS = typeof args.new_string === "string" ? args.new_string : null;
      if (oldS == null || newS == null) return null;
      const dels = splitLines(oldS);
      const adds = splitLines(newS);
      return {
        path: displayPath,
        hunks: [{ dels, adds }],
        totals: { adds: adds.length, dels: dels.length },
      };
    }
    case "MultiEdit": {
      const edits = Array.isArray(args.edits) ? args.edits : null;
      if (!edits) return null;
      const hunks: Array<{ dels: string[]; adds: string[] }> = [];
      let adds = 0;
      let dels = 0;
      for (const e of edits) {
        if (!e || typeof e !== "object") continue;
        const er = e as Record<string, unknown>;
        const oldS = typeof er.old_string === "string" ? er.old_string : null;
        const newS = typeof er.new_string === "string" ? er.new_string : null;
        if (oldS == null || newS == null) continue;
        const d = splitLines(oldS);
        const a = splitLines(newS);
        hunks.push({ dels: d, adds: a });
        dels += d.length;
        adds += a.length;
      }
      if (hunks.length === 0) return null;
      return { path: displayPath, hunks, totals: { adds, dels } };
    }
    case "Write": {
      const content = typeof args.content === "string" ? args.content : null;
      if (content == null) return null;
      const adds = splitLines(content);
      return {
        path: displayPath,
        hunks: [{ dels: [], adds }],
        totals: { adds: adds.length, dels: 0 },
      };
    }
    default:
      return null;
  }
}

// Render the parsed edit preview as a small unified-diff body. Same
// visual vocabulary as `FilesChanged` so an Edit card in the chat
// reads identically to the same change in the Files tab.
function EditPreviewBody({ preview }: { preview: EditPreview }) {
  return (
    <div className="border-border/40 bg-background/60 mt-1 overflow-hidden rounded border">
      {preview.path && (
        <div className="border-border/40 text-muted-foreground border-b px-2 py-1 font-mono text-[10px]">
          {preview.path}
        </div>
      )}
      <div className="max-h-72 overflow-auto px-2 py-1.5">
        {preview.hunks.map((h, i) => (
          <pre
            key={i}
            className="font-mono text-[10.5px] leading-tight whitespace-pre"
          >
            {i > 0 && (
              <span className="text-muted-foreground bg-muted/30 block">
                @@ edit {i + 1} @@
              </span>
            )}
            {h.dels.map((line, j) => (
              <span key={`d-${j}`} className="text-destructive block">
                {`- ${line || ""}`}
              </span>
            ))}
            {h.adds.map((line, j) => (
              <span key={`a-${j}`} className="text-emerald-500 block">
                {`+ ${line || ""}`}
              </span>
            ))}
          </pre>
        ))}
      </div>
    </div>
  );
}

// Best-effort path shortener: drop everything up to and including the
// worktree root marker, leaving the repo-relative path the user
// thinks in. Falls through unchanged for paths that don't match (the
// model occasionally writes already-relative paths).
function relativiseWorktreePath(absolute: string): string {
  const m = absolute.match(/\.codeless\/worktrees\/job-[^/]+\/(.*)$/);
  return m ? m[1] : absolute;
}

// First non-empty match from common arg keys. Enough to make a
// collapsed tool card useful at a glance; the full args sit one click
// away. Returns "" when the args have no recognisable shape — the
// card still renders, just without the trailing summary line.
function toolCallSummary(tool: string, argsJson: string): string {
  let parsed: unknown;
  try {
    parsed = JSON.parse(argsJson);
  } catch {
    return "";
  }
  if (parsed == null || typeof parsed !== "object") return "";
  const obj = parsed as Record<string, unknown>;
  const path =
    (typeof obj.file_path === "string" && obj.file_path) ||
    (typeof obj.path === "string" && obj.path) ||
    "";
  if (path) {
    // Trim the worktree prefix; keep the last two segments so `mod.rs`
    // files stay disambiguated by their parent dir.
    const idx = path.lastIndexOf("/");
    if (idx < 0) return path;
    const parentIdx = path.lastIndexOf("/", idx - 1);
    return parentIdx < 0 ? path.slice(idx + 1) : `…${path.slice(parentIdx)}`;
  }
  if (tool.toLowerCase() === "bash" && typeof obj.command === "string") {
    const cmd = obj.command.trim().split("\n")[0];
    return cmd.length > 60 ? `${cmd.slice(0, 60)}…` : cmd;
  }
  if (typeof obj.pattern === "string") return obj.pattern;
  if (typeof obj.query === "string") return obj.query;
  return "";
}
