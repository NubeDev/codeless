// Import-job modal. Drives the bundle-path -> inspect -> import
// round-trip specified in §"UI / Workspaces sidebar" of
// DOCS/SCOPE-JOB-EXPORT.md. The user picks (or types) a path to a
// `.codeless-job` bundle on the server, the dialog calls
// `inspect_job_bundle` to fetch the manifest without touching
// SQLite, surfaces a preview + any warnings, lets the user choose a
// conflict policy and optional rename, and on confirm calls
// `import_job`. Post-import warnings are surfaced to the caller so
// the JobPage can show them as a dismissible banner.

import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useRpc } from "@/lib/rpc";
import type {
  ImportConflictPolicy,
  ImportJobResult,
  InspectJobBundleResult,
} from "@/lib/rpc/methods";
import type { RepoId } from "@/lib/rpc/wire";

interface ImportJobDialogProps {
  // Destination workspace. The dialog renders disabled if null so the
  // sidebar caller does not need to gate it itself.
  workspaceId: RepoId | null;
  workspaceName: string | null;
  open: boolean;
  onOpenChange(open: boolean): void;
  // Fired after a successful import. The sidebar uses this to
  // navigate to the new Job page and surface any warnings as a
  // dismissible banner; tests use it to assert the result shape.
  onImported?(result: ImportJobResult): void;
}

