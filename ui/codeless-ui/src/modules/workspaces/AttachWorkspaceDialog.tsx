// Attach-workspace modal. Drives the picker -> debounced validator
// -> confirm round-trip specified in §"Attach modal" of
// DOCS/WORKSPACE-ATTACH.md. Three responsibilities, all in one file
// because they are tightly coupled to the same form state:
//
//   1. Path entry: a free-text input + a shell-injected `PathPicker`
//      "browse" button. The picker is the *only* shell-visible split
//      in this flow; the validator round-trip is identical on every
//      shell.
//
//   2. Live validation: `validate_workspace_path` fires on every
//      change with a ~200ms debounce (the server enforces a ~5/s
//      token-bucket cap per connection — debouncing here keeps a
//      runaway picker from getting rate-limited and silently
//      breaking the inline checks). The result drives the inline
//      "git repo / readable / writable" indicators and the
//      `disabled` state of the Attach button: any `WorkspaceProblem`
//      in the result disables submit.
//
//   3. Confirm: clicking Attach calls `add_repo` (best-effort — if
//      the path already has a `repos` row we reuse it) then
//      `attach_workspace`, and on success funnels the
//      `AttachedWorkspace` through `useWorkspacesStore.applyAttached`
//      so the table reflects immediately without waiting for the
//      `workspace_attached` event the runtime does not yet emit.

import { useCallback, useEffect, useRef, useState } from "react";

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
  AttachWorkspaceArgs,
  Repo,
  ServerInfo,
  ValidateWorkspacePathResult,
  WorkspaceProblem,
} from "@/lib/rpc/wire";
import { usePathPicker } from "@/lib/shell";
import { cn } from "@/lib/utils";

import { useWorkspacesStore } from "./store";

// Server-enforced rate limit on `validate_workspace_path` is ~5/s per
// connection (§"RPC additions" of the doc). Anything below ~200ms
// would burn that budget on rapid typing; anything above ~400ms
// makes the indicators feel laggy. 250ms threads the needle.
const VALIDATE_DEBOUNCE_MS = 250;

interface AttachWorkspaceDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
}

