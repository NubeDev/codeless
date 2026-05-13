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
  // Prompt-only jobs surface `InvalidArgument: ... file surface is
  // template-only` from `list_job_files`. That's an expected shape —
  // not an error worth alarming the user about. Render the same
  // friendly empty state the stages editor uses, so the Spec tab on
  // a mock/prompt-only job is informative rather than red.
  if (error && !listing) {
    const looksTemplateOnly =
      error.includes("template-only") || error.includes("invalid_argument");
    if (looksTemplateOnly) {
      return (
        <div className="text-muted-foreground p-4 text-sm italic">
          This job is a single-prompt run, not a multi-stage spec. The Spec
          pane is template-only — submit a job with a <code>template.yaml</code>{" "}
          to use it.
        </div>
      );
    }
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
            availableDocs={listing.entries
              .filter((e) => !e.is_template)
              .map((e) => e.name)}
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
  // Filenames in the job directory (excluding template.yaml) that
  // the user can add to the ordered `docs:` list. The picker shows
  // these in a dropdown filtered by what's not already attached.
  availableDocs: string[];
  onSaved: () => void;
}

interface StageRow {
  // Stable client-side id so React keys survive reorders.
  uid: number;
  title: string;
  isReview: boolean;
  // Per-stage docs (null when the stage hasn't opted into the
  // structured form yet; we keep null vs [] distinct so toggling
  // "+ add doc" doesn't silently rewrite a bare stage as structured
  // until the user actually attaches something).
  docs: string[] | null;
}

let stageUid = 0;
const mkRow = (
  title: string,
  isReview: boolean,
  docs: string[] | null = null,
): StageRow => ({
  uid: ++stageUid,
  title,
  isReview,
  docs,
});

