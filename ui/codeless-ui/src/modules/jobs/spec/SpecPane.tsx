import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  useEventStream,
  useJob,
  useRpc,
  type JobId,
  type ListJobFilesResult,
} from "@/lib/rpc";

import { MarkdownSection } from "./MarkdownSection";
import { TemplateSection } from "./TemplateSection";

interface Props {
  jobId: JobId;
  // Optional: open the file's absolute path in the host's editor tab
  // for serious editing. Threaded through to MarkdownSection's [open
  // in tab] button. The dashboard-singleton mount path doesn't pass
  // this; the tabbed-workspace mount does.
  onOpenFile?: (absPath: string) => void;
}

const SCOPE_PRESET = `# Scope

What this job is for. Replace this with what success looks like, what
is out of scope, the constraints, and the deliverables.
`;

const WORKFLOW_PRESET = `# Workflow

How the agent should drive the work. Replace this with how to sequence
the stages, what to verify between them, and what counts as done.
`;

// Spec pane: one vertical scroll, sections in the same order the
// runtime folds them into the prompt:
//
//   template.yaml  → name / goal / stages / docs order
//   SCOPE.md       → what the job is for
//   WORKFLOW.md    → how the agent should drive the work
//   other docs     → user-added supporting markdown
//
// Each section is independent: its own save / discard, its own
// commit, its own dirty indicator. Per-section save = small commits =
// a clean `git log` of "what evolved when". The user never has to
// reason about a 'global' save covering edits across files.
export function SpecPane({ jobId, onOpenFile }: Props) {
  const rpc = useRpc();
  const { data: job, refetch: refetchJob } = useJob(jobId);
  const [listing, setListing] = useState<ListJobFilesResult | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    try {
      const res = await rpc.call("list_job_files", { job_id: jobId });
      setListing(res);
      setListError(null);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setListError(msg);
    } finally {
      setLoading(false);
    }
  }, [rpc, jobId]);

  useEffect(() => {
    setLoading(true);
    void refresh();
  }, [refresh]);

  // Refetch the file list and job row whenever the spec changes
  // out-of-band — `update_job_template` from this pane already
  // updates via `afterSave`, but the same events fire when the chat
  // agent edits `.codeless/jobs/<name>/template.yaml` from a chat
  // turn (caught by `resync_template_from_disk` at the next
  // `start_job` / `resume_job`) or when another tab calls
  // `write_job_file` / `delete_job_file`. Without this hook the user
  // would see the agent's edits only after switching panes.
  useEventStream(
    { scope: "job", job_id: jobId },
    (env) => {
      if (
        env.event.type === "job-template-updated" ||
        env.event.type === "job-file-updated"
      ) {
        setLoading(true);
        void refresh();
        refetchJob();
      }
    },
  );

  // Common post-save callback for every editable surface in the
  // pane. Re-fetches the file list (so add/remove shows up) AND the
  // job row (so `template_yaml` is fresh — without this, a save in
  // TemplateSection would leave the parent's `job.template_yaml`
  // stale, the SPEC summary would re-render the pre-edit YAML, and
  // the user would think their save vanished).
  const afterSave = useCallback(() => {
    setLoading(true);
    void refresh();
    refetchJob();
  }, [refresh, refetchJob]);

  const onOpenInTab = useCallback(
    (filename: string) => {
      if (!onOpenFile || !listing?.directory_path) return;
      const sep = listing.directory_path.endsWith("/") ? "" : "/";
      onOpenFile(`${listing.directory_path}${sep}${filename}`);
    },
    [onOpenFile, listing?.directory_path],
  );

  // Derive promptOnly from the job data — no template_yaml means
  // the job was submitted with a free-form prompt only.
  const isPromptOnly = !!job && !job.template_yaml;

  if (loading && !listing && !isPromptOnly) {
    return (
      <div className="text-muted-foreground p-4 text-sm italic">loading…</div>
    );
  }

  // Legacy prompt-only jobs (submitted via `codeless run` or via the
  // CLI before the UI started always seeding a template). The Spec
  // pane has no files to show; surface the prompt itself for context
  // and tell the user how to iterate.
  if (isPromptOnly) {
    return (
      <div className="text-muted-foreground mx-auto max-w-2xl space-y-3 p-6 text-sm">
        <h2 className="text-foreground text-base font-medium">
          Prompt-only job
        </h2>
        <p>
          This job was submitted with a free-form prompt and has no
          editable spec on disk. The new submit flow always seeds{" "}
          <code>template.yaml</code> + <code>SCOPE.md</code> +{" "}
          <code>WORKFLOW.md</code> so the Spec pane has something to show
          — submit a fresh job (with a name) to use the iterate loop.
        </p>
        {job?.prompt && (
          <div className="space-y-1">
            <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
              prompt
            </div>
            <pre className="bg-muted/40 max-h-64 overflow-auto rounded p-2 text-xs whitespace-pre-wrap">
              {job.prompt}
            </pre>
          </div>
        )}
      </div>
    );
  }

  if (listError) {
    return <div className="text-destructive p-4 text-sm">{listError}</div>;
  }
  if (!listing) return null;

  const entries = listing.entries;
  const hasScope = entries.some((e) => e.is_scope);
  const hasWorkflow = entries.some((e) => e.is_workflow);
  const others = entries.filter(
    (e) => !e.is_template && !e.is_scope && !e.is_workflow,
  );
  // Available markdown docs the per-stage and global pickers can
  // attach. Excludes template.yaml — that's the spec, not a doc.
  // Order doesn't matter for the picker (the user-facing order is
  // the picker's own); the runtime is what reads the template back.
  const availableDocs = entries
    .filter((e) => !e.is_template && /\.md$/i.test(e.name))
    .map((e) => e.name);

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto flex max-w-3xl flex-col gap-4 p-4">
        <Header
          dirPath={listing.directory_path}
          jobId={jobId}
          onAdded={afterSave}
        />

        <TemplateSection
          jobId={jobId}
          templateYaml={job?.template_yaml ?? null}
          availableDocs={availableDocs}
          onSaved={afterSave}
        />

        <MarkdownSection
          jobId={jobId}
          filename="SCOPE.md"
          title="SCOPE.md"
          hint="the brief — what the job is for"
          presetBody={SCOPE_PRESET}
          exists={hasScope}
          onChanged={afterSave}
          onOpenInTab={onOpenFile ? onOpenInTab : undefined}
        />

        <MarkdownSection
          jobId={jobId}
          filename="WORKFLOW.md"
          title="WORKFLOW.md"
          hint="how the agent should drive the work"
          presetBody={WORKFLOW_PRESET}
          exists={hasWorkflow}
          onChanged={afterSave}
          onOpenInTab={onOpenFile ? onOpenInTab : undefined}
        />

        <OtherDocs
          jobId={jobId}
          others={others}
          onChanged={afterSave}
          onOpenInTab={onOpenFile ? onOpenInTab : undefined}
        />
      </div>
    </ScrollArea>
  );
}