export function AttachWorkspaceDialog({
  open,
  onOpenChange,
}: AttachWorkspaceDialogProps) {
  const rpc = useRpc();
  const picker = usePathPicker();

  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [nameTouched, setNameTouched] = useState(false);
  const [runner, setRunner] = useState<string>("");
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [validation, setValidation] = useState<ValidateWorkspacePathResult | null>(
    null,
  );
  const [validating, setValidating] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  // Reset every time the dialog opens so a previous half-filled
  // session doesn't leak into a fresh attach attempt.
  useEffect(() => {
    if (!open) return;
    setPath("");
    setName("");
    setNameTouched(false);
    setRunner("");
    setValidation(null);
    setSubmitError(null);
    rpc.serverInfo().then(setInfo).catch(() => setInfo(null));
  }, [open, rpc]);

  const debounceRef = useRef<number | null>(null);
  const inflightRef = useRef<AbortController | null>(null);

  useEffect(() => {
    if (!open) return;
    if (debounceRef.current !== null) {
      window.clearTimeout(debounceRef.current);
      debounceRef.current = null;
    }
    if (path.trim() === "") {
      setValidation(null);
      setValidating(false);
      return;
    }
    setValidating(true);
    debounceRef.current = window.setTimeout(() => {
      // Cancel any in-flight call so a fast typist's later result
      // wins over an earlier slow one. Abort is signalled via the
      // shared abort controller; the RPC layer respects it where it
      // can and the result race is decided by the `current` check.
      inflightRef.current?.abort();
      const ac = new AbortController();
      inflightRef.current = ac;
      rpc
        .call("validate_workspace_path", { path: path.trim() })
        .then((res) => {
          if (ac.signal.aborted) return;
          setValidation(res);
          setValidating(false);
          if (!nameTouched) {
            const basename = deriveBasename(res.canonical ?? path.trim());
            if (basename) setName(basename);
          }
        })
        .catch(() => {
          if (ac.signal.aborted) return;
          setValidating(false);
        });
    }, VALIDATE_DEBOUNCE_MS);
    return () => {
      if (debounceRef.current !== null) {
        window.clearTimeout(debounceRef.current);
        debounceRef.current = null;
      }
    };
  }, [path, open, rpc, nameTouched]);

  const onBrowse = useCallback(async () => {
    const picked = await picker.pickDirectory({ startPath: path || undefined });
    if (picked) setPath(picked);
  }, [picker, path]);

  const canSubmit =
    open &&
    !submitting &&
    !validating &&
    validation !== null &&
    validation.problems.length === 0 &&
    name.trim() !== "";

  const onAttach = useCallback(async () => {
    if (!validation || validation.problems.length > 0) return;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const canonical = validation.canonical ?? path.trim();
      // §"Attach modal": `add_repo` (if no row yet) + `attach_workspace`
      // in one transaction. The UI half discovers "no row yet" by
      // scanning `list_repos` against `local_path`; if a row exists we
      // skip `add_repo` and attach the existing one. The server's
      // unique index on `attached_workspaces.fs_root_canonical` is the
      // authoritative collision check — this list_repos pass is a
      // best-effort optimisation to avoid a spurious `Conflict`.
      const repos = await rpc.call("list_repos", {});
      const existing = repos.repos.find((r) => r.local_path === canonical);
      let repo: Repo;
      if (existing) {
        repo = existing;
      } else {
        repo = await rpc.call("add_repo", {
          name: name.trim(),
          clone_url: "",
          default_branch: validation.default_branch ?? "main",
          local_path: canonical,
          git_auth: { kind: "ssh", key_path: "" },
          concurrency_cap: null,
          default_runner: runner === "" ? null : runner,
        });
      }
      const args: AttachWorkspaceArgs = {
        repo_id: repo.id,
        fs_root_override: canonical === repo.local_path ? null : canonical,
      };
      const res = await rpc.call("attach_workspace", args);
      // Reflect immediately. The runtime does not emit
      // `workspace-attached` yet (see useWorkspacesSync); when it
      // does, `applyAttached` is idempotent on `repo_id` so the live
      // event is a no-op.
      useWorkspacesStore.getState().applyAttached(res.workspace);
      onOpenChange(false);
    } catch (e) {
      setSubmitError(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  }, [name, onOpenChange, path, rpc, runner, validation]);

  const runners = info?.available_cli_runners ?? [];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent data-testid="attach-workspace-dialog">
        <DialogHeader>
          <DialogTitle>Attach a workspace</DialogTitle>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="attach-ws-path">Path</Label>
            <div className="flex gap-2">
              <Input
                id="attach-ws-path"
                data-testid="attach-ws-path-input"
                value={path}
                onChange={(e) => setPath(e.target.value)}
                placeholder="/home/me/code/myproject"
                autoFocus
              />
              <Button
                type="button"
                variant="outline"
                onClick={onBrowse}
                data-testid="attach-ws-browse-button"
              >
                Browse…
              </Button>
            </div>
            <PathValidationSummary
              validating={validating}
              validation={validation}
              path={path}
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="attach-ws-name">Name</Label>
            <Input
              id="attach-ws-name"
              data-testid="attach-ws-name-input"
              value={name}
              onChange={(e) => {
                setName(e.target.value);
                setNameTouched(true);
              }}
              placeholder="myproject"
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="attach-ws-runner">Runner</Label>
            {runners.length === 0 ? (
              <p
                className="text-xs text-muted-foreground"
                data-testid="attach-ws-runner-empty"
              >
                No runners installed. Attach proceeds editor-only.
              </p>
            ) : (
              <Select value={runner} onValueChange={setRunner}>
                <SelectTrigger
                  id="attach-ws-runner"
                  data-testid="attach-ws-runner-select"
                >
                  <SelectValue placeholder="Select a runner…" />
                </SelectTrigger>
                <SelectContent>
                  {runners.map((r) => (
                    <SelectItem key={r} value={r}>
                      {r}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          </div>

          {submitError ? (
            <p
              className="text-xs text-destructive"
              data-testid="attach-ws-submit-error"
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
            data-testid="attach-ws-cancel-button"
          >
            Cancel
          </Button>
          <Button
            type="button"
            onClick={onAttach}
            disabled={!canSubmit}
            data-testid="attach-ws-submit-button"
          >
            {submitting ? "Attaching…" : "Attach workspace"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface PathValidationSummaryProps {
  validating: boolean;
  validation: ValidateWorkspacePathResult | null;
  path: string;
}

function PathValidationSummary({
  validating,
  validation,
  path,
}: PathValidationSummaryProps) {
  if (path.trim() === "") {
    return (
      <p className="text-xs text-muted-foreground">
        Enter or pick a directory on this machine.
      </p>
    );
  }
  if (validating || validation === null) {
    return (
      <p className="text-xs text-muted-foreground" data-testid="attach-ws-validating">
        Checking path…
      </p>
    );
  }
  return (
    <div
      className="flex flex-col gap-0.5 text-xs"
      data-testid="attach-ws-validation"
    >
      <div className="flex flex-wrap gap-x-3 gap-y-0.5">
        <Indicator ok={validation.is_dir} label="directory" />
        <Indicator ok={validation.readable} label="readable" />
        <Indicator ok={validation.writable} label="writable" />
        <Indicator
          ok={validation.is_git_repo}
          label={
            validation.is_git_repo
              ? `git repo${
                  validation.default_branch
                    ? ` (${validation.default_branch})`
                    : ""
                }`
              : "not a git repo"
          }
          warning={!validation.is_git_repo}
        />
      </div>
      {validation.problems.length > 0 ? (
        <ul
          className="list-disc pl-4 text-destructive"
          data-testid="attach-ws-problems"
        >
          {validation.problems.map((p, i) => (
            <li key={i}>{describeProblem(p)}</li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

function Indicator({
  ok,
  label,
  warning,
}: {
  ok: boolean;
  label: string;
  warning?: boolean;
}) {
  return (
    <span
      className={cn(
        ok
          ? "text-emerald-600 dark:text-emerald-400"
          : warning
            ? "text-amber-600 dark:text-amber-400"
            : "text-destructive",
      )}
    >
      {ok ? "✓" : warning ? "!" : "✗"} {label}
    </span>
  );
}

function describeProblem(p: WorkspaceProblem): string {
  if (typeof p === "string") {
    switch (p) {
      case "not-a-directory":
        return "Not a directory.";
      case "not-readable":
        return "Directory is not readable.";
      case "not-writable":
        return "Directory is not writable.";
      case "not-a-git-repo":
        return "Not a git repository.";
      case "system-path":
        return "Refusing to attach a system path (e.g. /, /etc, ~/.ssh).";
      case "symlink-outside-home":
        return "Symlink target lives outside the user's home directory.";
    }
  }
  if ("inside-another-workspace" in p) {
    return `Inside another attached workspace at ${p["inside-another-workspace"]["other-root"]}.`;
  }
  return "Unknown problem.";
}

function deriveBasename(p: string): string {
  const trimmed = p.replace(/[\\/]+$/, "");
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
}
