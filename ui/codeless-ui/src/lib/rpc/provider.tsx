// React context for the single `RpcClient` instance. Every component
// reads it via `useRpc()`; nothing constructs its own client. The shell
// entry (`shells/browser/main.tsx`, later `shells/desktop/main.tsx`)
// builds the right implementation for its transport and passes it into
// `<RpcProvider>`.

import { createContext, useContext, type ReactNode } from "react";

import type { RpcClient } from "./client";

const Ctx = createContext<RpcClient | null>(null);

export function RpcProvider({
  client,
  children,
}: {
  client: RpcClient;
  children: ReactNode;
}) {
  return <Ctx.Provider value={client}>{children}</Ctx.Provider>;
}

export function useRpc(): RpcClient {
  const c = useContext(Ctx);
  if (!c) {
    throw new Error(
      "useRpc(): no RpcProvider in tree. The shell entry must wrap <App> in <RpcProvider client={...}>.",
    );
  }
  return c;
}
