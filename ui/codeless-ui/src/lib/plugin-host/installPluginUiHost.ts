// Host shell wiring for the plugin-substrate UI federation.
//
// `installPluginUiHost(rpc, options?)` runs once at boot. It:
//
//   1. calls `rpc.call("list_plugins", {})` and caches the enabled
//      plugin set in the registry helpers exported below;
//   2. installs a `MfRuntime` on `@codeless/plugin-ui-sdk` so every
//      `<PluginSlot/>` mount can lazy-load the right MF remote;
//   3. for every enabled plugin that contributes UI, registers the
//      remote against its `mf-manifest.json` URL and pushes its
//      `[contributes.ui.exposes]` rows into the SDK's slot-
//      contribution table.
//
// The function is idempotent — re-invocation (e.g. React StrictMode's
// double-mount, or a Hot Module Reload of the boot file) returns the
// already-resolved descriptor without re-fetching the plugin list and
// without throwing on the second `setMfRuntime` because the SDK
// detects same-runtime re-install. A test/runtime reset hook is
// exported for vitest.
//
// Error budget for stage 12:
//
//   - `list_plugins` not implemented on the server (current state of
//     master): the call rejects with an RPC error; the host catches
//     it, logs once at debug level, and treats the registry as empty.
//     Every `<PluginSlot/>` then renders its fallback. The host UI
//     keeps working at full fidelity.
//   - manifest URL fetch failure / MF version mismatch: deferred to
//     `PluginSlot`'s per-contributor error boundary at render time.
//     The boot path does not block on any plugin's chunks.

import {
  registerPluginContributions,
  resetRegistryForTesting,
  resetMfRuntimeForTesting,
  setMfRuntime,
  type MfRuntime,
  type PluginContribution,
} from "@codeless/plugin-ui-sdk";

import { RpcError } from "../rpc/error";
import type { RpcClient } from "../rpc/client";
import type {
  PluginListEntry,
  PluginUiExposeEntry,
} from "../rpc/methods";

export interface PluginUiHostInstall {
  /** The plugin list the host shell read from `list_plugins`. Empty
   *  array when the call failed (server-side surface not yet wired)
   *  or the server returned nothing. */
  readonly plugins: readonly PluginListEntry[];
  /** True iff `list_plugins` succeeded. Surfaces in tests; UI
   *  callers should not branch on it. */
  readonly listed: boolean;
}

export interface InstallPluginUiHostOptions {
  /** Replace the default placeholder `MfRuntime`. The browser shell
   *  passes a real adapter wrapping `@module-federation/enhanced`;
   *  tests pass a fake. */
  mfRuntime?: MfRuntime;
  /** Forwarded to the placeholder runtime. Logs every failed
   *  `loadRemote` call through this so tests can observe without
   *  monkey-patching the console. */
  onPluginLoadError?: (
    remoteName: string,
    exposeName: string,
    cause: unknown,
  ) => void;
}

// The host owns exactly one install. A second concurrent install
// resolves to the same descriptor; the second call is wholly free.
let inflight: Promise<PluginUiHostInstall> | null = null;
let resolved: PluginUiHostInstall | null = null;

/**
 * Install the plugin UI host. Returns the same descriptor on every
 * call within a process.
 */
export function installPluginUiHost(
  rpc: RpcClient,
  options: InstallPluginUiHostOptions = {},
): Promise<PluginUiHostInstall> {
  if (resolved) return Promise.resolve(resolved);
  if (inflight) return inflight;
  inflight = (async () => {
    setMfRuntime(options.mfRuntime ?? buildPlaceholderRuntime(options));
    let listed = false;
    let plugins: PluginListEntry[] = [];
    try {
      const res = await rpc.call("list_plugins", {});
      plugins = res.plugins ?? [];
      listed = true;
    } catch (e) {
      if (e instanceof RpcError) {
        // eslint-disable-next-line no-console
        console.debug(
          "[plugin-host] list_plugins unavailable; treating registry as empty",
          e.kind,
        );
      } else {
        // eslint-disable-next-line no-console
        console.warn(
          "[plugin-host] list_plugins failed; treating registry as empty",
          e,
        );
      }
    }
    for (const p of plugins) {
      const contribution = toContribution(p);
      if (!contribution) continue;
      registerPluginContributions(contribution);
    }
    resolved = { plugins, listed };
    return resolved;
  })();
  return inflight;
}

function toContribution(p: PluginListEntry): PluginContribution | null {
  if (!p.contributes_ui || !p.ui || !p.ui.mf_manifest_url) return null;
  return {
    pluginId: p.id,
    remoteName: p.remote_name,
    manifestUrl: p.ui.mf_manifest_url,
    exposes: p.ui.exposes.map(
      (e: PluginUiExposeEntry) => ({
        name: e.name,
        module: e.module,
        slot: e.slot,
      }),
    ),
  };
}

/**
 * The placeholder runtime is the safety net for the moment when the
 * browser shell has not yet pushed a real MF adapter (e.g. the dev
 * preview running before `@module-federation/enhanced` is wired). It
 * accepts `registerRemote` so the registration call site stays
 * symmetric, but rejects every `loadRemote` with a clear, structured
 * error — surfaced inside the `<PluginSlot/>` error boundary, never
 * thrown synchronously into the host tree.
 *
 * The host shell that ships an MF runtime calls `setMfRuntime` via
 * `installPluginUiHost({ mfRuntime })` and the placeholder never
 * runs.
 */
function buildPlaceholderRuntime(
  options: InstallPluginUiHostOptions,
): MfRuntime {
  const remotes = new Map<string, string>();
  return {
    registerRemote(name, manifestUrl) {
      const prev = remotes.get(name);
      if (prev === undefined) {
        remotes.set(name, manifestUrl);
        return;
      }
      if (prev !== manifestUrl) {
        throw new Error(
          `[plugin-host] refusing to re-register MF remote ${name} with a different manifest url (was ${prev}, got ${manifestUrl})`,
        );
      }
    },
    async loadRemote(remoteName, exposeName) {
      const cause = new Error(
        "[plugin-host] no MF runtime adapter installed; pass options.mfRuntime to installPluginUiHost()",
      );
      options.onPluginLoadError?.(remoteName, exposeName, cause);
      throw cause;
    },
  };
}

/** Test-only. Discards the cached install + the SDK's MF runtime /
 *  registry so the next test starts from a clean slate. */
export function resetPluginUiHostForTesting(): void {
  inflight = null;
  resolved = null;
  resetMfRuntimeForTesting();
  resetRegistryForTesting();
}
