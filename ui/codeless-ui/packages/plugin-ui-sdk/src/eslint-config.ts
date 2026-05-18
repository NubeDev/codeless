/**
 * ESLint flat-config enforcing **R6** (CLAUDE.md hard rule):
 *
 *   A plugin's MF remote bundle must not import `@tauri-apps/*`, must
 *   not `fetch` the codeless server directly, must not bundle its own
 *   copy of React, zustand, or `@tanstack/react-query`. The only
 *   allowed network egress is through `RpcClient` sourced from the
 *   MF shared scope.
 *
 * Usage in a plugin's `eslint.config.js`:
 *
 * ```js
 * import codelessPlugin from "@codeless/plugin-ui-sdk/eslint-config";
 * export default [...codelessPlugin];
 * ```
 *
 * The config returns an array (flat-config shape). Plugins are free
 * to append their own rules afterwards.
 *
 * The rules deliberately use only ESLint built-ins
 * (`no-restricted-imports`, `no-restricted-syntax`,
 * `no-restricted-globals`) so the SDK does not need to ship a custom
 * ESLint plugin module. A plugin's only eslint dependency is eslint
 * itself.
 */

/**
 * Loose ESLint flat-config typing. We avoid importing `eslint`'s
 * types so this file compiles whether or not the plugin author has
 * `@types/eslint` installed.
 */
export interface CodelessFlatConfigEntry {
  files?: readonly string[];
  ignores?: readonly string[];
  rules?: Readonly<Record<string, unknown>>;
}

/**
 * The R6 rule wall. Two passes:
 *
 *   1. All plugin source files (`src/**`) — forbid the Tauri import,
 *      forbid `fetch` to the codeless server, forbid bundling the
 *      shared singletons (React, zustand, tanstack-query, ui-core,
 *      rpc, the SDK itself).
 *
 *   2. The `rsbuild.config.ts` itself escapes pass 1 — the plugin's
 *      build needs to *import* the SDK's `rsbuild-shared` and the MF
 *      plugin. Without an explicit exception, the singleton rule
 *      would also fire on the build config.
 *
 * The "no own copy" rule is enforced by `no-restricted-imports`'
 * pattern list: a plugin source file may not import any of the
 * shared singletons. At runtime they are injected by the host via
 * MF shared scope; at author time the plugin must import them from
 * `@codeless/plugin-ui-sdk` re-exports. (Those re-exports live in
 * `src/index.ts` and are re-routed by MF at build time.)
 */
export const codelessPluginEslintConfig: readonly CodelessFlatConfigEntry[] = [
  {
    files: ["src/**/*.{ts,tsx,js,jsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@tauri-apps/*"],
              message:
                "R6: plugin remotes must not import @tauri-apps/* — call RpcClient (from @codeless/plugin-ui-sdk) instead. The host shell handles the Tauri bridge.",
            },
            {
              group: ["react", "react/*", "react-dom", "react-dom/*"],
              message:
                "R6: plugin remotes must not bundle their own React. Import React from @codeless/plugin-ui-sdk re-exports; the host provides the singleton via MF shared scope.",
            },
            {
              group: ["zustand", "zustand/*"],
              message:
                "R6: plugin remotes must not bundle their own zustand. Use the SDK re-export so the host's store is the plugin's store.",
            },
            {
              group: ["@tanstack/react-query", "@tanstack/react-query/*"],
              message:
                "R6: plugin remotes must not bundle their own @tanstack/react-query. Use the SDK re-export so the host's cache is the plugin's cache.",
            },
          ],
        },
      ],
      "no-restricted-syntax": [
        "error",
        {
          // Any literal fetch("/api/...") or fetch("http://localhost:…/…")
          // call is forbidden: plugins must call RpcClient, not the
          // codeless server's HTTP surface directly. We can't tell
          // which URL is the codeless server statically, so the rule
          // is conservatively "no fetch() at all in plugin sources";
          // a plugin that genuinely needs to fetch a third-party URL
          // must inline a `// eslint-disable-next-line` with a
          // reason.
          selector: "CallExpression[callee.name='fetch']",
          message:
            "R6: plugin remotes must not call fetch() — go through RpcClient. If you need to reach a non-codeless URL, justify it with a // eslint-disable-next-line comment.",
        },
        {
          selector:
            "CallExpression[callee.object.name='window'][callee.property.name='fetch']",
          message:
            "R6: plugin remotes must not call window.fetch — go through RpcClient.",
        },
      ],
      "no-restricted-globals": [
        "error",
        {
          name: "fetch",
          message:
            "R6: plugin remotes must not call fetch — go through RpcClient.",
        },
        {
          name: "XMLHttpRequest",
          message:
            "R6: plugin remotes must not use XMLHttpRequest — go through RpcClient.",
        },
      ],
    },
  },
  {
    // The build config legitimately imports the rsbuild MF plugin
    // and the SDK's shared map. Without this carve-out the
    // singleton-import rule would fire on rsbuild.config.ts itself.
    files: ["rsbuild.config.{ts,js,mjs}", "rsbuild.config.*.{ts,js,mjs}"],
    rules: {
      "no-restricted-imports": "off",
      "no-restricted-syntax": "off",
    },
  },
];

export default codelessPluginEslintConfig;
