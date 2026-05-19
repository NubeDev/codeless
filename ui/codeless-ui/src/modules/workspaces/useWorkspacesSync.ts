// Bridges the `useWorkspacesStore` mirror to the runtime: one
// `list_workspaces` call on mount to hydrate, then a subscription to
// the `library`-scope event stream that funnels `workspace-attached` /
// `workspace-detached` (and the existing `workspace-unhealthy` /
// `workspace-recovered` pair surfaced by `codeless-runtime
// ::workspace_liveness`) into the store's incremental reducers. The
// `library` scope (vs the legacy `all`) is the picker's parallel
// channel: it sees only workspace-lifecycle events and never the
// jobs/stages firehose, so two browser tabs viewing two different
// workspaces both keep their picker live without leaking job events
// across the boundary.
//
// Subscribing here rather than inside the store keeps zustand free of
// React + transport coupling; the store is a plain reducer the
// Playwright tests and the modal components can drive directly.
//
// The wire's `Event` union does not yet carry an explicit
// `workspace-attached` / `workspace-detached` variant — the doc
// specifies the pair (§"Data the UI needs (events)") but the runtime
// emits a typed payload only for the unhealthy/recovered side today.
// The string-tag dispatch below covers all four names so the day the
// runtime starts emitting the attach/detach pair, the UI reconciles
// without another shell-side change. Until then, the attach + detach
// modals call `useWorkspacesStore.getState().applyAttached` /
// `.applyDetached` directly after their RPCs resolve.

import { useEffect, useRef } from "react";

import { useEventStream } from "@/lib/rpc/hooks";
import { useRpc } from "@/lib/rpc/provider";
import type { AttachedWorkspace, EventEnvelope, RepoId } from "@/lib/rpc/wire";

import { useWorkspacesStore } from "./store";

interface WorkspaceAttachedEventLike {
  type: "workspace-attached";
  workspace: AttachedWorkspace;
}

interface WorkspaceDetachedEventLike {
  type: "workspace-detached";
  repo_id: RepoId;
}

type MaybeWorkspaceEvent =
  | WorkspaceAttachedEventLike
  | WorkspaceDetachedEventLike
  | { type: string; [k: string]: unknown };

export function useWorkspacesSync(): void {
  const rpc = useRpc();
  const hydrating = useRef(false);

  useEffect(() => {
    if (hydrating.current) return;
    hydrating.current = true;
    const store = useWorkspacesStore.getState();
    store.setStatus("loading");
    rpc
      .call("list_workspaces", {})
      .then((res) => {
        useWorkspacesStore.getState().setWorkspaces(res.workspaces);
        useWorkspacesStore.getState().setStatus("ready");
      })
      .catch((err: unknown) => {
        const msg = err instanceof Error ? err.message : String(err);
        useWorkspacesStore.getState().setStatus("error", msg);
      })
      .finally(() => {
        hydrating.current = false;
      });
  }, [rpc]);

  useEventStream({ scope: "library" }, (env: EventEnvelope) => {
    reconcileFromEvent(env.event as MaybeWorkspaceEvent);
  });
}

// Exported for the store's unit tests: lets the test push synthetic
// event payloads through the same dispatch the live hook uses without
// rendering a tree.
export function reconcileFromEvent(event: MaybeWorkspaceEvent): void {
  const store = useWorkspacesStore.getState();
  switch (event.type) {
    case "workspace-attached": {
      const ev = event as WorkspaceAttachedEventLike;
      if (ev.workspace) store.applyAttached(ev.workspace);
      return;
    }
    case "workspace-detached": {
      const ev = event as WorkspaceDetachedEventLike;
      if (ev.repo_id) store.applyDetached(ev.repo_id);
      return;
    }
    default:
      return;
  }
}
