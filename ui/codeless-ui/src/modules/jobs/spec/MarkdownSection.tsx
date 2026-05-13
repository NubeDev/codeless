import { useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { useRpc, type JobId } from "@/lib/rpc";

import { InlineEditor } from "./InlineEditor";

interface Props {
  jobId: JobId;
  filename: string;
  // Optional canned body used when the file does not exist on disk
  // and the user clicks `+ add`. Lets the parent customise the seed
  // for SCOPE.md vs WORKFLOW.md vs a generic doc.
  presetBody?: string;
  // Caller-supplied human label and one-line hint that describe what
  // this file is for. Shows up in the section header.
  title?: string;
  hint?: string;
  // Whether this file currently exists in the job directory. Drives
  // the "add" vs "edit" branch.
  exists: boolean;
  // Whether the user can delete this file. SCOPE/WORKFLOW are
  // technically deletable too but typically the user wants to wipe
  // them rather than delete; arbitrary docs the user added are full
  // delete candidates.
  deletable?: boolean;
  // Refresh hook so the parent can re-fetch list_job_files after a
  // create / delete that changes the entry list.
  onChanged: () => void;
  // Open the file in the editor tab host (for serious edits). Only
  // shown when the parent provides this; the dashboard-singleton
  // mount path doesn't use it.
  onOpenInTab?: (filename: string) => void;
}

export function MarkdownSection({
  jobId,
  filename,
  presetBody = "",
  title,
  hint,
  exists,
  deletable,
  onChanged,
  onOpenInTab,
}: Props) {
  const rpc = useRpc();
  const [diskContent, setDiskContent] = useState<string | null>(null);
  const [buffer, setBuffer] = useState<string>("");
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Read the file when it exists. The read happens once per
  // (jobId, filename, exists) tuple — we rely on the parent calling
  // onChanged() to flip exists / re-mount us when an out-of-band
  // edit happens.
  useEffect(() => {
    if (!exists) {
      setDiskContent(null);
      setBuffer("");
      setEditing(false);
      return;
    }
    let cancelled = false;
    rpc
      .call("read_job_file", { job_id: jobId, filename })
      .then((res) => {
        if (cancelled) return;
        setDiskContent(res.content);
        setBuffer(res.content);
        setError(null);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, jobId, filename, exists]);

  const dirty = useMemo(
    () => editing && diskContent !== null && buffer !== diskContent,
    [editing, diskContent, buffer],
  );

  const onCreate = async () => {
    setBusy(true);
    setError(null);
    try {
      await rpc.call("write_job_file", {
        job_id: jobId,
        filename,
        content: presetBody,
      });
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const onSave = async () => {
    setBusy(true);
    setError(null);
    try {
      await rpc.call("write_job_file", {
        job_id: jobId,
        filename,
        content: buffer,
      });
      setDiskContent(buffer);
      setEditing(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const onDiscard = () => {
    setBuffer(diskContent ?? "");
    setEditing(false);
    setError(null);
  };

  const onDelete = async () => {
    if (!confirm(`Delete ${filename}? This commits the removal.`)) return;
    setBusy(true);
    setError(null);
    try {
      await rpc.call("delete_job_file", { job_id: jobId, filename });
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="border-border/50 rounded border">
      <SectionHeader
        title={title ?? filename}
        subtitle={hint}
        filename={title ? filename : undefined}
        dirty={dirty}
        actions={
          !exists ? (
            <Button
              size="sm"
              onClick={() => void onCreate()}
              disabled={busy}
              className="h-7 px-2 text-xs"
            >
              {busy ? "creating…" : `+ add ${filename}`}
            </Button>
          ) : editing ? (
            <>
              {onOpenInTab && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => onOpenInTab(filename)}
                  className="h-7 px-2 text-xs"
                  title="Open in a full editor tab"
                >
                  open in tab
                </Button>
              )}
              <Button
                size="sm"
                variant="ghost"
                onClick={onDiscard}
                disabled={busy}
                className="h-7 px-2 text-xs"
              >
                discard
              </Button>
              <Button
                size="sm"
                onClick={() => void onSave()}
                disabled={busy || !dirty}
                className="h-7 px-2 text-xs"
              >
                {busy ? "saving…" : "save"}
              </Button>
            </>
          ) : (
            <>
              {onOpenInTab && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => onOpenInTab(filename)}
                  className="h-7 px-2 text-xs"
                  title="Open in a full editor tab"
                >
                  open in tab
                </Button>
              )}
              {deletable && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => void onDelete()}
                  disabled={busy}
                  className="text-destructive h-7 px-2 text-xs"
                >
                  delete
                </Button>
              )}
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setEditing(true)}
                className="h-7 px-2 text-xs"
              >
                edit
              </Button>
            </>
          )
        }
      />
      {error && (
        <div className="text-destructive border-destructive/40 bg-destructive/5 border-t px-3 py-2 text-xs">
          {error}
        </div>
      )}
      {!exists ? (
        <div className="text-muted-foreground p-4 text-sm">
          Not on disk yet. <code>+ add {filename}</code> seeds it with a
          one-screen template and commits it in the source repo.
        </div>
      ) : diskContent === null ? (
        <div className="text-muted-foreground p-4 text-sm italic">loading…</div>
      ) : (
        <div className="p-3">
          <InlineEditor
            value={buffer}
            onChange={setBuffer}
            language="markdown"
            readOnly={!editing}
          />
        </div>
      )}
    </section>
  );
}

function SectionHeader({
  title,
  subtitle,
  filename,
  dirty,
  actions,
}: {
  title: string;
  subtitle?: string;
  filename?: string;
  dirty?: boolean;
  actions?: React.ReactNode;
}) {
  return (
    <header className="border-border/40 flex items-center gap-2 border-b px-3 py-2">
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="font-mono text-sm">{title}</span>
          {filename && (
            <span className="text-muted-foreground font-mono text-[10px]">
              {filename}
            </span>
          )}
          {dirty && (
            <span
              className="text-[10px] text-amber-600 dark:text-amber-400"
              title="unsaved changes"
            >
              · edited
            </span>
          )}
        </div>
        {subtitle && (
          <div className="text-muted-foreground text-[10px]">{subtitle}</div>
        )}
      </div>
      <div className="flex items-center gap-1">{actions}</div>
    </header>
  );
}
