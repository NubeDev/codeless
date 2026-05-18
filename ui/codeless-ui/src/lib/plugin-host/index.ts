// Host-side seam for the plugin-substrate UI federation. The browser
// (and Tauri) shell entry calls `installPluginUiHost(rpc)` once; every
// other consumer reads contributors out of `@codeless/plugin-ui-sdk`
// directly. Keeping a thin host module separate from the SDK is how
// the SDK stays mobile-safe (R1) — the host module is free to import
// host-only modules like the `RpcClient` impl.

export {
  installPluginUiHost,
  resetPluginUiHostForTesting,
} from "./installPluginUiHost";
export type {
  PluginUiHostInstall,
  InstallPluginUiHostOptions,
} from "./installPluginUiHost";

// Re-export the host-facing pieces of the SDK so a slot site can do
// `import { PluginSlot } from "@/lib/plugin-host"` without learning
// the SDK package name. Plugin AUTHORS import from the SDK directly;
// the host re-export is a convenience for sites inside `ui/codeless-
// ui/src/`.
export {
  PluginSlot,
  parseSlotId,
  isKnownSlot,
} from "@codeless/plugin-ui-sdk";
export type {
  PluginSlotProps,
  ParsedSlotId,
  SlotName,
} from "@codeless/plugin-ui-sdk";
