import { useCallback, useEffect, useState } from "react";
import { useRpc, type AssistantThread } from "@/lib/rpc";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { CommonChat } from "@/modules/chat";

// Stage-6 `/assistant` surface. Two-pane layout: a thread-list rail on
// the left, the selected thread's `CommonChat` on the right. Threads
// and messages live in SQLite (Decisions §2); the rail and the chat
// view are thin selectors over `assistant.*` RPCs — no client-owned
// authoritative state. The richer pieces (attachments, action cards,
// streaming responder) slot in over later stages without rewriting
// this scaffold.
export function AssistantPage() {
  const rpc = useRpc();
  const [threads, setThreads] = useState<AssistantThread[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const refresh = useCallback(
    async (preferSelectId?: string) => {
      setErr(null);
      try {
        const res = await rpc.call("list_assistant_threads", {});
        setThreads(res.threads);
        setLoaded(true);
        setSelectedId((prev) => {
          if (preferSelectId) return preferSelectId;
          if (prev && res.threads.some((t) => t.id === prev)) return prev;
          return res.threads[0]?.id ?? null;
        });
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
        setLoaded(true);
      }
    },
    [rpc],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onNew = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setErr(null);
    try {
      const created = await rpc.call("create_assistant_thread", {});
      await refresh(created.id);
    } catch (e: unknown) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [busy, rpc, refresh]);

  const onDelete = useCallback(
    async (id: string) => {
      if (busy) return;
      setBusy(true);
      setErr(null);
      try {
        await rpc.call("delete_assistant_thread", { thread_id: id });
        // After delete, drop selection if we just removed it; the
        // refresh will pick whichever thread now sits at the top.
        setSelectedId((prev) => (prev === id ? null : prev));
        await refresh();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [busy, rpc, refresh],
  );

  const selected = threads.find((t) => t.id === selectedId) ?? null;

  return (
    <div className="flex h-full min-h-0 w-full">
      <aside className="flex w-64 min-w-[16rem] flex-col border-r border-border/60 bg-card">
        <div className="flex items-center justify-between border-b border-border/60 px-3 py-2">
          <h2 className="text-sm font-medium">Threads</h2>
          <Button size="sm" onClick={onNew} disabled={busy}>
            New
          </Button>
        </div>
        <ScrollArea className="min-h-0 flex-1">
          <ul className="flex flex-col">
            {!loaded ? (
              <li className="px-3 py-2 text-xs text-muted-foreground">
                Loading…
              </li>
            ) : threads.length === 0 ? (
              <li className="px-3 py-4 text-xs text-muted-foreground">
                No threads yet. Hit New to start one.
              </li>
            ) : (
              threads.map((t) => (
                <li
                  key={t.id}
                  className={cn(
                    "group flex items-center gap-1 border-b border-border/40 px-2 py-2",
                    selectedId === t.id && "bg-accent/40",
                  )}
                >
                  <button
                    type="button"
                    onClick={() => setSelectedId(t.id)}
                    className="flex-1 truncate text-left text-sm hover:underline"
                  >
                    {t.title}
                  </button>
                  <button
                    type="button"
                    aria-label="Delete thread"
                    onClick={() => void onDelete(t.id)}
                    disabled={busy}
                    className="rounded px-1.5 py-0.5 text-xs text-muted-foreground opacity-0 hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100"
                  >
                    Delete
                  </button>
                </li>
              ))
            )}
          </ul>
        </ScrollArea>
        {err && (
          <div className="border-t border-destructive/40 bg-destructive/10 px-3 py-2 text-[11px] text-destructive">
            {err}
          </div>
        )}
      </aside>

      <section className="flex min-w-0 flex-1 flex-col">
        {selected ? (
          <CommonChat
            kind="assistant"
            thread={selected}
            onThreadTouched={() => void refresh(selected.id)}
          />
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            Select or create a thread to start chatting.
          </div>
        )}
      </section>
    </div>
  );
}
