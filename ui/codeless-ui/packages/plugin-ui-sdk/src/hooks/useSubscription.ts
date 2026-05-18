// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/useSubscription.ts@a7fecef1c641cc8800aa2162f108131c6b426451
import { useGraphStoreSubscription } from "@codeless/ui-core";
import type { GraphEvent } from "@codeless/rpc";

export type GraphEventHandler = (event: GraphEvent) => void;

/**
 * Subscribes to slot-change events for the given node paths.
 *
 * Taps into the host's shared GraphStore (one SSE connection for the
 * whole app) rather than opening a new EventSource per mounted
 * component.
 */
export function useSubscription(
  subjects: string[],
  onEvent: GraphEventHandler,
): void {
  useGraphStoreSubscription(subjects, (path, slot, value, generation) => {
    onEvent({
      event: "slot_changed",
      path,
      slot,
      value,
      generation,
      seq: 0,
    } as unknown as GraphEvent);
  });
}
