import { create } from "zustand";

import type { RpcClient } from "@/lib/rpc";
import { getCrossWindowEvents } from "@/lib/shell";

import {
  BUILTIN_AGENTS,
  deletePersonaViaRpc,
  loadAgents,
  loadAgentsFromRpc,
  newAgentId,
  saveActiveAgentId,
  saveCustomAgents,
  upsertPersonaViaRpc,
  type Agent,
} from "../lib/agents";

const CHANGED_EVENT = "codeless://ai-agents-changed";

type AgentsState = {
  hydrated: boolean;
  customAgents: Agent[];
  activeId: string;
  /** All agents, builtin first. */
  all: () => Agent[];
  // `rpc` is optional so legacy call-sites that never wired an
  // `RpcClient` through (tests, the KV-only fallback path) keep
  // working. When supplied, the runtime is the source of truth and
  // the KV cache mirrors it; when omitted, the KV is read directly
  // and writes never round-trip to the server. Mirrors SCOPE.md R4.
  hydrate: (rpc?: RpcClient) => Promise<void>;
  setActiveId: (id: string) => void;
  upsert: (agent: Agent, rpc?: RpcClient) => Promise<void>;
  remove: (id: string, rpc?: RpcClient) => Promise<void>;
};

let initialized = false;

function broadcast(): void {
  void getCrossWindowEvents().emit(CHANGED_EVENT);
}

export const useAgentsStore = create<AgentsState>((set, get) => ({
  hydrated: false,
  customAgents: [],
  activeId: BUILTIN_AGENTS[0].id,
  all: () => [...BUILTIN_AGENTS, ...get().customAgents],
  hydrate: async (rpc) => {
    if (initialized) return;
    initialized = true;
    const { custom, activeId } = rpc
      ? await loadAgentsFromRpc(rpc)
      : await loadAgents();
    set({ customAgents: custom, activeId, hydrated: true });

    void getCrossWindowEvents().listen(CHANGED_EVENT, async () => {
      const fresh = rpc ? await loadAgentsFromRpc(rpc) : await loadAgents();
      set({ customAgents: fresh.custom, activeId: fresh.activeId });
    });
  },
  setActiveId: (id) => {
    set({ activeId: id });
    void saveActiveAgentId(id).then(broadcast);
  },
  upsert: async (agent, rpc) => {
    if (agent.builtIn && !rpc) return;
    // RPC is the source of truth — write through, then update the
    // in-memory map and the KV cache from the returned row so all
    // three layers stay coherent. Without an RPC, fall through to the
    // legacy KV-only path; built-ins still cannot be edited there
    // because the legacy path predates upsert-against-built-ins.
    const stored = rpc ? await upsertPersonaViaRpc(rpc, agent) : agent;
    const list = get().customAgents;
    const idx = list.findIndex((a) => a.id === stored.id);
    // Built-ins live in BUILTIN_AGENTS, not customAgents — when the
    // RPC echoes back an edited built-in, the customAgents list is
    // left untouched.
    let next = list;
    if (!stored.builtIn) {
      next =
        idx === -1
          ? [...list, stored]
          : list.map((a) => (a.id === stored.id ? stored : a));
      set({ customAgents: next });
    }
    await saveCustomAgents(next);
    broadcast();
  },
  remove: async (id, rpc) => {
    if (rpc) {
      await deletePersonaViaRpc(rpc, id);
    }
    const list = get().customAgents.filter((a) => a.id !== id);
    set({ customAgents: list });
    let active = get().activeId;
    if (active === id) {
      active = BUILTIN_AGENTS[0].id;
      set({ activeId: active });
      void saveActiveAgentId(active);
    }
    await saveCustomAgents(list);
    broadcast();
  },
}));

export { newAgentId };
