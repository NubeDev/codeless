/**
 * Module Federation runtime seam.
 *
 * The SDK does not bundle a Module Federation runtime; that is the
 * host shell's job (browser / Tauri desktop / mobile each pick the
 * exact build of `@module-federation/enhanced` they need). The SDK
 * exposes a thin `MfRuntime` interface — the host installs one
 * adapter at boot, and `PluginSlot` resolves contributors through
 * it. Tests inject a fake.
 *
 * Keeping the runtime injection-shaped means the SDK has zero
 * dependency on Module Federation tooling, so plugin authors can
 * unit-test their components with vitest without an rsbuild config.
 */

import {
  SLOT_VOCABULARY,
  type SlotName,
  type SlotShape,
  parseSlotId,
  isKnownSlot,
} from "./slots";

export { SLOT_VOCABULARY, parseSlotId, isKnownSlot };
export type { SlotName, SlotShape };

/**
 * The MF host adapter the SDK calls into. A real implementation wraps
 * `@module-federation/enhanced/runtime`:
 *
 * ```ts
 * import { init, loadRemote } from "@module-federation/enhanced/runtime";
 * setMfRuntime({
 *   registerRemote: (name, entry) =>
 *     init({ name: "codeless-host", remotes: [{ name, entry }] }),
 *   loadRemote: (name, exposeName) => loadRemote(`${name}/${exposeName}`),
 * });
 * ```
 */
export interface MfRuntime {
  /**
   * Register a plugin's MF remote against its `mf-manifest.json` URL.
   * Idempotent: calling twice with the same `(name, manifestUrl)` is a
   * no-op. Calling with a different `manifestUrl` for the same
   * `name` must throw — the host owns versioning, the SDK refuses to
   * silently swap.
   */
  registerRemote(name: string, manifestUrl: string): void;
  /**
   * Resolve a previously-registered remote's exposed module. The
   * returned value is whatever the plugin exposed — typically a
   * React component as the default export.
   */
  loadRemote<T = unknown>(remoteName: string, exposeName: string): Promise<T>;
}

let installed: MfRuntime | null = null;

/**
 * Install the MF runtime adapter. Called once by the host shell at
 * boot, before any `<PluginSlot/>` mounts. Reinstall throws — the
 * host owns this and a second install almost certainly indicates a
 * double-mount bug.
 */
export function setMfRuntime(rt: MfRuntime): void {
  if (installed !== null && installed !== rt) {
    throw new Error(
      "@codeless/plugin-ui-sdk: MfRuntime already installed — the host shell installs exactly once at boot",
    );
  }
  installed = rt;
}

/** Returns the installed runtime, or null if none has been installed. */
export function getMfRuntime(): MfRuntime | null {
  return installed;
}

/**
 * Test-only. Discards the installed runtime. Tests that install a
 * fake `MfRuntime` should call this in `afterEach` so the next test
 * starts from a clean slate.
 */
export function resetMfRuntimeForTesting(): void {
  installed = null;
}

/**
 * Build the codeless server URL for a plugin's MF manifest. The host
 * shell does not assemble these by hand; both the registration call
 * and any debug logging use this helper so the path layout stays
 * single-sourced.
 */
export function pluginManifestUrl(pluginId: string, baseUrl = ""): string {
  const trimmed = baseUrl.replace(/\/+$/, "");
  return `${trimmed}/plugins/${encodeURIComponent(pluginId)}/ui/mf-manifest.json`;
}
