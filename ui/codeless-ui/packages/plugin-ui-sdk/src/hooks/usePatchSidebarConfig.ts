// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/usePatchSidebarConfig.ts@a7fecef1c641cc8800aa2162f108131c6b426451
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useAgentClient } from "./useAgentClient";
import type { SidebarConfig } from "@codeless/rpc";

export function usePatchSidebarConfig() {
  const { data: client } = useAgentClient();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (config: SidebarConfig) => client!.sidebar.patchConfig(config),
    onSuccess: () => { void qc.invalidateQueries({ queryKey: ["sidebar-config"] }); },
  });
}
