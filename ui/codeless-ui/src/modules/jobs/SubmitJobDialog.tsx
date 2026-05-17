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
import { useRpc, type Repo } from "@/lib/rpc";

import {
  JobComposer,
  composerToSubmitArgs,
  useJobComposerState,
} from "./composer";

interface Props {
  repo: Repo;
  trigger?: React.ReactNode;
}

// Thin shell around `JobComposer`. Owns dialog open-state, the
// `submit_job` call, the error banner, and the per-open
// `/server/info` fetch. Every field, every validation rule and the
// wire-shape mapping live in the composer module so the assistant's
// `draft_job` card can reuse them verbatim — see
// `DOCS/SCOPE-ASSISTANT-PARITY.md` W2.
export function SubmitJobDialog({ repo, trigger }: Props) {
  const rpc = useRpc();
  const [open, setOpen] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Composer state lives in `useJobComposerState`; React preserves
  // it across renders while the dialog is open and discards it when
  // the dialog tree unmounts on close.
  const state = useJobComposerState({ repo });

  // Fetch /server/info once per dialog-open. Re-running on each
  // open is cheap (a single unauthenticated GET) and reflects
  // post-boot state changes — e.g. operator restarted with
  // --enable-claude.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    rpc
      .serverInfo()
      .then((i) => {
        if (cancelled) return;
        state.setInfo(i);
        // Prefer the repo's saved default when the server still
        // advertises that runner; otherwise honour the server's
        // own default flag. This keeps repo-level preferences
        // sticky while not silently submitting jobs against a
        // runner the operator has since disabled.
        const repoPick = repo.default_runner
          ? i.runners.find((r) => r.id === repo.default_runner)
          : undefined;
        const serverDefault = i.runners.find((r) => r.default);
        state.setRunner(
          repoPick?.id ?? serverDefault?.id ?? i.runners[0]?.id ?? "",
        );
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, rpc, repo.default_runner]);

  const submit = async () => {
    if (!state.canSubmit) {
      setError("fix the highlighted fields before submitting");
      return;
    }
    setSubmitting(true);
    setError(null);
    // Hard timeout so a hung transport (server down mid-submit, an
    // unreachable mock client, an SSE proxy stalling the fetch)
    // does not leave the button frozen on "submitting…". 10s is
    // well above the trait method's normal latency on a healthy
    // core.
    const timer = window.setTimeout(() => {
      setError("submit timed out after 10s — check the server is reachable");
      setSubmitting(false);
    }, 10_000);
    try {
      const job = await rpc.call("submit_job", composerToSubmitArgs(state));
      // eslint-disable-next-line no-console
      console.log("submit_job ok", job);
      setOpen(false);
      state.setName("");
      state.setBranch("");
      state.setBranchTouched(false);
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
            Pick a name. The job lands as a draft with{" "}
            <code>template.yaml</code>, <code>SCOPE.md</code>, and{" "}
            <code>WORKFLOW.md</code> already on disk — edit them in the
            SPEC pane, then click <code>run</code>.
          </DialogDescription>
        </DialogHeader>
        <JobComposer state={state} />
        {error && <div className="text-destructive text-xs">{error}</div>}
        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            cancel
          </Button>
          <Button onClick={submit} disabled={submitting || !state.canSubmit}>
            {submitting
              ? "submitting…"
              : state.runImmediately
                ? "submit + run"
                : "save as draft"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
