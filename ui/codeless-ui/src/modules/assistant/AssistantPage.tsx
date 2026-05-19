import { useCallback, useEffect, useRef, useState } from "react";
import {
  useEventStream,
  useRpc,
  type AssistantThread,
  type EventEnvelope,
} from "@/lib/rpc";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { CommonChat } from "@/modules/chat";
import { PluginSlot } from "@/lib/plugin-host";
import { useAssistantFocus } from "./focusStore";
import { ThreadModeDropdown } from "./ThreadModeDropdown";

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
  const focusThreadId = useAssistantFocus((s) => s.currentThreadId);
  const setFocusThreadId = useAssistantFocus((s) => s.setCurrentThreadId);
  const [selectedId, setSelectedId] = useState<string | null>(focusThreadId);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // The rail's selection and the footer's bound thread are two views
  // of the same pointer (focusStore.currentThreadId). Pushing the
  // selection out keeps the footer in sync with the rail without the
  // rail owning the focus store; the footer is the one that persists
  // the choice across launches.
  const selectThread = useCallback(
    (id: string | null) => {
      setSelectedId(id);
      setFocusThreadId(id);
    },
    [setFocusThreadId],
  );

  const refresh = useCallback(
    async (preferSelectId?: string) => {
      setErr(null);
      try {
        const res = await rpc.call("list_assistant_threads", {});
        setThreads(res.threads);
        setLoaded(true);
        const fallback = res.threads[0]?.id ?? null;
        const next =
          preferSelectId && res.threads.some((t) => t.id === preferSelectId)
            ? preferSelectId
            : selectedId && res.threads.some((t) => t.id === selectedId)
              ? selectedId
              : focusThreadId && res.threads.some((t) => t.id === focusThreadId)
                ? focusThreadId
                : fallback;
        if (next !== selectedId) selectThread(next);
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
        setLoaded(true);
      }
    },
    [rpc, selectedId, focusThreadId, selectThread],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Re-sort fan-out keys off the `assistant-thread-touched` envelope
  // the runtime publishes on every `touch_assistant_thread`. Replaces
  // the legacy `focusStore.refreshTick` polling counter
  // (`DOCS/SCOPE-ASSISTANT-PARITY.md` §W1c). `scope: "all"` is the only
  // filter that captures touches across every thread; the rail
  // discriminates by event variant client-side so it ignores the
  // job-scope traffic the bus also carries.
  //
  // A microtask-coalesced ref collapses the replay backlog (one
  // envelope per historical touch) into a single refresh on first
  // mount, then forwards each live envelope as a refresh trigger. The
  // alternative — passing a non-zero `since` to skip the replay —
  // would require a captured cursor the page does not have at mount.
  const refreshPendingRef = useRef(false);
  const onTouchEvent = useCallback(
    (env: EventEnvelope) => {
      if (env.event.type !== "assistant-thread-touched") return;
      if (refreshPendingRef.current) return;
      refreshPendingRef.current = true;
      queueMicrotask(() => {
        refreshPendingRef.current = false;
        void refresh();
      });
    },
    [refresh],
  );
  useEventStream({ scope: "all" }, onTouchEvent);

  const onNew = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setErr(null);
    try {
      // PS5: a thread declares its persona at creation. The Assistant
      // surface uses the seeded `builtin:general` row until the UI
      // grows a persona picker (PS6's plugin manifest registers more
      // personas and the picker reads from `list_personas`).
      const created = await rpc.call("create_assistant_thread", {
        persona_id: "builtin:general",
      });
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
        if (selectedId === id) selectThread(null);
        await refresh();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [busy, rpc, refresh, selectedId, selectThread],
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
                    onClick={() => selectThread(t.id)}
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
            threadId={selected.id}
            thread={selected}
            onThreadTouched={() => void refresh(selected.id)}
          />
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            Select or create a thread to start chatting.
          </div>
        )}
      </section>

      {/*
        Right-rail context panel — ASSISTANT-SCOPE §1. Hosts the
        per-thread filesystem permission dropdown (job
        `assistant-fs-tools` stage 7) and the plugin federation slot
        below it. Mounted only when a thread is selected so neither
        renders against a null `threadId`.

        Plugin UI federation slot — DOCS/plugins/PLUGIN-UI-FEDERATION.md
        § Slot vocabulary, row "assistant-panel". The slot renders
        nothing when no plugin contributes to it (R6 fallback path).
        When the active thread's persona belongs to a plugin and that
        plugin's manifest declares an `assistant-panel` exposed
        module, the SDK lazy-loads it inside a per-contributor error
        boundary so a misbehaving plugin can never blank the chat
        pane. `threadId` is forwarded as a prop alongside the SDK's
        `slotArg`.
      */}
      {selected && (
        <aside className="flex w-64 min-w-[16rem] flex-col gap-3 border-l border-border/60 bg-card p-3">
          <ThreadModeDropdown
            thread={selected}
            onChanged={() => void refresh(selected.id)}
          />
          <PluginSlot
            id="assistant-panel"
            threadId={selected.id}
            fallback={null}
          />
        </aside>
      )}
    </div>
  );
}
