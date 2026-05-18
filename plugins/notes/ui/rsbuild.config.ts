// Rsbuild configuration for the notes plugin's Module-Federation
// remote. The host shell serves the produced `mf-manifest.json` at
// `/plugins/notes/ui/mf-manifest.json` (codeless-server stage 11)
// and registers the remote at boot (host-shell stage 10). The plugin
// itself only ships *what it exposes* — the singleton-version pin
// map is sourced wholesale from `@codeless/plugin-ui-sdk/rsbuild-
// shared` so the host and every plugin stay in lockstep on React,
// react-dom, zustand, @tanstack/react-query, @codeless/rpc, and the
// SDK itself. A plugin that wants to disagree about a singleton
// version cannot, by construction: the import is the only source
// of truth.
//
// PLUGIN-UI-FEDERATION.md §"Shared singletons" is the spec; this
// config implements it.

import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";
import { pluginModuleFederation } from "@module-federation/rsbuild-plugin";
import sharedSingletons from "@codeless/plugin-ui-sdk/rsbuild-shared";

export default defineConfig({
  plugins: [
    pluginReact(),
    pluginModuleFederation({
      // The host shell registers this remote under the same name; the
      // server picks it up out of `plugin.toml`'s `id = "notes"`.
      name: "notes",
      filename: "mf-manifest.json",
      exposes: {
        // Mounted at the `assistant-panel` slot. Per
        // PLUGIN-UI-FEDERATION.md §"Slot vocabulary", the slot is
        // non-parameterised and per-plugin-per-thread; one expose
        // suffices.
        "./AssistantPanel": "./src/AssistantPanel.tsx",
      },
      shared: sharedSingletons,
    }),
  ],
  output: {
    distPath: { root: "dist" },
    // codeless-server (stage 11) serves the bundle out of the
    // plugin's `ui/dist/` via ServeDir. Keeping `assetPrefix`
    // relative lets the same artefact load whether the host shell
    // is hosted at the codeless-server root or under a sub-path.
    assetPrefix: "./",
  },
  source: {
    entry: {
      // No standalone entry: the bundle is consumed exclusively via
      // MF `loadRemote`. An empty entry would fail rsbuild's
      // validation, so the AssistantPanel's source path doubles as
      // the entry; rsbuild + MF tree-shake the unused side.
      index: "./src/AssistantPanel.tsx",
    },
  },
});
