// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/hooks/useAgentClient.ts@a7fecef1c641cc8800aa2162f108131c6b426451
/**
 * useAgentClient — resolves the singleton RpcClient.
 *
 * Thin re-export of `useAgent` from `@codeless/ui-core`, renamed for
 * clarity in the plugin author context. Returns a react-query
 * `UseQueryResult`.
 */
export { useAgent as useAgentClient } from "@codeless/ui-core";
