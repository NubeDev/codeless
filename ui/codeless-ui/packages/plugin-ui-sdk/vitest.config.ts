/// <reference types="vitest" />
import { defineConfig } from "vitest/config";

/**
 * Vitest configuration for @codeless/plugin-ui-sdk.
 *
 * Mirrors the host ui-codeless setup (jsdom + globals) so plugin
 * authors can run a single vitest config across the host shell and
 * any plugin UI they ship from this repo. `include` is restricted to
 * source files that survive the stage-8 typecheck — the rest of the
 * rubix port lights up as later stages fill in `@codeless/rpc` and
 * the missing UI primitives.
 */
export default defineConfig({
  test: {
    environment: "jsdom",
    globals: true,
    include: [
      "src/editable-collection.test.tsx",
      "src/slots.test.ts",
      "src/mf.test.ts",
      "src/registration.test.ts",
      "src/eslint-config.test.ts",
      "src/components/PluginSlot.test.tsx",
    ],
  },
});
