import { useCallback, useEffect, useMemo, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import {
  useRpc,
  type JobFileEntry,
  type JobId,
  type ListJobFilesResult,
} from "@/lib/rpc";

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
  const [listing, setListing] = useState<ListJobFilesResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<string | null>(null);
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

  // After a refresh, keep the previously-selected file selected if
  // it still exists; otherwise prefer template.yaml, falling back to
  // the first entry. Selecting nothing for an empty directory is
  // intentional — the user has to add a file first.
  useEffect(() => {
    if (!listing) return;
    if (listing.entries.length === 0) {
      setSelected(null);
      setBuffer("");
      setSavedContent("");
      return;
    }
    const stillExists =
      selected && listing.entries.some((e) => e.name === selected);
    if (stillExists) return;
    const tpl = listing.entries.find((e) => e.is_template);
    setSelected((tpl ?? listing.entries[0]).name);
  }, [listing, selected]);

  // Load the selected file's content on selection change. template.yaml
  // is rendered through `read_job_file` like any other file so the
  // UI can show the spec verbatim without rebuilding it from `Job`'s
  // `template_yaml` column (which is the *seed*, not the on-disk
  // source after the first migration).
  useEffect(() => {
    if (!selected) {
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
            {listing.entries.map((entry) => (
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
                  {entry.is_template && (
                    <Badge variant="outline" className="ml-2 text-[9px]">
                      spec
                    </Badge>
                  )}
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
                {!entry.is_template && (
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
                )}
              </li>
            ))}
            {listing.entries.length === 0 && (
              <li className="text-muted-foreground px-3 py-2 text-xs italic">
                (empty)
              </li>
            )}
          </ul>
        </ScrollArea>
      </aside>

      <section className="flex min-w-0 flex-1 flex-col">
        {selected && selectedEntry ? (
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
