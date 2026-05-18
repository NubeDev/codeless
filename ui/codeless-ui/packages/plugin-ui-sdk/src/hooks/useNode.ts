// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/useNode.ts@a7fecef1c641cc8800aa2162f108131c6b426451
import { useEffect } from "react";
import { useAgent, useGraphStoreOptional, useGraphStoreNode } from "@codeless/ui-core";
import type { NodeSnapshot } from "@codeless/rpc";

/**
 * Returns a live NodeSnapshot for `path`, driven by the host's shared
 * SSE connection. No per-hook EventSource is created.
 *
 * If the node is not yet in the GraphStore cache one HTTP GET is
 * issued to prime the cache; subsequent updates arrive via SSE.
 */
export function useNode(path: string): NodeSnapshot | undefined {
  const store = useGraphStoreOptional();
  const agent = useAgent();

  const cached = useGraphStoreNode(path);

  useEffect(() => {
    if (!store || !agent.data || !path || cached !== undefined) return;
    void agent.data.nodes
      .getNode(path)
      .then((node) => store.getState()._mergeNodes([node]))
      .catch(console.error);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [store, agent.data, path]);

  return cached;
}
