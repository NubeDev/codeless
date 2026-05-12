import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
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
import { Textarea } from "@/components/ui/textarea";
import { useRpc, type Repo, type RunnerInfo, type ServerInfo } from "@/lib/rpc";

interface Props {
  repo: Repo;
  // Defaults pulled from the repo row; tunable in the form.
  trigger?: React.ReactNode;
}

// 6 lowercase base36 chars: enough to disambiguate concurrent jobs in a
// session without forcing the user to read a full ULID. The branch is
// the one durable artefact of a job; collisions just bounce off
// `git worktree add` and the user retries.
function freshBranchSuffix(): string {
  return Math.floor(Math.random() * 36 ** 6)
    .toString(36)
    .padStart(6, "0");
}

function freshBranch(): string {
  return `codeless/job-${freshBranchSuffix()}`;
}

// `mock` is the no-prereqs no-side-effects runner; tagging it as "demo"
// in the dropdown stops users from picking it expecting real edits.
// Other runner ids surface as-is.
function runnerLabel(r: RunnerInfo): string {
  return r.id === "mock" ? "mock (demo)" : r.id;
}

export function SubmitJobDialog({ repo, trigger }: Props) {
  const rpc = useRpc();
  const [open, setOpen] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [branch, setBranch] = useState(freshBranch);
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [runner, setRunner] = useState<string>(repo.default_runner ?? "");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Fetch /server/info once per dialog-open. Re-running on each open
  // is cheap (a single unauthenticated GET) and reflects post-boot
  // state changes — e.g. operator restarted with --enable-claude.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    rpc
      .serverInfo()
      .then((i) => {
        if (cancelled) return;
        setInfo(i);
        // Prefer the repo's saved default when the server still
        // advertises that runner; otherwise honour the server's
        // own default flag. This keeps repo-level preferences
        // sticky while not silently submitting jobs against a
        // runner the operator has since disabled.
        const repoPick = repo.default_runner
          ? i.runners.find((r) => r.id === repo.default_runner)
          : undefined;
        const serverDefault = i.runners.find((r) => r.default);
        setRunner(repoPick?.id ?? serverDefault?.id ?? i.runners[0]?.id ?? "");
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setError(
          `could not load runner list: ${e instanceof Error ? e.message : String(e)}`,
        );
      });
    return () => {
      cancelled = true;
    };
  }, [open, rpc, repo.default_runner]);

  const submit = async () => {
    setSubmitting(true);
    setError(null);
    // Hard timeout so a hung transport (server down mid-submit, an
    // unreachable mock client, an SSE proxy stalling the fetch) does
    // not leave the button frozen on "submitting…". 10s is well above
    // the trait method's normal latency on a healthy core.
    const timer = window.setTimeout(() => {
      setError("submit timed out after 10s — check the server is reachable");
      setSubmitting(false);
    }, 10_000);
    try {
      const job = await rpc.call("submit_job", {
        repo_id: repo.id,
        prompt: prompt || null,
        template_yaml: null,
        runner,
        branch,
        cost_cap_cents: 500,
        wall_clock_cap_ms: 30 * 60 * 1000,
      });
      // eslint-disable-next-line no-console
      console.log("submit_job ok", job);
      setOpen(false);
      setPrompt("");
      setBranch(freshBranch());
    } catch (e) {
      // eslint-disable-next-line no-console
      console.error("submit_job failed", e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      window.clearTimeout(timer);
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        {trigger ?? <Button size="sm">new job</Button>}
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Submit job — {repo.name}</DialogTitle>
          <DialogDescription>
            Queue a new job in this repo. The core will provision a worktree
            and run it on the chosen runner.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="prompt">Prompt</Label>
            <Textarea
              id="prompt"
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Describe what the agent should do…"
              rows={5}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="branch">Branch</Label>
              <Input
                id="branch"
                value={branch}
                onChange={(e) => setBranch(e.target.value)}
              />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="runner">Runner</Label>
              <Select
                value={runner}
                onValueChange={setRunner}
                disabled={!info || info.runners.length === 0}
              >
                <SelectTrigger id="runner">
                  <SelectValue placeholder={info ? "Select runner" : "Loading…"} />
                </SelectTrigger>
                <SelectContent>
                  {info?.runners.map((r) => (
                    <SelectItem key={r.id} value={r.id}>
                      {runnerLabel(r)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>
          {error && <div className="text-destructive text-xs">{error}</div>}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            cancel
          </Button>
          <Button onClick={submit} disabled={submitting}>
            {submitting ? "submitting…" : "submit"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
