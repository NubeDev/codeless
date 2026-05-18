// Stage-12 end-to-end coverage for the plugin UI federation seam.
// These tests pin the three load-bearing behaviours the substrate-
// runtimes job declares for the UI surface (SCOPE.md §Tests):
//
//   1. host_loads_plugin_remote_and_mounts_assistant_panel — with the
//      `notes` plugin contribution registered, the host's
//      `<PluginSlot id="assistant-panel"/>` mounts the plugin's real
//      `AssistantPanel` remote (imported here from
//      `plugins/notes/ui/src/AssistantPanel.tsx`) and the recent-notes
//      list rendered from a stubbed `tools_call` shows up in the DOM.
//
//   2. mismatched_react_fails_loudly — a plugin whose MF runtime
//      rejects `loadRemote` with a "React singleton version mismatch"
//      error renders the SDK's structured error card *inside the
//      slot* instead of crashing the host shell.
//
//   3. r6_eslint_rejects_forbidden_imports — the R6 ESLint flat-config
//      shipped by `@codeless/plugin-ui-sdk/eslint-config` flags a
//      plugin source file that imports `@tauri-apps/api/core`, that
//      calls `fetch(...)` directly, or that bundles its own copy of
//      `react`. The check is driven through a flat-config interpreter
//      that mirrors ESLint's `no-restricted-imports` /
//      `no-restricted-syntax` / `no-restricted-globals` semantics; the
//      SDK's `eslint-config.test.ts` covers the rule's *shape*, this
//      test covers its *behaviour* on a real plugin source string.
//      Running ESLint itself is left to the plugin author's local
//      tooling — the SDK doesn't pull eslint into the host's test
//      runtime.
//
// The host is exercised through `installPluginUiHost` so the
// boot-time wiring stays on the test path; the SDK's MF runtime seam
// is satisfied with an in-test fake. The substrate's REST surface
// (stage 11) is not in the loop here — the test feeds the plugin
// contribution directly into the RPC mock.
import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement, type ReactNode } from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";

import {
  PluginSlot,
  resetPluginSlotCacheForTesting,
  type MfRuntime,
} from "@codeless/plugin-ui-sdk";
import codelessPluginEslintConfig from "@codeless/plugin-ui-sdk/eslint-config";

import type { RpcClient } from "../rpc/client";
import type { PluginListEntry } from "../rpc/methods";

import {
  installPluginUiHost,
  resetPluginUiHostForTesting,
} from "./installPluginUiHost";

import NotesAssistantPanel, {
  type RpcLike,
} from "../../../../../plugins/notes/ui/src/AssistantPanel";

interface Harness {
  container: HTMLDivElement;
  root: Root;
}

async function mountAndFlush(node: ReactNode): Promise<Harness> {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(node);
  });
  // One tick for the lazy(remote) promise chain, one for React to
  // commit. Followed by an extra tick for the AssistantPanel's
  // `useEffect` → setState → re-render.
  await act(async () => {
    await new Promise<void>((r) => setTimeout(r, 0));
  });
  await act(async () => {
    await new Promise<void>((r) => setTimeout(r, 0));
  });
  return { container, root };
}

function notesContribution(): PluginListEntry {
  return {
    id: "notes",
    version: "0.1.0",
    remote_name: "notes",
    contributes_ui: true,
    ui: {
      mf_manifest_url: "http://server/plugins/notes/ui/mf-manifest.json",
      exposes: [
        {
          name: "AssistantPanel",
          module: "./AssistantPanel",
          slot: "assistant-panel",
        },
      ],
    },
  };
}

function fakeListPluginsRpc(rows: PluginListEntry[]): RpcClient {
  return {
    call: vi.fn().mockImplementation((method: string) => {
      if (method === "list_plugins") return Promise.resolve({ plugins: rows });
      return Promise.reject(new Error(`unexpected RPC method ${method}`));
    }),
  } as unknown as RpcClient;
}

