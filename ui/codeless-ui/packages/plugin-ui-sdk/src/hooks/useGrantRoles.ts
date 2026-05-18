// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/useGrantRoles.ts@a7fecef1c641cc8800aa2162f108131c6b426451
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useAgentClient } from "./useAgentClient";

export function useGrantRoles(userId: string) {
  const { data: client } = useAgentClient();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (role: string) =>
      client!.users.grantRole(userId, { role, bulk_action_id: crypto.randomUUID() }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["users"] });
    },
  });
}
