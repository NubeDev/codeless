// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/useSlot.ts@a7fecef1c641cc8800aa2162f108131c6b426451
import { useNode } from "./useNode";
import type { Slot } from "@codeless/rpc";

/**
 * Returns the current value of a named slot on a node.
 * Updates live whenever the shared GraphStore receives a
 * `slot_changed` SSE event.
 */
export function useSlot(path: string, slotName: string): Slot | undefined {
  const node = useNode(path);
  return node?.slots.find((s) => s.name === slotName);
}