afterEach(() => {
  resetPluginUiHostForTesting();
  resetPluginSlotCacheForTesting();
  document.body.innerHTML = "";
});

describe("plugin_ui_e2e", () => {
  it("host_loads_plugin_remote_and_mounts_assistant_panel", async () => {
    const rpc = fakeListPluginsRpc([notesContribution()]);

    const loadRemote = vi
      .fn()
      .mockImplementation(async (remoteName: string, exposeName: string) => {
        if (remoteName === "notes" && exposeName === "AssistantPanel") {
          return { default: NotesAssistantPanel };
        }
        throw new Error(`no fake for ${remoteName}/${exposeName}`);
      });
    const mfRuntime: MfRuntime = {
      registerRemote: vi.fn(),
      loadRemote,
    };

    const install = await installPluginUiHost(rpc, { mfRuntime });
    expect(install.listed).toBe(true);
    expect(install.plugins).toHaveLength(1);

    const recentNotes = [
      { id: "n1", title: "kickoff", updated_at: 1 },
      { id: "n2", title: "next steps", updated_at: 2 },
    ];
    const pluginRpc: RpcLike = {
      call: vi.fn().mockImplementation((method: string, args: unknown) => {
        expect(method).toBe("tools_call");
        expect(args).toEqual({
          tool: "notes.list_recent",
          args: { limit: 5 },
        });
        return Promise.resolve({ notes: recentNotes });
      }),
    };

    const { container } = await mountAndFlush(
      createElement(PluginSlot, {
        id: "assistant-panel",
        rpc: pluginRpc,
        fallback: createElement("div", { "data-fallback": "true" }, "fallback"),
        loading: createElement(
          "div",
          { "data-loading": "true" },
          "panel loading",
        ),
      }),
    );

    expect(loadRemote).toHaveBeenCalledWith("notes", "AssistantPanel");
    expect(pluginRpc.call).toHaveBeenCalledTimes(1);

    const section = container.querySelector(
      'section[data-plugin="notes"][data-state="ready"]',
    );
    expect(section).not.toBeNull();
    const titles = Array.from(container.querySelectorAll("li[data-note-id]"));
    expect(titles.map((li) => li.textContent)).toEqual([
      "kickoff",
      "next steps",
    ]);
    expect(container.querySelector('[data-fallback="true"]')).toBeNull();
  });

  it("mismatched_react_fails_loudly", async () => {
    const rpc = fakeListPluginsRpc([notesContribution()]);

    // Simulates MF's behaviour when a plugin pins a different major
    // of a singleton: `loadRemote` rejects synchronously with an
    // error message MF surfaces verbatim. The SDK's per-contributor
    // error boundary catches it and renders the structured error
    // card in the slot.
    const mismatch = new Error(
      "Module Federation: shared module react expected ^19, plugin pinned ^18",
    );
    const mfRuntime: MfRuntime = {
      registerRemote: vi.fn(),
      loadRemote: vi.fn().mockRejectedValue(mismatch),
    };
    await installPluginUiHost(rpc, { mfRuntime });

    const { container } = await mountAndFlush(
      createElement(PluginSlot, {
        id: "assistant-panel",
        rpc: { call: () => Promise.resolve({}) },
        fallback: createElement("div", { "data-fallback": "true" }),
      }),
    );

    const card = container.querySelector(
      '[data-codeless-plugin-error="true"]',
    );
    expect(card).not.toBeNull();
    expect(card?.getAttribute("data-plugin-id")).toBe("notes");
    expect(card?.getAttribute("data-slot-id")).toBe("assistant-panel");
    expect(card?.textContent ?? "").toMatch(/react expected \^19/);
    expect(card?.textContent ?? "").toMatch(/plugin pinned \^18/);
    // The host page must not have rendered the slot's fallback —
    // the slot DID have a contributor, the contributor just failed
    // to load, so the slot rendered an error card instead of the
    // empty-slot fallback.
    expect(container.querySelector('[data-fallback="true"]')).toBeNull();
  });

  it("r6_eslint_rejects_forbidden_imports", () => {
    // Forbidden source — exercises each rule axis at least once:
    //   - `@tauri-apps/api/core`            → forbidden import group
    //   - `react`                            → forbidden singleton import
    //   - `fetch("/rpc/list_plugins")`       → forbidden syntax + global
    //   - `window.fetch(...)`                → forbidden syntax
    //   - `new XMLHttpRequest()`             → forbidden global
    const bad = [
      'import { invoke } from "@tauri-apps/api/core";',
      'import React from "react";',
      'export const probe = () => fetch("/rpc/list_plugins");',
      'export const probe2 = () => window.fetch("/rpc/list_plugins");',
      'export const probe3 = () => new XMLHttpRequest();',
    ].join("\n");

    const reports = lintPluginSource(bad);

    const hasTauri = reports.some(
      (r) => r.ruleId === "no-restricted-imports" && /R6:.*@tauri-apps/.test(r.message),
    );
    const hasReact = reports.some(
      (r) =>
        r.ruleId === "no-restricted-imports" &&
        /R6:.*React/.test(r.message),
    );
    const hasFetch = reports.some(
      (r) => r.ruleId === "no-restricted-syntax" && /R6:.*fetch/.test(r.message),
    );
    const hasWindowFetch = reports.some(
      (r) =>
        r.ruleId === "no-restricted-syntax" &&
        /R6:.*window\.fetch/.test(r.message),
    );
    const hasXhr = reports.some(
      (r) =>
        r.ruleId === "no-restricted-globals" &&
        /R6:.*XMLHttpRequest/.test(r.message),
    );

    expect(hasTauri).toBe(true);
    expect(hasReact).toBe(true);
    expect(hasFetch).toBe(true);
    expect(hasWindowFetch).toBe(true);
    expect(hasXhr).toBe(true);

    // The control case — a plugin source that only uses what the
    // SDK + host expose — must lint clean.
    const good = [
      'import { useState } from "@codeless/plugin-ui-sdk";',
      'export const ok = (rpc: { call(m: string, a: unknown): Promise<unknown> }) =>',
      '  rpc.call("tools_call", { tool: "notes.list_recent", args: {} });',
    ].join("\n");
    expect(lintPluginSource(good)).toEqual([]);
  });
});

