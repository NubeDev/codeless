/**
 * Codeless plugin UI slot vocabulary.
 *
 * The host shell enumerates the slots; a plugin manifest declares
 * which slots its modules contribute to. Plugins cannot invent slot
 * ids — adding a slot is a host-side change. See
 * DOCS/plugins/PLUGIN-UI-FEDERATION.md § Slot vocabulary.
 *
 * v0.1 (locked 2026-05-18, plugin-substrate-runtimes stage 1):
 *
 *   assistant-panel                       — 1 module per plugin per thread
 *   tool-result:<tool_id>                 — exactly 1 module per tool id
 *   persona-picker:<persona_id>           — exactly 1 module per persona id
 *   settings-page:<plugin_id>             — exactly 1 module per plugin
 *   composer-attachment-action:<plugin_id> — unbounded per plugin
 */

export type SlotName =
  | "assistant-panel"
  | "tool-result"
  | "persona-picker"
  | "settings-page"
  | "composer-attachment-action";

export type SlotCardinality =
  | "per-plugin-per-thread"
  | "exactly-one"
  | "unbounded";

export interface SlotShape {
  /** True when the slot id has the form `<name>:<arg>`. */
  readonly parameterised: boolean;
  /** How many modules may be mounted at a single resolved slot id. */
  readonly cardinality: SlotCardinality;
  /** Which manifest field owns the parameter — used for ownership checks at load. */
  readonly ownerKind: "plugin" | "tool" | "persona" | null;
}

export const SLOT_VOCABULARY: { readonly [K in SlotName]: SlotShape } = {
  "assistant-panel": {
    parameterised: false,
    cardinality: "per-plugin-per-thread",
    ownerKind: "plugin",
  },
  "tool-result": {
    parameterised: true,
    cardinality: "exactly-one",
    ownerKind: "tool",
  },
  "persona-picker": {
    parameterised: true,
    cardinality: "exactly-one",
    ownerKind: "persona",
  },
  "settings-page": {
    parameterised: true,
    cardinality: "exactly-one",
    ownerKind: "plugin",
  },
  "composer-attachment-action": {
    parameterised: true,
    cardinality: "unbounded",
    ownerKind: "plugin",
  },
};

export const SLOT_NAMES: readonly SlotName[] = Object.keys(
  SLOT_VOCABULARY,
) as SlotName[];

export interface ParsedSlotId {
  readonly name: SlotName;
  readonly shape: SlotShape;
  /** Null when the slot is non-parameterised. */
  readonly arg: string | null;
  /** The full slot id, exactly as written in the manifest. */
  readonly raw: string;
}

/**
 * Parse a slot id (e.g. `"tool-result:notes.append"`) into its name +
 * optional argument. Returns null when the id does not match the slot
 * vocabulary or violates its parameterised/non-parameterised form.
 *
 * Caller is expected to surface the rejection as a manifest parse
 * error at plugin load time.
 */
export function parseSlotId(raw: string): ParsedSlotId | null {
  if (typeof raw !== "string" || raw.length === 0) return null;
  const colon = raw.indexOf(":");
  const name = (colon === -1 ? raw : raw.slice(0, colon)) as SlotName;
  const shape = SLOT_VOCABULARY[name];
  if (!shape) return null;
  if (shape.parameterised) {
    if (colon === -1) return null;
    const arg = raw.slice(colon + 1);
    if (arg.length === 0) return null;
    return { name, shape, arg, raw };
  }
  if (colon !== -1) return null;
  return { name, shape, arg: null, raw };
}

/**
 * True when `slotId` is a syntactically valid v0.1 slot id. Does NOT
 * check ownership — that is the host's load-time responsibility, since
 * the SDK does not see the plugin id / persona id / tool id registry.
 */
export function isKnownSlot(slotId: string): boolean {
  return parseSlotId(slotId) !== null;
}