// Structured spec editor. `name`, `goal`, and an ordered list of
// stages with an optional REVIEW prefix are the entire authoring
// surface (matches `JobTemplate` server-side). Save serialises back
// to YAML and round-trips through `update_job_template`, which is
// the only path that mutates `template.yaml` — `write_job_file`
// rejects it as reserved.
function StagesEditor({
  jobId,
  templateYaml,
  availableDocs,
  onSaved,
}: StagesEditorProps) {
  const rpc = useRpc();
  const [seedKey, setSeedKey] = useState<string>("");
  const [name, setName] = useState("");
  const [goal, setGoal] = useState("");
  // `null` ⇒ no docs: block in YAML (legacy auto-discover, all *.md
  // in alpha order). `[]` ⇒ explicit empty (no docs at all). The UI
  // toggles between the two via a "control docs order" affordance so
  // both states are reachable from the editor.
  const [docs, setDocs] = useState<string[] | null>(null);
  const [stages, setStages] = useState<StageRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (templateYaml === null) {
      setName("");
      setGoal("");
      setDocs(null);
      setStages([]);
      setSeedKey("");
      return;
    }
    if (templateYaml === seedKey) return;
    const parsed = parseTemplate(templateYaml);
    setName(parsed.name);
    setGoal(parsed.goal);
    setDocs(parsed.docs);
    setStages(parsed.stages.map((s) => mkRow(s.title, s.isReview, s.docs)));
    setSeedKey(templateYaml);
  }, [templateYaml, seedKey]);

  const dirty = useMemo(() => {
    if (templateYaml === null) return false;
    return serialise(name, goal, docs, stages) !== templateYaml;
  }, [name, goal, docs, stages, templateYaml]);

  const onSave = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const yaml = serialise(name, goal, docs, stages);
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
  }, [rpc, jobId, name, goal, docs, stages, onSaved]);

  const onDiscard = useCallback(() => {
    if (templateYaml === null) return;
    const parsed = parseTemplate(templateYaml);
    setName(parsed.name);
    setGoal(parsed.goal);
    setDocs(parsed.docs);
    setStages(parsed.stages.map((s) => mkRow(s.title, s.isReview, s.docs)));
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
          <DocsSection
            docs={docs}
            setDocs={setDocs}
            availableDocs={availableDocs}
            busy={busy}
          />
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
                  className="border-border/40 flex flex-col gap-1 rounded border px-2 py-1"
                >
                  <div className="flex items-center gap-1">
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
                  </div>
                  <StageDocsRow
                    row={row}
                    availableDocs={availableDocs}
                    busy={busy}
                    onChange={(next) =>
                      setStages((rows) =>
                        rows.map((r, j) => (j === i ? { ...r, docs: next } : r)),
                      )
                    }
                  />
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

interface StageDocsRowProps {
  row: StageRow;
  availableDocs: string[];
  busy: boolean;
  onChange: (next: string[] | null) => void;
}

// Per-stage docs row sits underneath each stage's title. Three
// states:
//   - `row.docs === null`: stage hasn't opted in. We show a tiny
//     muted "+ docs for this stage" button. Clicking starts the
//     structured form with an empty list.
//   - `row.docs === []`: opted in, no docs yet. Show the picker
//     dropdown so the user can pick the first one.
//   - `row.docs === [...]`: render attached doc chips + the picker.
function StageDocsRow({ row, availableDocs, busy, onChange }: StageDocsRowProps) {
  if (row.docs === null) {
    return (
      <div className="text-muted-foreground ml-7 flex items-center gap-2 text-[10px]">
        <button
          type="button"
          className="hover:text-foreground underline-offset-2 hover:underline"
          onClick={() => onChange([])}
          disabled={busy}
        >
          + docs for this stage
        </button>
      </div>
    );
  }

  const attached = new Set(row.docs);
  const addable = availableDocs.filter((n) => !attached.has(n));

  return (
    <div className="ml-7 flex flex-col gap-1">
      <div className="text-muted-foreground flex items-center gap-2 text-[10px] uppercase">
        <span>stage docs</span>
        <button
          type="button"
          className="hover:text-foreground underline-offset-2 hover:underline"
          onClick={() => onChange(null)}
          disabled={busy}
        >
          (remove)
        </button>
      </div>
      {row.docs.length > 0 && (
        <ul className="flex flex-wrap gap-1">
          {row.docs.map((d, di) => (
            <li
              key={`${d}-${di}`}
              className="border-border/40 bg-muted/30 flex items-center gap-1 rounded border px-1.5 py-0.5 font-mono text-[10px]"
            >
              <span>{d}</span>
              <button
                type="button"
                className="text-muted-foreground hover:text-foreground"
                onClick={() =>
                  onChange(
                    di === 0 ? row.docs! : row.docs!.map((x, j) =>
                      j === di - 1 ? row.docs![di] : j === di ? row.docs![di - 1] : x,
                    ),
                  )
                }
                disabled={busy || di === 0}
                aria-label="Move up"
              >
                ↑
              </button>
              <button
                type="button"
                className="text-muted-foreground hover:text-foreground"
                onClick={() =>
                  onChange(
                    di === row.docs!.length - 1
                      ? row.docs!
                      : row.docs!.map((x, j) =>
                          j === di + 1 ? row.docs![di] : j === di ? row.docs![di + 1] : x,
                        ),
                  )
                }
                disabled={busy || di === row.docs!.length - 1}
                aria-label="Move down"
              >
                ↓
              </button>
              <button
                type="button"
                className="text-muted-foreground hover:text-destructive"
                onClick={() => onChange(row.docs!.filter((_, j) => j !== di))}
                disabled={busy}
                aria-label="Remove"
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
      {addable.length > 0 ? (
        <select
          className="border-border/60 h-6 w-fit rounded border bg-transparent px-1 text-[10px]"
          value=""
          onChange={(e) => {
            if (e.target.value) onChange([...row.docs!, e.target.value]);
          }}
          disabled={busy}
        >
          <option value="">+ add doc</option>
          {addable.map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      ) : (
        <span className="text-muted-foreground text-[10px] italic">
          no more files to attach
        </span>
      )}
    </div>
  );
}

interface DocsSectionProps {
  docs: string[] | null;
  setDocs: (next: string[] | null) => void;
  availableDocs: string[];
  busy: boolean;
}

// Docs section of the spec editor. Two distinct states:
//   - `docs === null`: no `docs:` block in YAML. The runtime falls
//     back to auto-discover (every .md, SCOPE/WORKFLOW first, rest
//     alpha). UI shows a hint + a "Control docs order" button that
//     seeds the list from `availableDocs` so the user can curate.
//   - `docs === []` or non-empty: explicit ordered list. UI shows
//     rows with ↑/↓ reorder, × remove, plus a "+ add doc" dropdown
//     filtered to docs that aren't already attached. A "Reset to
//     auto" button removes the block entirely (back to `null`).
function DocsSection({ docs, setDocs, availableDocs, busy }: DocsSectionProps) {
  if (docs === null) {
    return (
      <div className="border-border/40 rounded border border-dashed p-3">
        <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
          docs
        </div>
        <p className="text-muted-foreground mt-1 text-xs">
          Auto-discover: every <code>*.md</code> in the job dir is read,
          SCOPE.md and WORKFLOW.md first, then the rest alphabetically.
        </p>
        <Button
          size="sm"
          variant="outline"
          className="mt-2 h-6 px-2 text-[10px]"
          onClick={() => setDocs(autoDiscoverDefault(availableDocs))}
          disabled={busy}
        >
          Control docs order
        </Button>
      </div>
    );
  }

  const attached = new Set(docs);
  const addable = availableDocs.filter((n) => !attached.has(n));

  return (
    <div>
      <div className="text-muted-foreground mb-1 flex items-center justify-between text-[10px] uppercase">
        <span>docs (in order)</span>
        <button
          type="button"
          className="text-muted-foreground hover:text-foreground text-[10px] underline-offset-2 hover:underline"
          onClick={() => setDocs(null)}
          disabled={busy}
        >
          Reset to auto
        </button>
      </div>
      <ul className="space-y-1">
        {docs.map((d, i) => (
          <li
            key={`${d}-${i}`}
            className="border-border/40 flex items-center gap-1 rounded border px-2 py-1"
          >
            <span className="text-muted-foreground w-5 text-right font-mono text-[10px]">
              {i + 1}.
            </span>
            <span className="flex-1 truncate font-mono text-xs">{d}</span>
            <button
              type="button"
              className="text-muted-foreground hover:text-foreground h-7 w-7 text-xs"
              onClick={() =>
                setDocs(
                  i === 0
                    ? docs
                    : docs.map((x, j) =>
                        j === i - 1 ? docs[i] : j === i ? docs[i - 1] : x,
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
                setDocs(
                  i === docs.length - 1
                    ? docs
                    : docs.map((x, j) =>
                        j === i + 1 ? docs[i] : j === i ? docs[i + 1] : x,
                      ),
                )
              }
              disabled={busy || i === docs.length - 1}
              aria-label="Move down"
            >
              ↓
            </button>
            <button
              type="button"
              className="text-muted-foreground hover:text-destructive h-7 w-7 text-xs"
              onClick={() => setDocs(docs.filter((_, j) => j !== i))}
              disabled={busy}
              aria-label="Remove"
            >
              ×
            </button>
          </li>
        ))}
        {docs.length === 0 && (
          <li className="text-muted-foreground border-border/40 rounded border border-dashed p-3 text-center text-xs italic">
            No docs attached. The agent will see only the goal + stages.
          </li>
        )}
      </ul>
      {addable.length > 0 && (
        <div className="mt-2 flex items-center gap-2">
          <select
            className="border-border/60 h-7 rounded border bg-transparent px-2 text-xs"
            value=""
            onChange={(e) => {
              if (e.target.value) setDocs([...docs, e.target.value]);
            }}
            disabled={busy}
          >
            <option value="">+ add doc</option>
            {addable.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
          <span className="text-muted-foreground text-[10px]">
            choose from files in the Spec pane sidebar
          </span>
        </div>
      )}
      {availableDocs.length === 0 && (
        <p className="text-muted-foreground mt-2 text-[10px]">
          No markdown files in the job dir yet. Use{" "}
          <em>+ file</em> in the sidebar to add SCOPE.md, WORKFLOW.md, or any
          design notes.
        </p>
      )}
    </div>
  );
}

// Default seed for the docs list when the user first clicks "Control
// docs order". Mirrors the auto-discover order so toggling control on
// then saving with no further edits is a no-op for the agent.
function autoDiscoverDefault(available: string[]): string[] {
  const md = available.filter((n) => /\.md$/i.test(n));
  const scope = md.find((n) => n.toLowerCase() === "scope.md");
  const workflow = md.find((n) => n.toLowerCase() === "workflow.md");
  const rest = md.filter(
    (n) => n.toLowerCase() !== "scope.md" && n.toLowerCase() !== "workflow.md",
  );
  rest.sort();
  const out: string[] = [];
  if (scope) out.push(scope);
  if (workflow) out.push(workflow);
  out.push(...rest);
  return out;
}

interface ParsedStage {
  title: string;
  isReview: boolean;
  // Per-stage docs (basenames inside the job dir). `null` ⇒ the
  // stage opted out of the structured form (it was authored as a
  // bare title string). `[]` ⇒ explicit empty stage-docs list. The
  // editor mirrors the Rust `StageSpec.docs: Option<Vec<String>>`.
  docs: string[] | null;
}

interface ParsedSpec {
  name: string;
  goal: string;
  // `null` ⇒ no `docs:` block in the YAML at all (legacy auto-discover
  // behaviour). `[]` ⇒ explicit empty list (no docs flow to the agent).
  // Both meanings are user-visible so we don't collapse them on parse.
  docs: string[] | null;
  stages: ParsedStage[];
}

// Mirror of `codeless_runtime::template::JobTemplate::parse_yaml`. We
// stay regex-based for parity with the mock client and to avoid a
// YAML parser dep — the spec surface is one level deep and the
// invariant is enforced server-side on save. Two stage shapes are
// recognised: bare title strings (`- "do thing"`) and structured
// maps (`- title: "do thing"` with optional `review` / `docs`).
function parseTemplate(yaml: string): ParsedSpec {
  const name = /^\s*name\s*:\s*(.+?)\s*$/m.exec(yaml)?.[1] ?? "";
  const goal = /^\s*goal\s*:\s*(.+?)\s*$/m.exec(yaml)?.[1] ?? "";
  const docs = parseListBlock(yaml, "docs");
  const stages: ParsedStage[] = [];
  const stagesIdx = yaml.search(/^\s*stages\s*:\s*$/m);
  if (stagesIdx >= 0) {
    const lines = yaml.slice(stagesIdx).split("\n").slice(1);
    let i = 0;
    while (i < lines.length) {
      const raw = lines[i];
      const bullet = raw.match(/^(\s*)-\s+(.*)$/);
      if (!bullet) {
        // Blank lines inside the block are tolerated; a non-bullet
        // non-blank line at any indent means the stages block ended.
        if (raw.trim() === "") {
          i++;
          continue;
        }
        if (/^\S/.test(raw)) break;
        i++;
        continue;
      }
      const indent = bullet[1].length;
      const body = bullet[2];
      const titleInline = /^title\s*:\s*(.+)$/.exec(body);
      if (titleInline) {
        // Structured stage: read indented child keys until the next
        // bullet at the same indent or the next top-level key.
        let stage: ParsedStage = {
          title: titleInline[1].trim(),
          isReview: false,
          docs: null,
        };
        i++;
        while (i < lines.length) {
          const r = lines[i];
          if (r.trim() === "") {
            i++;
            continue;
          }
          // End of structured block: another bullet, OR a non-indented line.
          const nextBullet = r.match(/^(\s*)-\s+/);
          if (nextBullet && nextBullet[1].length <= indent) break;
          if (/^\S/.test(r)) break;
          const childIndent = (r.match(/^(\s*)/)?.[1].length ?? 0);
          if (childIndent <= indent) break;

          const reviewMatch = /^\s*review\s*:\s*(true|false)\s*$/.exec(r);
          if (reviewMatch) {
            stage.isReview = reviewMatch[1] === "true";
            i++;
            continue;
          }
          const docsHead = /^\s*docs\s*:\s*(.*)$/.exec(r);
          if (docsHead) {
            const inline = docsHead[1].trim();
            if (inline.startsWith("[") && inline.endsWith("]")) {
              const inner = inline.slice(1, -1).trim();
              stage.docs =
                inner === ""
                  ? []
                  : inner
                      .split(",")
                      .map((s) => s.trim())
                      .filter((s) => s.length > 0);
              i++;
              continue;
            }
            // Block form for stage docs.
            stage.docs = [];
            i++;
            while (i < lines.length) {
              const dr = lines[i];
              if (dr.trim() === "") {
                i++;
                continue;
              }
              const docB = dr.match(/^(\s*)-\s+(.+?)\s*$/);
              if (!docB || docB[1].length <= indent + 2) {
                if (docB && docB[1].length <= indent) break;
                if (!docB) break;
                break;
              }
              stage.docs.push(docB[2]);
              i++;
            }
            continue;
          }
          // Unknown child key: skip.
          i++;
        }
        stages.push(stage);
        continue;
      }
      // Bare-title bullet (with optional REVIEW prefix).
      const t = body.trim();
      if (t.startsWith("REVIEW ")) {
        stages.push({ title: t.slice("REVIEW ".length).trim(), isReview: true, docs: null });
      } else if (t === "REVIEW") {
        stages.push({ title: "", isReview: true, docs: null });
      } else {
        stages.push({ title: t, isReview: false, docs: null });
      }
      i++;
    }
  }
  return { name, goal, docs, stages };
}

// Pull a single-level YAML list out of the template by key. Returns
// `null` when the key is absent (so the caller can distinguish "no
// docs block" from "empty docs block"). Stops at the first top-level
// key after the block, identical to the stages parser.
function parseListBlock(yaml: string, key: string): string[] | null {
  const headRe = new RegExp(`^\\s*${key}\\s*:\\s*(.*)$`, "m");
  const m = headRe.exec(yaml);
  if (!m) return null;
  // Inline form: `docs: []` or `docs: [a, b]`
  const inline = m[1].trim();
  if (inline.startsWith("[") && inline.endsWith("]")) {
    const body = inline.slice(1, -1).trim();
    if (body === "") return [];
    return body
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
  }
  if (inline !== "") return null;
  // Block form: lines that follow until the next top-level key.
  const after = yaml.slice(yaml.indexOf(m[0]) + m[0].length).split("\n").slice(1);
  const out: string[] = [];
  for (const raw of after) {
    const item = raw.match(/^\s*-\s+(.*)$/);
    if (item) {
      out.push(item[1].trim());
      continue;
    }
    if (raw.trim() === "") continue;
    if (/^\S/.test(raw)) break;
  }
  return out;
}

// Inverse of `parseTemplate`. Keeps the YAML stable across save
// round-trips so the dirty check (`current === templateYaml`) is
// meaningful — same field order, same quoting (none, lines are
// authored as raw scalars). Stages emit either a bare title bullet
// or the structured mapping form depending on whether the stage has
// opted into per-stage docs (or carries an explicit `review` flag in
// the structured shape).
function serialise(
  name: string,
  goal: string,
  docs: string[] | null,
  stages: StageRow[],
): string {
  const lines = [`name: ${name}`, `goal: ${goal}`];
  if (docs !== null) {
    if (docs.length === 0) {
      lines.push("docs: []");
    } else {
      lines.push("docs:");
      for (const d of docs) lines.push(`  - ${d.trim()}`);
    }
  }
  lines.push("stages:");
  for (const s of stages) {
    const t = s.title.trim();
    // A stage emits the structured form only when it actually has
    // per-stage docs attached. REVIEW stays as the bare-string `REVIEW `
    // prefix to keep flat-template ergonomics — switching review on
    // shouldn't force the YAML to bloat.
    if (s.docs !== null) {
      lines.push(`  - title: ${t}`);
      if (s.isReview) lines.push(`    review: true`);
      if (s.docs.length === 0) {
        lines.push(`    docs: []`);
      } else {
        lines.push(`    docs:`);
        for (const d of s.docs) lines.push(`      - ${d.trim()}`);
      }
    } else {
      const prefix = s.isReview ? (t.length > 0 ? "REVIEW " : "REVIEW") : "";
      lines.push(`  - ${prefix}${t}`);
    }
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