function Header({
  dirPath,
  jobId,
  onAdded,
}: {
  dirPath: string | null;
  jobId: JobId;
  onAdded: () => void;
}) {
  const [adding, setAdding] = useState(false);
  return (
    <header className="space-y-1">
      <h2 className="text-sm font-semibold">Spec</h2>
      <p className="text-muted-foreground text-xs leading-snug">
        Files under{" "}
        <code className="text-[11px]">
          {dirPath ?? ".codeless/jobs/<name>/"}
        </code>
        . Edits here commit in the source repo and apply to the next run.
      </p>
      {adding ? (
        <NewDocInline
          jobId={jobId}
          onCreated={() => {
            setAdding(false);
            onAdded();
          }}
          onCancel={() => setAdding(false)}
        />
      ) : (
        <div className="pt-1">
          <Button
            size="sm"
            variant="ghost"
            className="h-7 px-2 text-xs"
            onClick={() => setAdding(true)}
          >
            + add markdown doc
          </Button>
        </div>
      )}
    </header>
  );
}

function NewDocInline({
  jobId,
  onCreated,
  onCancel,
}: {
  jobId: JobId;
  onCreated: () => void;
  onCancel: () => void;
}) {
  const rpc = useRpc();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const trimmed = name.trim();
      const filename = /\.(md|yaml|yml)$/.test(trimmed)
        ? trimmed
        : `${trimmed}.md`;
      await rpc.call("write_job_file", {
        job_id: jobId,
        filename,
        content: `# ${filename}\n\n`,
      });
      setName("");
      onCreated();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex items-center gap-2 pt-1">
      <Input
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="design.md"
        className="h-7 max-w-xs text-xs"
        spellCheck={false}
      />
      <Button
        size="sm"
        onClick={() => void submit()}
        disabled={!name.trim() || busy}
        className="h-7 px-2 text-xs"
      >
        {busy ? "creating…" : "create"}
      </Button>
      <Button
        size="sm"
        variant="ghost"
        onClick={onCancel}
        className="h-7 px-2 text-xs"
      >
        cancel
      </Button>
      {error && <span className="text-destructive text-xs">{error}</span>}
    </div>
  );
}

function OtherDocs({
  jobId,
  others,
  onChanged,
  onOpenInTab,
}: {
  jobId: JobId;
  others: { name: string }[];
  onChanged: () => void;
  onOpenInTab?: (filename: string) => void;
}) {
  if (others.length === 0) return null;
  return (
    <div className="flex flex-col gap-3">
      <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
        other docs
      </div>
      {others.map((d) => (
        <MarkdownSection
          key={d.name}
          jobId={jobId}
          filename={d.name}
          exists
          deletable
          onChanged={onChanged}
          onOpenInTab={onOpenInTab}
        />
      ))}
    </div>
  );
}
