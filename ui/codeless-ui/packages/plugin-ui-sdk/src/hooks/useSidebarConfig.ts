// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/useSidebarConfig.ts@a7fecef1c641cc8800aa2162f108131c6b426451
import { useQuery } from "@tanstack/react-query";
import { useAgentClient } from "./useAgentClient";
import type { SidebarConfig } from "@codeless/rpc";

const EMPTY: SidebarConfig = { version: 1, sections: [] };

export function useSidebarConfig() {
  const { data: client } = useAgentClient();
  return useQuery<SidebarConfig>({
    queryKey: ["sidebar-config"],
    queryFn: () => client!.sidebar.getConfig(),
    enabled: client !== undefined,
    staleTime: 30_000,
    placeholderData: EMPTY,
  });
}
