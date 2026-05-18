// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/useUsers.ts@a7fecef1c641cc8800aa2162f108131c6b426451
import { useQuery } from "@tanstack/react-query";
import { useAgentClient } from "./useAgentClient";
import type { User } from "@codeless/rpc";

export interface UserFilters {
  filter?: string;
  sort?: string;
  page?: number;
  size?: number;
}

export function useUsers(filters: UserFilters = {}) {
  const { data: client } = useAgentClient();
  const { filter, sort, page, size } = filters;
  return useQuery<User[]>({
    queryKey: ["users", filter, sort, page, size],
    queryFn: () => client!.users.list({ filter, sort, page, size }),
    enabled: client !== undefined,
    staleTime: 30_000,
  });
}