export function ImportJobDialog({
  workspaceId,
  workspaceName,
  open,
  onOpenChange,
  onImported,
}: ImportJobDialogProps) {
  const rpc = useRpc();

  const [bundlePath, setBundlePath] = useState("");
  const [inspecting, setInspecting] = useState(false);
  const [inspect, setInspect] = useState<InspectJobBundleResult | null>(null);
  const [inspectError, setInspectError] = useState<string | null>(null);
  const [rename, setRename] = useState("");
  const [policy, setPolicy] = useState<ImportConflictPolicy>("Refuse");
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  // Reset every open so a half-filled previous attempt does not
  // leak into a fresh import.
  useEffect(() => {
    if (!open) return;
    setBundlePath("");
    setInspect(null);
    setInspectError(null);
    setRename("");
    setPolicy("Refuse");
    setSubmitError(null);
  }, [open]);

  const onInspect = useCallback(async () => {
    const path = bundlePath.trim();
    if (path === "") return;
    setInspecting(true);
    setInspectError(null);
    setInspect(null);
    try {
      const res = await rpc.call("inspect_job_bundle", { bundle_path: path });
      setInspect(res);
    } catch (e) {
      setInspectError(e instanceof Error ? e.message : String(e));
    } finally {
      setInspecting(false);
    }
  }, [bundlePath, rpc]);

  const onImport = useCallback(async () => {
    if (!workspaceId || !inspect) return;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const result = await rpc.call("import_job", {
        workspace_id: workspaceId,
        bundle_path: bundlePath.trim(),
        rename_to: rename.trim() === "" ? null : rename.trim(),
        on_conflict: policy,
      });
      onImported?.(result);
      onOpenChange(false);
    } catch (e) {
      setSubmitError(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  }, [bundlePath, inspect, onImported, onOpenChange, policy, rename, rpc, workspaceId]);

  const canImport =
    workspaceId !== null &&
    inspect !== null &&
    !submitting &&
    !inspecting;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent data-testid="import-job-dialog">
        <DialogHeader>
          <DialogTitle>Import a Job</DialogTitle>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          {workspaceId === null ? (
            <p
              className="text-xs text-destructive"
              data-testid="import-job-no-workspace"
            >
              No active workspace. Attach a workspace first.
            </p>
          ) : (
            <p
              className="text-xs text-muted-foreground"
              data-testid="import-job-destination"
            >
              Importing into <span className="font-medium">{workspaceName ?? workspaceId}</span>.
            </p>
          )}

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="import-job-path">Bundle path</Label>
            <div className="flex gap-2">
              <Input
                id="import-job-path"
                data-testid="import-job-path-input"
                value={bundlePath}
                onChange={(e) => setBundlePath(e.target.value)}
                placeholder="/path/to/<name>.codeless-job"
                autoFocus
              />
              <Button
                type="button"
                variant="outline"
                onClick={() => void onInspect()}
                disabled={bundlePath.trim() === "" || inspecting || submitting}
                data-testid="import-job-inspect-button"
              >
                {inspecting ? "Inspecting…" : "Inspect"}
              </Button>
            </div>
            {inspectError ? (
              <p
                className="text-xs text-destructive"
                data-testid="import-job-inspect-error"
              >
                {inspectError}
              </p>
            ) : null}
          </div>

          {inspect ? (
            <ManifestPreview inspect={inspect} />
          ) : (
            <p className="text-xs text-muted-foreground">
              Pick a `.codeless-job` bundle to preview its manifest.
            </p>
          )}

          {inspect && inspect.local_warnings.length > 0 ? (
            <WarningsBanner
              warnings={inspect.local_warnings.map((w) => w.message)}
              testid="import-job-warnings"
            />
          ) : null}

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="import-job-rename">Rename to (optional)</Label>
            <Input
              id="import-job-rename"
              data-testid="import-job-rename-input"
              value={rename}
              onChange={(e) => setRename(e.target.value)}
              placeholder={inspect?.manifest.source.job_name ?? "<job name>"}
              disabled={!inspect}
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="import-job-policy">On name conflict</Label>
            <Select
              value={policy}
              onValueChange={(v) => setPolicy(v as ImportConflictPolicy)}
            >
              <SelectTrigger
                id="import-job-policy"
                data-testid="import-job-policy-select"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="Refuse">
                  Refuse (default) — surface the existing Job
                </SelectItem>
                <SelectItem value="Suffix">
                  Suffix — import under a new name
                </SelectItem>
                <SelectItem value="Replace">
                  Replace — drop the existing Job's rows
                </SelectItem>
              </SelectContent>
            </Select>
          </div>

          {submitError ? (
            <p
              className="text-xs text-destructive"
              data-testid="import-job-submit-error"
            >
              {submitError}
            </p>
          ) : null}
        </div>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={submitting}
            data-testid="import-job-cancel-button"
          >
            Cancel
          </Button>
          <Button
            type="button"
            onClick={() => void onImport()}
            disabled={!canImport}
            data-testid="import-job-submit-button"
          >
            {submitting ? "Importing…" : "Import"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ManifestPreview({ inspect }: { inspect: InspectJobBundleResult }) {
  const m = inspect.manifest;
  return (
    <div
      className="rounded-md border border-border/60 bg-card/40 p-3 text-xs"
      data-testid="import-job-manifest-preview"
    >
      <div className="grid grid-cols-[auto,1fr] gap-x-3 gap-y-1">
        <span className="text-muted-foreground">Job</span>
        <span className="font-medium">{m.source.job_name}</span>
        <span className="text-muted-foreground">Source</span>
        <span className="truncate font-mono">
          {m.source.repo_url}@{m.source.repo_commit.slice(0, 7)}
        </span>
        <span className="text-muted-foreground">Workspace</span>
        <span>{m.source.workspace_name}</span>
        <span className="text-muted-foreground">Runs</span>
        <span>{m.source.run_count}</span>
        <span className="text-muted-foreground">Events</span>
        <span>{m.content.total_events}</span>
        <span className="text-muted-foreground">Exported</span>
        <span>
          {m.exported_at} (codeless {m.exporter.codeless_version})
        </span>
        <span className="text-muted-foreground">Size</span>
        <span>{formatBytes(inspect.bytes)}</span>
      </div>
    </div>
  );
}

// Reused by the post-import banner on the Job page (mounted via the
// `onImported` callback). Lives here because the warning shape is
// dialog-scoped wire data; if a third caller wants it, lift to a
// shared `ImportWarningsBanner.tsx`.
export function WarningsBanner({
  warnings,
  testid,
}: {
  warnings: string[];
  testid?: string;
}) {
  if (warnings.length === 0) return null;
  return (
    <div
      className="rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs text-amber-700 dark:text-amber-300"
      data-testid={testid}
    >
      <div className="mb-1 font-medium">Warnings</div>
      <ul className="list-disc pl-4">
        {warnings.map((w, i) => (
          <li key={i}>{w}</li>
        ))}
      </ul>
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MiB`;
}
