/**
 * Shape tests for the R6 ESLint flat-config. We cannot run ESLint
 * inline here without pulling its full pipeline into vitest; instead
 * pin the contract that plugin authors and host-side enforcement
 * tooling depend on:
 *
 *   - the config is an array (flat-config shape);
 *   - the first entry targets plugin sources;
 *   - the singletons that must not be re-bundled are listed;
 *   - the Tauri import pattern is forbidden;
 *   - fetch / XMLHttpRequest are on the no-restricted-globals list;
 *   - the build-config carve-out exists.
 *
 * Full lint-rule integration coverage is owned by
 * `plugin_ui_e2e::r6_eslint_rejects_forbidden_imports`.
 */
import { describe, it, expect } from "vitest";
import codelessConfig from "./eslint-config";

interface RestrictedImportsRule {
  patterns: { group: string[]; message: string }[];
}
type Rule = unknown;

function rule(entry: { rules?: Record<string, Rule> } | undefined, name: string): Rule {
  if (!entry?.rules) return undefined;
  return entry.rules[name];
}

describe("codelessPluginEslintConfig", () => {
  it("is a non-empty array (flat-config shape)", () => {
    expect(Array.isArray(codelessConfig)).toBe(true);
    expect(codelessConfig.length).toBeGreaterThan(0);
  });

  const main = codelessConfig.find(
    (e) => e.files?.some((f) => f.startsWith("src/")) ?? false,
  );
  const buildCarveOut = codelessConfig.find(
    (e) => e.files?.some((f) => f.startsWith("rsbuild.config.")) ?? false,
  );

  it("targets plugin sources", () => {
    expect(main).toBeDefined();
  });

  it("forbids @tauri-apps/* via no-restricted-imports", () => {
    const r = rule(main, "no-restricted-imports") as
      | [string, RestrictedImportsRule]
      | undefined;
    expect(r?.[0]).toBe("error");
    const groups = r?.[1].patterns.flatMap((p) => p.group) ?? [];
    expect(groups).toContain("@tauri-apps/*");
  });

  it("forbids re-bundling each shared singleton", () => {
    const r = rule(main, "no-restricted-imports") as
      | [string, RestrictedImportsRule]
      | undefined;
    const groups = r?.[1].patterns.flatMap((p) => p.group) ?? [];
    for (const pkg of ["react", "react-dom", "zustand", "@tanstack/react-query"]) {
      expect(groups).toContain(pkg);
    }
  });

  it("forbids fetch and XMLHttpRequest globally", () => {
    const r = rule(main, "no-restricted-globals") as
      | [string, ...{ name: string; message: string }[]]
      | undefined;
    const names = (r ?? []).slice(1).map((n) => (n as { name: string }).name);
    expect(names).toContain("fetch");
    expect(names).toContain("XMLHttpRequest");
  });

  it("carves out the rsbuild.config.* build script", () => {
    expect(buildCarveOut).toBeDefined();
    expect(rule(buildCarveOut, "no-restricted-imports")).toBe("off");
    expect(rule(buildCarveOut, "no-restricted-syntax")).toBe("off");
  });
});
