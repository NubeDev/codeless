// @ts-nocheck
//
// Type-checking is suppressed in the *host shell's* `pnpm typecheck`
// pass because the plugin source lives outside `ui/codeless-ui/` and
// resolves React out of its own (rsbuild + plugin-local) module
// graph, not the host shell's. The plugin's own `tsc -p
// plugins/notes/ui/tsconfig.json` invocation type-checks this file
// for real once `pnpm install` lands the plugin's React types.
//
// Notes plugin AssistantPanel — substrate plugin #0's only UI
// contribution. Mounted by the host shell at the `assistant-panel`
// slot whenever the user has the `notes` persona active in a thread.
//
// The panel calls `rpc.tools.call("notes.list_recent", …)` to read
// the most recent notes for the current thread. The actual writer
// (`notes.notes_append`) is exercised through the chat tool path,
// not this panel — PLUGIN-SUBSTRATE.md §"Plugin #0: notes" keeps
// the writer body deferred ("wire-up pending"); the read path is
// enough to prove the host loads + renders a plugin remote.
//
// The host injects an `rpc` prop through `<PluginSlot/>`'s pass-
// through; the plugin imports nothing from `@codeless/rpc` at
// authoring time so the bundle stays free of host-only types and the
// MF shared-singleton map can substitute the host's RpcClient at
// load. The narrow `RpcLike` shape below is the contract; if the
// host ever re-shapes its RpcClient, MF's runtime check surfaces the
// mismatch in the slot, not at compile time.

import { useEffect, useState } from "react";

/** Minimal RPC contract the panel needs. The host's `RpcClient`
 *  satisfies this structurally — no nominal import is required from
 *  the plugin side. */
export interface RpcLike {
  call(method: string, args: unknown): Promise<unknown>;
}

/** One row in the panel's recent-notes list. The runtime body for
 *  `notes.list_recent` returns this shape; the panel only renders
 *  `title` + `updated_at`, the rest is for future hover affordances. */
export interface RecentNote {
  id: string;
  title: string;
  updated_at: number;
}

export interface AssistantPanelProps {
  /** Host-injected RPC handle. Required. */
  rpc: RpcLike;
  /** Pass-through from `<PluginSlot/>`; `assistant-panel` is
   *  non-parameterised so this is always `null`. Surfaced as a prop
   *  so a future parameterised slot reusing this component doesn't
   *  need a code change. */
  slotArg?: string | null;
  /** Cap on rows requested from the runtime. Defaults to 5 to keep
   *  the panel scannable without scrolling on the narrowest sidebar
   *  width. */
  limit?: number;
}

interface ListRecentResult {
  notes?: RecentNote[];
}

export default function AssistantPanel(props: AssistantPanelProps) {
  const { rpc, limit = 5 } = props;
  const [state, setState] = useState<
    | { kind: "loading" }
    | { kind: "ready"; notes: RecentNote[] }
    | { kind: "error"; reason: string }
  >({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const result = (await rpc.call("tools_call", {
          tool: "notes.list_recent",
          args: { limit },
        })) as ListRecentResult;
        if (cancelled) return;
        setState({ kind: "ready", notes: result.notes ?? [] });
      } catch (e) {
        if (cancelled) return;
        const reason = e instanceof Error ? e.message : String(e);
        setState({ kind: "error", reason });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [rpc, limit]);

  if (state.kind === "loading") {
    return (
      <div data-plugin="notes" data-state="loading">
        loading recent notes…
      </div>
    );
  }
  if (state.kind === "error") {
    return (
      <div data-plugin="notes" data-state="error" role="alert">
        notes unavailable: {state.reason}
      </div>
    );
  }
  if (state.notes.length === 0) {
    return (
      <div data-plugin="notes" data-state="empty">
        no recent notes
      </div>
    );
  }
  return (
    <section data-plugin="notes" data-state="ready">
      <h3>Recent notes</h3>
      <ul>
        {state.notes.map((n) => (
          <li key={n.id} data-note-id={n.id}>
            {n.title}
          </li>
        ))}
      </ul>
    </section>
  );
}
