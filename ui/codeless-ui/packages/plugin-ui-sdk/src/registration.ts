/**
 * Plugin slot-contribution registry.
 *
 * The host shell calls `registerPluginContributions(...)` once per
 * enabled plugin (typically after `RpcClient.plugins.list()` returns).
 * `<PluginSlot id="..." />` then looks contributors up by slot id at
 * render time. There is no plugin-side API here — plugins declare
 * their contributions in `plugin.toml`; this module is the host-side
 * landing pad for that data.
 *
 * The registry is process-global. There is exactly one host shell
 * per process; tests reset it with `resetRegistryForTesting()`.
 */

import { parseSlotId, type ParsedSlotId } from "./slots";

/**
 * One MF-exposed module a plugin contributes. Mirrors a
 * `[[contributes.ui.exposes]]` row in `plugin.toml`.
 */
export interface PluginExpose {
  /** MF expose name, e.g. `"AssistantPanel"`. */
  readonly name: string;
  /** Source-path of the module within the plugin, e.g. `"./AssistantPanel"`. */
  readonly module: string;
  /** Resolved slot id, e.g. `"tool-result:notes.append"`. */
  readonly slot: string;
}

/**
 * The full set of UI contributions from one plugin. Built from the
 * plugin's `contributes.ui` block by the host's
 * `RpcClient.plugins.list()` adapter.
 */
export interface PluginContribution {
  readonly pluginId: string;
  /** MF remote name. Conventionally equal to `pluginId`. */
  readonly remoteName: string;
  /** Fully-qualified URL to the plugin's `mf-manifest.json`. */
  readonly manifestUrl: string;
  readonly exposes: readonly PluginExpose[];
}

/**
 * The contributor view stored by slot. `<PluginSlot/>` consumes this
 * shape exclusively — it never sees the surrounding
 * `PluginContribution` plumbing.
 */
export interface SlotContributor {
  readonly pluginId: string;
  readonly remoteName: string;
  readonly manifestUrl: string;
  readonly exposeName: string;
  readonly slot: ParsedSlotId;
}

interface Registry {
  /** Keyed by full slot id (e.g. `"tool-result:notes.append"`). */
  readonly bySlotId: Map<string, SlotContributor[]>;
  /** Keyed by plugin id; tracks which slot ids that plugin owns so we can unregister cleanly. */
  readonly slotIdsByPlugin: Map<string, Set<string>>;
}

const registry: Registry = {
  bySlotId: new Map(),
  slotIdsByPlugin: new Map(),
};

/**
 * Cause used to fan a host-side fix-up onto every mounted PluginSlot
 * when the registry mutates. PluginSlot subscribes; nothing else
 * should.
 */
const listeners = new Set<() => void>();
function notify(): void {
  for (const cb of listeners) cb();
}

/**
 * Subscribe to registry changes. Returns an unsubscribe function.
 * PluginSlot uses this to re-render when a plugin is enabled or
 * disabled at runtime; non-UI callers should not need it.
 */
export function subscribeToRegistry(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/**
 * Register one plugin's UI contributions. Idempotent on
 * `pluginId` — re-registering with the same id replaces the previous
 * entry. Unknown slot ids are silently dropped here: the host runs
 * strict manifest validation upstream, so anything that reaches this
 * call has already passed; this defensive check only catches a host
 * bug, not a plugin one.
 */
export function registerPluginContributions(c: PluginContribution): void {
  unregisterPluginContributions(c.pluginId);
  const slotIds = new Set<string>();
  for (const ex of c.exposes) {
    const parsed = parseSlotId(ex.slot);
    if (!parsed) continue;
    const contributor: SlotContributor = {
      pluginId: c.pluginId,
      remoteName: c.remoteName,
      manifestUrl: c.manifestUrl,
      exposeName: ex.name,
      slot: parsed,
    };
    const list = registry.bySlotId.get(ex.slot) ?? [];
    list.push(contributor);
    registry.bySlotId.set(ex.slot, list);
    slotIds.add(ex.slot);
  }
  registry.slotIdsByPlugin.set(c.pluginId, slotIds);
  notify();
}

/**
 * Drop a plugin's contributions. Used when a plugin is disabled at
 * runtime, or in tests.
 */
export function unregisterPluginContributions(pluginId: string): void {
  const slotIds = registry.slotIdsByPlugin.get(pluginId);
  if (!slotIds) return;
  for (const slotId of slotIds) {
    const list = registry.bySlotId.get(slotId);
    if (!list) continue;
    const remaining = list.filter((c) => c.pluginId !== pluginId);
    if (remaining.length === 0) registry.bySlotId.delete(slotId);
    else registry.bySlotId.set(slotId, remaining);
  }
  registry.slotIdsByPlugin.delete(pluginId);
  notify();
}

/**
 * Look up contributors for a resolved slot id. Returns the empty
 * array when nothing is registered (so PluginSlot can render its
 * fallback without a null check).
 *
 * For "exactly 1" slots, the host's manifest validation already
 * rejects double registration, so the returned array's length is at
 * most 1 in practice — but the SDK does not assume that, since a
 * future runtime-enable feature might briefly contend during a
 * swap.
 */
export function getSlotContributors(slotId: string): readonly SlotContributor[] {
  return registry.bySlotId.get(slotId) ?? [];
}

/** Test-only. Drops every registered contribution. */
export function resetRegistryForTesting(): void {
  registry.bySlotId.clear();
  registry.slotIdsByPlugin.clear();
  notify();
}
