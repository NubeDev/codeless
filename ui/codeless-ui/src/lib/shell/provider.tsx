import { createContext, useContext, type ReactNode } from "react";

import type { ShellCapabilities } from "./capabilities";
import {
  noopWindowControls,
  type WindowControlsAdapter,
} from "./window-controls";

interface ShellValue {
  capabilities: ShellCapabilities;
  windowControls: WindowControlsAdapter;
}

const ShellContext = createContext<ShellValue | null>(null);

type ProviderProps = {
  capabilities: ShellCapabilities;
  windowControls?: WindowControlsAdapter;
  children: ReactNode;
};

export function ShellProvider({
  capabilities,
  windowControls,
  children,
}: ProviderProps) {
  const value: ShellValue = {
    capabilities,
    windowControls: windowControls ?? noopWindowControls,
  };
  return (
    <ShellContext.Provider value={value}>{children}</ShellContext.Provider>
  );
}

export function useShell(): ShellValue {
  const v = useContext(ShellContext);
  if (!v) throw new Error("useShell must be used inside <ShellProvider>");
  return v;
}

export function useShellCapabilities(): ShellCapabilities {
  return useShell().capabilities;
}

export function useWindowControls(): WindowControlsAdapter {
  return useShell().windowControls;
}
