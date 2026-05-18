/**
 * Source of truth for Module Federation shared-singleton version pins.
 *
 * The host's `rsbuild.config.ts` AND every plugin's `rsbuild.config.ts`
 * import this object as the `shared` field of `pluginModuleFederation`.
 * If host and plugin agree, MF unifies the singleton; if a plugin pins
 * a different major it renders a structured error in the slot at
 * runtime (see `plugin_ui_e2e::mismatched_react_fails_loudly`).
 *
 * Bump the version below in the same commit as the host's package.json
 * peer for the same library. Anything not in this map is allowed to be
 * bundled per-plugin — keep the map tight to keep plugin bundles small.
 */

export interface MfSharedEntry {
  /** MF requires this for singleton enforcement. */
  readonly singleton: true;
  /**
   * Semver range. Plugins outside this range still load, but MF
   * surfaces the mismatch as a runtime warning + render error.
   * Keep these as ranges, not exact versions, so a host patch bump
   * doesn't break every plugin.
   */
  readonly requiredVersion: string;
  /**
   * Eager loading is off everywhere. Plugin chunks are loaded async
   * by `PluginSlot`, so an eager singleton load only pads the host
   * bundle without unblocking anything.
   */
  readonly eager: false;
}

export type MfSharedSingletons = { readonly [pkg: string]: MfSharedEntry };

/**
 * Authoritative shared-singleton map. Order is alphabetical so a diff
 * against this file makes the change obvious.
 */
export const sharedSingletons: MfSharedSingletons = {
  "@codeless/plugin-ui-sdk": { singleton: true, requiredVersion: "^0.1", eager: false },
  "@codeless/rpc": { singleton: true, requiredVersion: "^0.1", eager: false },
  "@codeless/ui-core": { singleton: true, requiredVersion: "^0.1", eager: false },
  "@tanstack/react-query": { singleton: true, requiredVersion: "^5", eager: false },
  react: { singleton: true, requiredVersion: "^19", eager: false },
  "react-dom": { singleton: true, requiredVersion: "^19", eager: false },
  zustand: { singleton: true, requiredVersion: "^5", eager: false },
};

/**
 * Returned by `@codeless/plugin-ui-sdk/rsbuild-shared` as the default
 * export so a plugin's `rsbuild.config.ts` can write
 * `shared: codelessShared` without unpacking the named export.
 */
export default sharedSingletons;
