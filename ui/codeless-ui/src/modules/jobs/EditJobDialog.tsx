import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
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
import { useRpc, type Job, type ServerInfo } from "@/lib/rpc";

const PERMISSION_MODES: { id: string; label: string }[] = [
  { id: "bypass", label: "Bypass" },
  { id: "accept_edits", label: "Accept edits" },
  { id: "plan", label: "Plan" },
  { id: "default", label: "Default" },
];

const NO_EFFORT = "__none__";

const EFFORT_LEVELS: { id: string; label: string }[] = [
  { id: NO_EFFORT, label: "Default (no extra thinking)" },
  { id: "low", label: "Low — think" },
  { id: "medium", label: "Medium — think hard" },
  { id: "high", label: "High — ultrathink" },
];

const RUNNER_MODELS: Record<string, { id: string; label: string }[]> = {
  claude: [
    { id: "claude-opus-4-7", label: "Claude Opus 4.7" },
    { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6" },
    { id: "claude-haiku-4-5", label: "Claude Haiku 4.5" },
  ],
};

const SERVER_PICK = "__server_default__";

interface Props {
  job: Job;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: (updated: Job) => void;
}

export function EditJobDialog({ job, open, onOpenChange, onSaved }: Props) {
  const rpc = useRpc();
  const [info, setInfo] = useState<ServerInfo | null>(null);

  const [runner, setRunner] = useState(job.runner);
  const [model, setModel] = useState(job.model ?? SERVER_PICK);
  const [permissionMode, setPermissionMode] = useState(
    job.permission_mode ?? SERVER_PICK,
  );
  const [effort, setEffort] = useState(job.effort || NO_EFFORT);
  const [branch, setBranch] = useState(job.branch);
  const [costCapUsd, setCostCapUsd] = useState(
    String(job.cost_cap_cents / 100),
  );
  const [wallClockMin, setWallClockMin] = useState(
    String(Math.round(job.wall_clock_cap_ms / 60_000)),
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const costCapCents = Math.max(
    1,
    Math.round(parseFloat(costCapUsd || "0") * 100),
  );
  const wallClockMs = Math.max(
    1,
    Math.round(parseFloat(wallClockMin || "0") * 60_000),
  );

  const models = RUNNER_MODELS[runner] ?? [];

  // Re-seed form when job prop changes (e.g. refetch after SSE update).
  useEffect(() => {
    setRunner(job.runner);
    setModel(job.model ?? SERVER_PICK);
    setPermissionMode(job.permission_mode ?? SERVER_PICK);
    setEffort(job.effort || NO_EFFORT);
    setBranch(job.branch);
    setCostCapUsd(String(job.cost_cap_cents / 100));
    setWallClockMin(String(Math.round(job.wall_clock_cap_ms / 60_000)));
  }, [job]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    rpc.serverInfo().then((i) => {
      if (!cancelled) setInfo(i);
    });
    return () => {
      cancelled = true;
    };
  }, [open, rpc]);

  const editable =
    job.status === "draft" ||
    job.status === "stopped" ||
    job.status === "failed" ||
    job.status === "completed";

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      const updated = await rpc.call("update_job", {
        job_id: job.id,
        runner,
        model: model === SERVER_PICK ? "" : model,
        permission_mode:
          permissionMode === SERVER_PICK ? "" : permissionMode,
        effort: effort === NO_EFFORT ? "" : effort,
        cost_cap_cents: costCapCents,
        wall_clock_cap_ms: wallClockMs,
        branch,
      });
      onSaved(updated);
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Edit job</DialogTitle>
          <DialogDescription>
            {editable
              ? "Change runner settings, caps, or branch."
              : "This job is currently active and cannot be edited."}
          </DialogDescription>
        </DialogHeader>

        {!editable ? (
          <p className="text-muted-foreground py-4 text-sm">
            Stop the job first to edit its settings.
          </p>
        ) : (
          <div className="grid gap-4 py-2">
            {/* Runner */}
            <div className="grid gap-1.5">
              <Label>Runner</Label>
              <Select value={runner} onValueChange={setRunner}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {(info?.runners ?? [{ id: runner, default: true }]).map(
                    (r) => (
                      <SelectItem key={r.id} value={r.id}>
                        {r.id === "mock" ? "mock (demo)" : r.id}
                      </SelectItem>
                    ),
                  )}
                </SelectContent>
              </Select>
            </div>

            {/* Model */}
            {models.length > 0 && (
              <div className="grid gap-1.5">
                <Label>Model</Label>
                <Select value={model} onValueChange={setModel}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={SERVER_PICK}>
                      Default (server picks)
                    </SelectItem>
                    {models.map((m) => (
                      <SelectItem key={m.id} value={m.id}>
                        {m.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            {/* Permission */}
            {runner === "claude" && (
              <div className="grid gap-1.5">
                <Label>Permission</Label>
                <Select
                  value={permissionMode}
                  onValueChange={setPermissionMode}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={SERVER_PICK}>
                      Default (server picks)
                    </SelectItem>
                    {PERMISSION_MODES.map((p) => (
                      <SelectItem key={p.id} value={p.id}>
                        {p.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            {/* Effort */}
            {runner === "claude" && (
              <div className="grid gap-1.5">
                <Label>Effort</Label>
                <Select value={effort} onValueChange={setEffort}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {EFFORT_LEVELS.map((e) => (
                      <SelectItem key={e.id} value={e.id}>
                        {e.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            {/* Branch */}
            <div className="grid gap-1.5">
              <Label>Branch</Label>
              <Input value={branch} onChange={(e) => setBranch(e.target.value)} />
            </div>

            {/* Caps */}
            <div className="grid grid-cols-2 gap-3">
              <div className="grid gap-1.5">
                <Label>Cost cap (USD)</Label>
                <Input
                  type="number"
                  min="0.01"
                  step="0.01"
                  value={costCapUsd}
                  onChange={(e) => setCostCapUsd(e.target.value)}
                />
              </div>
              <div className="grid gap-1.5">
                <Label>Wall-clock cap (min)</Label>
                <Input
                  type="number"
                  min="1"
                  step="1"
                  value={wallClockMin}
                  onChange={(e) => setWallClockMin(e.target.value)}
                />
              </div>
            </div>

            {error && (
              <p className="text-destructive text-sm">{error}</p>
            )}
          </div>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            cancel
          </Button>
          {editable && (
            <Button onClick={handleSave} disabled={saving}>
              {saving ? "saving…" : "save"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
