// Plugin lint config. Re-exports the R6 flat-config shipped by
// `@codeless/plugin-ui-sdk` so the notes plugin uses the same wall
// every plugin uses: no @tauri-apps/* imports, no direct fetch to the
// codeless server, no parallel copies of React, zustand, or
// @tanstack/react-query. A plugin author adding rules of their own
// appends them after the spread.
import codelessPluginEslintConfig from "@codeless/plugin-ui-sdk/eslint-config";

export default [...codelessPluginEslintConfig];
