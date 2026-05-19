import { useCallback, useState } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  useRpc,
  type AssistantThread,
  type AssistantThreadMode,
} from "@/lib/rpc";

// `/assistant` context-panel control (ASSISTANT-SCOPE §1, right rail)
// for the per-thread filesystem-tool permission posture introduced by
// job `assistant-fs-tools`. The displayed value comes from the
// `AssistantThread` row the caller passes in — i.e. the server's
// authoritative state (R4) — not from a local optimistic cache. The
// dropdown calls `set_assistant_thread_mode` and lets the caller
// refresh the thread row so the displayed value reflects what
// SQLite actually stores; a server reject (NotFound, decode error)
// therefore reverts the visible value on its own.
//
// `mode` is optional on the wire (older threads predate the column
// and the server-side default is `read-only` until a migration
// back-fills); falling back here keeps the dropdown stable for any
// row that round-trips before the back-fill lands.

const MODES: { value: AssistantThreadMode; label: string; hint: string }[] = [
  {
    value: "read-only",
    label: "Read-only",
    hint: "Assistant can list, read and search files. No writes.",
  },
  {
    value: "approve-edits",
    label: "Approve edits",
    hint: "Each file write surfaces as an action card you confirm.",
  },
  {
    value: "bypass",
    label: "Bypass",
    hint: "Writes run immediately. Job scopes still pause-gated.",
  },
];

export interface ThreadModeDropdownProps {
  thread: AssistantThread;
  onChanged?: () => void;
}

export function ThreadModeDropdown({
  thread,
  onChanged,
}: ThreadModeDropdownProps) {
  const rpc = useRpc();
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const current: AssistantThreadMode = thread.mode ?? "read-only";

  const onChange = useCallback(
    async (next: string) => {
      if (busy) return;
      const mode = next as AssistantThreadMode;
      if (mode === current) return;
      setBusy(true);
      setErr(null);
      try {
        await rpc.call("set_assistant_thread_mode", {
          thread_id: thread.id,
          mode,
        });
        onChanged?.();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [busy, current, onChanged, rpc, thread.id],
  );

  const hint = MODES.find((m) => m.value === current)?.hint ?? "";

  return (
    <div className="flex flex-col gap-1">
      <label
        htmlFor="assistant-thread-mode"
        className="text-xs font-medium text-muted-foreground"
      >
        Filesystem permission
      </label>
      <Select value={current} onValueChange={onChange} disabled={busy}>
        <SelectTrigger id="assistant-thread-mode" className="h-8 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {MODES.map((m) => (
            <SelectItem key={m.value} value={m.value} className="text-xs">
              {m.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <p className="text-[11px] leading-snug text-muted-foreground">{hint}</p>
      {err && (
        <p className="text-[11px] leading-snug text-destructive">{err}</p>
      )}
    </div>
  );
}
