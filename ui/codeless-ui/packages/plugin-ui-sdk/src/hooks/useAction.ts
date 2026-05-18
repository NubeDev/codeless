// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/useAction.ts@a7fecef1c641cc8800aa2162f108131c6b426451
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useAgent } from "@codeless/ui-core";
import type { UiActionRequest, UiActionResponse } from "@codeless/rpc";

/**
 * Fires a SDUI action against the host RPC and returns the response.
 *
 * Wraps `client.ui.action(req)` in a react-query mutation so the
 * caller gets `isPending`, `isError`, and `data` for free.
 */
export function useAction() {
  const agent = useAgent();
  // queryClient retained for future fine-grained invalidations.
  void useQueryClient();

  return useMutation<UiActionResponse, Error, UiActionRequest>({
    mutationFn: (req) => {
      if (!agent.data) throw new Error("RpcClient not ready");
      return agent.data.ui.action(req);
    },
  });
}
