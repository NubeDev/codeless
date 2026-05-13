import { useCallback, useEffect, useMemo, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import {
  useJob,
  useRpc,
  type JobFileEntry,
  type JobId,
  type ListJobFilesResult,
} from "@/lib/rpc";

// Synthetic file-list entry that opens the structured stages editor
// instead of a raw textarea. Always rendered at the top of the list,
// whether or not template.yaml has been written to disk yet — the
// stages editor seeds from the `template_yaml` DB column when the
// directory layout doesn't exist, then takes over the on-disk file
// on first save.
const STAGES_KEY = "__stages__";

// The Spec pane. Two-pane layout matching JOB-DIR.md "The UI":
// file list on the left, an editor on the right. The editor surface
// is a plain monospace textarea — the design calls for CodeMirror,
// but the UI workspace does not yet ship a reusable code-editor
// component, and adding the CodeMirror dependency tree is out of
// scope for this stage. The textarea keeps the contract identical
// (controlled value, save, discard); the editor can be swapped out
// later without changing the file's structure.

interface Props {
  jobId: JobId;
}

const SCOPE_PRESET = `# Scope

What this job is for. Replace this with what success looks like, what
is out of scope, constraints, and deliverables.
`;

const WORKFLOW_PRESET = `# Workflow

How the agent should drive the work. Replace this with sequencing,
what to verify between stages, and what counts as done.
`;

export function JobFilesPane({ jobId }: Props) {
  const rpc = useRpc();
  const { data: job } = useJob(jobId);
  const [listing, setListing] = useState<ListJobFilesResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<string | null>(STAGES_KEY);
  const [buffer, setBuffer] = useState<string>("");
  const [savedContent, setSavedContent] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [showNewDialog, setShowNewDialog] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const result = await rpc.call("list_job_files", { job_id: jobId });
      setListing(result);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [rpc, jobId]);

  useEffect(() => {
    setLoading(true);
    void refresh();
  }, [refresh]);

  // After a refresh, keep the user's current selection sticky. The
  // STAGES_KEY entry is always present, so it's the default landing
  // surface — and a freshly-loaded job with no files lands on the
  // stages editor rather than a blank pane.
  useEffect(() => {
    if (!listing) return;
    if (selected === STAGES_KEY) return;
    const stillExists =
      selected && listing.entries.some((e) => e.name === selected);
    if (stillExists) return;
    setSelected(STAGES_KEY);
  }, [listing, selected]);

  // Load the selected file's content on selection change. The
  // synthetic STAGES_KEY entry does not read through `read_job_file`
  // — the stages editor pulls the YAML directly from `job.template_yaml`
  // (the DB seed) and saves through `update_job_template`, which is
  // the single canonical write path for the spec.
  useEffect(() => {
    if (!selected || selected === STAGES_KEY) {
      setBuffer("");
      setSavedContent("");
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const result = await rpc.call("read_job_file", {
          job_id: jobId,
          filename: selected,
        });
        if (cancelled) return;
        setBuffer(result.content);
        setSavedContent(result.content);
      } catch (e) {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [rpc, jobId, selected]);

  const dirty = buffer !== savedContent;
  const selectedEntry: JobFileEntry | null = useMemo(
    () => listing?.entries.find((e) => e.name === selected) ?? null,
    [listing, selected],
  );

  const hasScope = useMemo(
    () => listing?.entries.some((e) => e.is_scope) ?? false,
    [listing],
  );
  const hasWorkflow = useMemo(
    () => listing?.entries.some((e) => e.is_workflow) ?? false,
    [listing],
  );

  const onSave = useCallback(async () => {
    if (!selected) return;
    setBusy(true);
    try {
      const result = await rpc.call("write_job_file", {
        job_id: jobId,
        filename: selected,
        content: buffer,
      });
      setError(null);
      setSavedContent(buffer);
      await refresh();
      setSelected(result.name);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [rpc, jobId, selected, buffer, refresh]);

  const onDiscard = useCallback(() => {
    setBuffer(savedContent);
  }, [savedContent]);

  const onDelete = useCallback(
    async (name: string) => {
      if (!window.confirm(`Delete ${name}?`)) return;
      setBusy(true);
      try {
        await rpc.call("delete_job_file", {
          job_id: jobId,
          filename: name,
        });
        setError(null);
        if (selected === name) setSelected(null);
        await refresh();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [rpc, jobId, refresh, selected],
  );

  const onCreate = useCallback(
    async (name: string, content: string) => {
      setBusy(true);
      try {
        const result = await rpc.call("write_job_file", {
          job_id: jobId,
          filename: name,
          content,
        });
        setError(null);
        setShowNewDialog(false);
        await refresh();
        setSelected(result.name);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [rpc, jobId, refresh],
  );

  if (loading) {
    return (
      <div className="text-muted-foreground p-4 text-sm italic">loading…</div>
    );
  }
  if (error && !listing) {
    return <div className="text-destructive p-4 text-sm">{error}</div>;
  }
  if (!listing) return null;

  return (
    <div className="flex h-full min-h-0">
      <aside className="border-border/40 flex w-56 min-w-0 flex-col border-r">
        <div className="border-border/40 flex items-center justify-between border-b px-3 py-2">
          <span className="text-muted-foreground text-[10px] uppercase tracking-wide">
            files
          </span>
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-xs"
            onClick={() => setShowNewDialog(true)}
          >
            + file
          </Button>
        </div>

        {listing.layout === "flat" && (
          <div className="border-border/40 text-muted-foreground border-b px-3 py-2 text-xs">
            Legacy flat-YAML layout. Your first save will promote this job
            to a directory so you can add SCOPE / WORKFLOW / docs.
          </div>
        )}
        {listing.layout === "none" && (
          <div className="border-border/40 text-muted-foreground border-b px-3 py-2 text-xs">
            No files yet. Click <em>+ file</em> to add SCOPE.md,
            WORKFLOW.md, or any other markdown.
          </div>
        )}

        <ScrollArea className="min-h-0 flex-1">
          <ul className="py-1">
            <li
              key={STAGES_KEY}
              className={cn(
                "flex items-center justify-between px-3 py-1 text-xs cursor-pointer",
                selected === STAGES_KEY
                  ? "bg-accent text-accent-foreground"
                  : "hover:bg-muted/40",
              )}
              onClick={() => setSelected(STAGES_KEY)}
            >
              <span className="truncate">
                Stages
                <Badge variant="outline" className="ml-2 text-[9px]">
                  spec
                </Badge>
              </span>
            </li>
            {listing.entries
              .filter((e) => !e.is_template)
              .map((entry) => (
                <li
                  key={entry.name}
                  className={cn(
                    "group flex items-center justify-between px-3 py-1 text-xs cursor-pointer",
                    selected === entry.name
                      ? "bg-accent text-accent-foreground"
                      : "hover:bg-muted/40",
                  )}
                  onClick={() => setSelected(entry.name)}
                >
                  <span className="truncate">
                    {entry.name}
                    {entry.is_scope && (
                      <Badge variant="outline" className="ml-2 text-[9px]">
                        scope
                      </Badge>
                    )}
                    {entry.is_workflow && (
                      <Badge variant="outline" className="ml-2 text-[9px]">
                        workflow
                      </Badge>
                    )}
                  </span>
                  <button
                    type="button"
                    className="text-muted-foreground hover:text-destructive ml-2 hidden text-xs group-hover:inline"
                    onClick={(e) => {
                      e.stopPropagation();
                      void onDelete(entry.name);
                    }}
                    aria-label={`Delete ${entry.name}`}
                  >
                    ×
                  </button>
                </li>
              ))}
          </ul>
        </ScrollArea>
      </aside>

      <section className="flex min-w-0 flex-1 flex-col">
        {selected === STAGES_KEY ? (
          <StagesEditor
            jobId={jobId}
            templateYaml={job?.template_yaml ?? null}
            onSaved={() => void refresh()}
          />
        ) : selected && selectedEntry ? (
          <>
            <div className="border-border/40 flex items-center gap-2 border-b px-3 py-2 text-xs">
              <span className="truncate font-mono">{selected}</span>
              <div className="ml-auto flex gap-2">
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-6 px-2 text-xs"
                  onClick={onDiscard}
                  disabled={!dirty || busy}
                >
                  Discard
                </Button>
                <Button
                  size="sm"
                  className="h-6 px-2 text-xs"
                  onClick={() => void onSave()}
                  disabled={!dirty || busy}
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
            <textarea
              className="font-mono flex-1 resize-none border-0 bg-transparent p-3 text-xs leading-snug outline-none"
              value={buffer}
              onChange={(e) => setBuffer(e.target.value)}
              spellCheck={false}
            />
          </>
        ) : (
          <div className="text-muted-foreground p-4 text-sm italic">
            Select a file on the left, or click <em>+ file</em>.
          </div>
        )}
      </section>

      {showNewDialog && (
        <NewFileDialog
          hasScope={hasScope}
          hasWorkflow={hasWorkflow}
          onClose={() => setShowNewDialog(false)}
          onCreate={onCreate}
        />
      )}
    </div>
  );
}

interface StagesEditorProps {
  jobId: JobId;
  // The YAML the runtime currently believes is canonical. May be
  // `null` for prompt-only jobs — the editor renders a friendly
  // "not a template-style job" state in that case.
  templateYaml: string | null;
  onSaved: () => void;
}

interface StageRow {
  // Stable client-side id so React keys survive reorders.
  uid: number;
  title: string;
  isReview: boolean;
}

let stageUid = 0;
const mkRow = (title: string, isReview: boolean): StageRow => ({
  uid: ++stageUid,
  title,
  isReview,
});

// Structured spec editor. `name`, `goal`, and an ordered list of
// stages with an optional REVIEW prefix are the entire authoring
// surface (matches `JobTemplate` server-side). Save serialises back
// to YAML and round-trips through `update_job_template`, which is
// the only path that mutates `template.yaml` — `write_job_file`
// rejects it as reserved.
function StagesEditor({ jobId, templateYaml, onSaved }: StagesEditorProps) {
  const rpc = useRpc();
  const [seedKey, setSeedKey] = useState<string>("");
  const [name, setName] = useState("");
  const [goal, setGoal] = useState("");
  const [stages, setStages] = useState<StageRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (templateYaml === null) {
      setName("");
      setGoal("");
      setStages([]);
      setSeedKey("");
      return;
    }
    if (templateYaml === seedKey) return;
    const parsed = parseTemplate(templateYaml);
    setName(parsed.name);
    setGoal(parsed.goal);
    setStages(parsed.stages.map((s) => mkRow(s.title, s.isReview)));
    setSeedKey(templateYaml);
  }, [templateYaml, seedKey]);

  const dirty = useMemo(() => {
    if (templateYaml === null) return false;
    return serialise(name, goal, stages) !== templateYaml;
  }, [name, goal, stages, templateYaml]);

  const onSave = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const yaml = serialise(name, goal, stages);
      await rpc.call("update_job_template", {
        job_id: jobId,
        template_yaml: yaml,
      });
      setSeedKey(yaml);
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [rpc, jobId, name, goal, stages, onSaved]);

  const onDiscard = useCallback(() => {
    if (templateYaml === null) return;
    const parsed = parseTemplate(templateYaml);
    setName(parsed.name);
    setGoal(parsed.goal);
    setStages(parsed.stages.map((s) => mkRow(s.title, s.isReview)));
  }, [templateYaml]);

  if (templateYaml === null) {
    return (
      <div className="text-muted-foreground p-4 text-sm italic">
        This job has no template — it was submitted with a raw prompt,
        not a multi-stage spec. The Spec pane only edits template-style
        jobs.
      </div>
    );
  }

  return (
    <div className="flex h-full min-w-0 flex-col">
      <div className="border-border/40 flex items-center gap-2 border-b px-3 py-2 text-xs">
        <span className="font-mono">stages editor</span>
        <span className="text-muted-foreground">— edits template.yaml</span>
        <div className="ml-auto flex gap-2">
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-xs"
            onClick={onDiscard}
            disabled={!dirty || busy}
          >
            Discard
          </Button>
          <Button
            size="sm"
            className="h-6 px-2 text-xs"
            onClick={() => void onSave()}
            disabled={!dirty || busy}
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
      <ScrollArea className="min-h-0 flex-1">
        <div className="space-y-3 p-3">
          <div>
            <label className="text-muted-foreground mb-1 block text-[10px] uppercase">
              name
            </label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-job"
              disabled={busy}
            />
            <p className="text-muted-foreground mt-1 text-[10px]">
              The job-directory slug. Renames are refused; submit a fresh
              job to change.
            </p>
          </div>
          <div>
            <label className="text-muted-foreground mb-1 block text-[10px] uppercase">
              goal
            </label>
            <Input
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              placeholder="One sentence on what this job is for."
              disabled={busy}
            />
          </div>
          <div>
            <div className="text-muted-foreground mb-1 flex items-center justify-between text-[10px] uppercase">
              <span>stages</span>
              <Button
                size="sm"
                variant="ghost"
                className="h-5 px-2 text-[10px]"
                onClick={() =>
                  setStages((rows) => [...rows, mkRow("", false)])
                }
                disabled={busy}
              >
                + add stage
              </Button>
            </div>
            <ul className="space-y-1">
              {stages.map((row, i) => (
                <li
                  key={row.uid}
                  className="border-border/40 flex items-center gap-1 rounded border px-2 py-1"
                >
                  <span className="text-muted-foreground w-5 text-right font-mono text-[10px]">
                    {i + 1}.
                  </span>
                  <button
                    type="button"
                    className={cn(
                      "rounded px-1.5 text-[10px] uppercase",
                      row.isReview
                        ? "bg-yellow-200/40 text-yellow-900 dark:bg-yellow-500/20 dark:text-yellow-300"
                        : "text-muted-foreground hover:bg-muted/40",
                    )}
                    onClick={() =>
                      setStages((rows) =>
                        rows.map((r, j) =>
                          j === i ? { ...r, isReview: !r.isReview } : r,
                        ),
                      )
                    }
                    disabled={busy}
                    title="Toggle REVIEW gate"
                  >
                    review
                  </button>
                  <Input
                    value={row.title}
                    onChange={(e) =>
                      setStages((rows) =>
                        rows.map((r, j) =>
                          j === i ? { ...r, title: e.target.value } : r,
                        ),
                      )
                    }
                    className="h-7 flex-1 text-xs"
                    placeholder="what this stage does"
                    disabled={busy}
                  />
                  <button
                    type="button"
                    className="text-muted-foreground hover:text-foreground h-7 w-7 text-xs"
                    onClick={() =>
                      setStages((rows) =>
                        i === 0
                          ? rows
                          : rows.map((r, j) =>
                              j === i - 1
                                ? rows[i]
                                : j === i
                                ? rows[i - 1]
                                : r,
                            ),
                      )
                    }
                    disabled={busy || i === 0}
                    aria-label="Move up"
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    className="text-muted-foreground hover:text-foreground h-7 w-7 text-xs"
                    onClick={() =>
                      setStages((rows) =>
                        i === rows.length - 1
                          ? rows
                          : rows.map((r, j) =>
                              j === i + 1
                                ? rows[i]
                                : j === i
                                ? rows[i + 1]
                                : r,
                            ),
                      )
                    }
                    disabled={busy || i === stages.length - 1}
                    aria-label="Move down"
                  >
                    ↓
                  </button>
                  <button
                    type="button"
                    className="text-muted-foreground hover:text-destructive h-7 w-7 text-xs"
                    onClick={() =>
                      setStages((rows) => rows.filter((_, j) => j !== i))
                    }
                    disabled={busy}
                    aria-label="Delete stage"
                  >
                    ×
                  </button>
                </li>
              ))}
              {stages.length === 0 && (
                <li className="text-muted-foreground border-border/40 rounded border border-dashed p-3 text-center text-xs italic">
                  No stages yet. Click <em>+ add stage</em>.
                </li>
              )}
            </ul>
          </div>
        </div>
      </ScrollArea>
    </div>
  );
}

interface ParsedStage {
  title: string;
  isReview: boolean;
}

interface ParsedSpec {
  name: string;
  goal: string;
  stages: ParsedStage[];
}

// Mirror of `codeless_runtime::template::JobTemplate::parse_yaml`. We
// stay regex-based for parity with the mock client and to avoid a
// YAML parser dep — the spec surface is one level deep and the
// invariant is enforced server-side on save.
function parseTemplate(yaml: string): ParsedSpec {
  const name = /^\s*name\s*:\s*(.+?)\s*$/m.exec(yaml)?.[1] ?? "";
  const goal = /^\s*goal\s*:\s*(.+?)\s*$/m.exec(yaml)?.[1] ?? "";
  const stages: ParsedStage[] = [];
  const stagesIdx = yaml.search(/^\s*stages\s*:\s*$/m);
  if (stagesIdx >= 0) {
    const after = yaml.slice(stagesIdx).split("\n").slice(1);
    for (const raw of after) {
      const m = raw.match(/^\s*-\s+(.*)$/);
      if (m) {
        const body = m[1].trim();
        if (body.startsWith("REVIEW ")) {
          stages.push({ title: body.slice("REVIEW ".length).trim(), isReview: true });
        } else if (body === "REVIEW") {
          stages.push({ title: "", isReview: true });
        } else {
          stages.push({ title: body, isReview: false });
        }
        continue;
      }
      if (raw.trim() === "") continue;
      if (/^\S/.test(raw)) break;
    }
  }
  return { name, goal, stages };
}

// Inverse of `parseTemplate`. Keeps the YAML stable across save
// round-trips so the dirty check (`current === templateYaml`) is
// meaningful — same field order, same quoting (none, lines are
// authored as raw scalars).
function serialise(name: string, goal: string, stages: StageRow[]): string {
  const lines = [`name: ${name}`, `goal: ${goal}`, "stages:"];
  for (const s of stages) {
    const t = s.title.trim();
    const prefix = s.isReview ? (t.length > 0 ? "REVIEW " : "REVIEW") : "";
    lines.push(`  - ${prefix}${t}`);
  }
  return lines.join("\n") + "\n";
}

interface NewFileDialogProps {
  hasScope: boolean;
  hasWorkflow: boolean;
  onClose: () => void;
  onCreate: (name: string, content: string) => void;
}

function NewFileDialog({
  hasScope,
  hasWorkflow,
  onClose,
  onCreate,
}: NewFileDialogProps) {
  const [name, setName] = useState("");
  const [content, setContent] = useState("");

  const presetScope = useCallback(() => {
    setName("SCOPE.md");
    setContent(SCOPE_PRESET);
  }, []);
  const presetWorkflow = useCallback(() => {
    setName("WORKFLOW.md");
    setContent(WORKFLOW_PRESET);
  }, []);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
    >
      <div
        className="bg-background border-border w-[36rem] max-w-[90vw] rounded-md border p-4 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="mb-3 text-sm font-medium">New file</h2>
        <div className="mb-2 flex gap-2">
          {!hasScope && (
            <Button size="sm" variant="outline" onClick={presetScope}>
              SCOPE.md preset
            </Button>
          )}
          {!hasWorkflow && (
            <Button size="sm" variant="outline" onClick={presetWorkflow}>
              WORKFLOW.md preset
            </Button>
          )}
        </div>
        <label className="text-muted-foreground mb-1 block text-[10px] uppercase">
          filename
        </label>
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="design.md"
          className="mb-3"
        />
        <label className="text-muted-foreground mb-1 block text-[10px] uppercase">
          content
        </label>
        <textarea
          className="border-border/60 mb-3 h-48 w-full resize-none rounded border bg-transparent p-2 font-mono text-xs leading-snug outline-none"
          value={content}
          onChange={(e) => setContent(e.target.value)}
          spellCheck={false}
        />
        <div className="flex justify-end gap-2">
          <Button size="sm" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            size="sm"
            onClick={() => onCreate(name, content)}
            disabled={!name.trim()}
          >
            Create
          </Button>
        </div>
      </div>
    </div>
  );
}