/**
 * Minimal ESLint flat-config interpreter sufficient to drive the R6
 * config end-to-end against an in-memory source string. Recognises
 * only the rule shapes the R6 config actually uses
 * (no-restricted-imports' `patterns[].group`, no-restricted-syntax's
 * `selector` for the two literal-fetch selectors, no-restricted-
 * globals' `name`); anything else is a no-op so an unrelated rule
 * added later cannot fail this test for the wrong reason.
 *
 * We avoid pulling real ESLint into the host's test runtime: the SDK
 * intentionally does not depend on eslint and the host doesn't
 * either. The plugin author runs the actual `eslint --config
 * eslint.config.js` locally; this test guarantees the shipped
 * config's rules *would* flag the documented violations.
 */
interface LintReport {
  ruleId: string;
  message: string;
}

interface RestrictedImportsRule {
  patterns?: { group?: string[]; message: string }[];
}
interface RestrictedSyntaxRule {
  selector: string;
  message: string;
}
interface RestrictedGlobalRule {
  name: string;
  message: string;
}

function lintPluginSource(source: string): LintReport[] {
  const reports: LintReport[] = [];
  // The R6 config splits into a "plugin sources" entry (src/**) and a
  // build-config carve-out (rsbuild.config.*). We only run the first;
  // the source string represents a file under src/.
  const entry = codelessPluginEslintConfig.find((e) =>
    e.files?.some((f) => f.startsWith("src/")),
  );
  if (!entry?.rules) return reports;

  const noRestrictedImports = entry.rules["no-restricted-imports"] as
    | [string, RestrictedImportsRule]
    | undefined;
  if (noRestrictedImports) {
    const [, opts] = noRestrictedImports;
    for (const pat of opts.patterns ?? []) {
      const regexes = (pat.group ?? []).map(globToRegex);
      for (const importPath of extractImportPaths(source)) {
        if (regexes.some((re) => re.test(importPath))) {
          reports.push({
            ruleId: "no-restricted-imports",
            message: pat.message,
          });
        }
      }
    }
  }

  const noRestrictedSyntax = entry.rules["no-restricted-syntax"] as
    | [string, ...RestrictedSyntaxRule[]]
    | undefined;
  if (noRestrictedSyntax) {
    const [, ...rules] = noRestrictedSyntax;
    for (const r of rules) {
      // The R6 config uses ESLint AST selectors. We don't ship a JS
      // parser here, so we match the two selectors literally: a
      // bare `fetch(` call (anything not preceded by `.`) and a
      // `window.fetch(` call. This is what those selectors flag in
      // an ESLint run, just expressed as a regex.
      if (r.selector === "CallExpression[callee.name='fetch']") {
        const re = /(^|[^.\w$])fetch\s*\(/m;
        if (re.test(source)) {
          reports.push({ ruleId: "no-restricted-syntax", message: r.message });
        }
      } else if (
        r.selector ===
        "CallExpression[callee.object.name='window'][callee.property.name='fetch']"
      ) {
        if (/\bwindow\s*\.\s*fetch\s*\(/.test(source)) {
          reports.push({ ruleId: "no-restricted-syntax", message: r.message });
        }
      }
    }
  }

  const noRestrictedGlobals = entry.rules["no-restricted-globals"] as
    | [string, ...RestrictedGlobalRule[]]
    | undefined;
  if (noRestrictedGlobals) {
    const [, ...rules] = noRestrictedGlobals;
    for (const r of rules) {
      // The selector for a "global" is "the identifier appears as a
      // call/new target without a leading `.`". We treat `new X()` as
      // a global reference for `X`.
      const usePattern = new RegExp(
        `(^|[^.\\w$])(new\\s+)?${escapeRegex(r.name)}\\s*\\(`,
        "m",
      );
      if (usePattern.test(source)) {
        reports.push({ ruleId: "no-restricted-globals", message: r.message });
      }
    }
  }

  return reports;
}

function extractImportPaths(source: string): string[] {
  const out: string[] = [];
  // Static `import ... from "x"` / `import "x"`. Dynamic `import("x")`.
  const reStatic = /\bimport\b(?:[^"';]*from\s*)?["']([^"']+)["']/g;
  const reDynamic = /\bimport\s*\(\s*["']([^"']+)["']\s*\)/g;
  for (const re of [reStatic, reDynamic]) {
    let m: RegExpExecArray | null;
    while ((m = re.exec(source)) !== null) out.push(m[1] ?? "");
  }
  return out.filter(Boolean);
}

function globToRegex(glob: string): RegExp {
  // Convert an ESLint `no-restricted-imports` group pattern to a
  // regex. R6's intent (and ESLint's behaviour with the `ignore`
  // library it uses under the hood) is that `@tauri-apps/*` blocks
  // every subpath of `@tauri-apps`, not just one segment — so we
  // treat `*` as "any characters". A bare name (no `*`) becomes an
  // exact match; this lets `react` and `react/*` co-exist in the
  // same `group` array without overlap surprises.
  let pat = "";
  for (let i = 0; i < glob.length; ) {
    if (glob[i] === "*") {
      pat += ".*";
      i += 1;
    } else {
      pat += escapeRegex(glob[i] ?? "");
      i += 1;
    }
  }
  return new RegExp(`^${pat}$`);
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
