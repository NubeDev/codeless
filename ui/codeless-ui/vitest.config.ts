import path from "path";

import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Vitest config kept separate from `vite.config.ts` so the production
// build pipeline never picks up `jsdom`, `@testing-library/*`, or the
// test-only globals. The path alias mirrors `tsconfig.json`'s `@/*`.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    setupFiles: ["./src/test/setup.ts"],
  },
});
