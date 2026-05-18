// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/useUser.ts@a7fecef1c641cc8800aa2162f108131c6b426451
import { useQuery } from "@tanstack/react-query";
import { useAgentClient } from "./useAgentClient";
import type { User } from "@codeless/rpc";

export function useUser(userId: string) {
  const { data: client } = useAgentClient();
  return useQuery<User | undefined>({
    queryKey: ["users", "detail", userId],
    queryFn: async () => {
      const users = await client!.users.list({ filter: `id==${userId}` });
      return users[0];
    },
    enabled: client !== undefined && !!userId,
    staleTime: 30_000,
  });
}
