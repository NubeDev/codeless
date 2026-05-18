// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/useSlotWriter.ts@a7fecef1c641cc8800aa2162f108131c6b426451
import { useCallback, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { RpcClient } from "@codeless/rpc";
import { useAgent } from "@codeless/ui-core";

/**
 * Options for a single `writeSlot` call.
 *
 * `expectedGeneration` enables OCC writes: the server rejects the
 * write with a 409 if the slot's generation differs, preventing a
 * silent clobber. Leave `undefined` for LWW (last-writer-wins).
 */
export interface WriteSlotOptions {
  expectedGeneration?: number;
}

export interface SlotWriterApi {
  writeSlot: (
    path: string,
    slot: string,
    value: unknown,
    opts?: WriteSlotOptions,
  ) => Promise<boolean>;
  isPending: boolean;
  error: Error | null;
  clearError: () => void;
}

/**
 * `useSlotWriter` — imperative slot write for plugin micro-frontends.
 *
 * Plugin authors reach for this when they want to write a slot value
 * directly from a custom control, settings panel, or form — anywhere
 * outside the SDUI two-way-binding system.
 */
export function useSlotWriter(): SlotWriterApi {
  const agentQuery = useAgent();
  const qc = useQueryClient();
  const [isPending, setIsPending] = useState(false);
  const [error, setError] = useState<Error | null>(null);

  const clearError = useCallback(() => setError(null), []);

  const writeSlot = useCallback(
    async (
      path: string,
      slot: string,
      value: unknown,
      opts?: WriteSlotOptions,
    ): Promise<boolean> => {
      const client = agentQuery.data as RpcClient | undefined;
      if (!client) {
        setError(new Error("RpcClient not ready"));
        return false;
      }

      setIsPending(true);
      setError(null);

      try {
        await client.slots.writeSlot(path, slot, value, {
          expectedGeneration: opts?.expectedGeneration,
        });

        await qc.invalidateQueries({
          predicate: (q) => {
            const key = q.queryKey;
            return (
              Array.isArray(key) &&
              key.some((k) => typeof k === "string" && k === path)
            );
          },
        });

        return true;
      } catch (err) {
        setError(err instanceof Error ? err : new Error(String(err)));
        return false;
      } finally {
        setIsPending(false);
      }
    },
    [agentQuery.data, qc],
  );

  return { writeSlot, isPending, error, clearError };
}
