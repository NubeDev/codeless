/**
 * `<PluginSlot/>` end-to-end inside jsdom. Tests pin:
 *
 *   - empty slot renders fallback;
 *   - registered contributor's module is loaded through the
 *     installed MfRuntime and rendered;
 *   - the slot argument is passed through as `slotArg`;
 *   - a contributor that crashes during render is caught by the
 *     per-contributor error boundary (other contributors keep
 *     rendering);
 *   - an unknown slot id renders an error card, not children.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement, type ReactElement, type ReactNode } from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import {
  PluginSlot,
  resetPluginSlotCacheForTesting,
} from "./PluginSlot";
import {
  registerPluginContributions,
  resetRegistryForTesting,
} from "../registration";
import {
  setMfRuntime,
  resetMfRuntimeForTesting,
  type MfRuntime,
} from "../mf";

interface FakeRuntimeOptions {
  modules: Record<string, unknown>;
  failOn?: Set<string>;
}

function fakeRuntime(opts: FakeRuntimeOptions): MfRuntime {
  return {
    registerRemote: vi.fn(),
    loadRemote: async <T,>(name: string, exposeName: string): Promise<T> => {
      const key = `${name}/${exposeName}`;
      if (opts.failOn?.has(key)) {
        throw new Error(`forced failure for ${key}`);
      }
      const mod = opts.modules[key];
      if (mod === undefined) throw new Error(`no fake module for ${key}`);
      return mod as T;
    },
  };
}

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
  // Two microtask ticks: one for loadRemote's Promise.resolve chain,
  // one for React to commit the suspended component once it resolves.
  await act(async () => {
    await new Promise<void>((r) => setTimeout(r, 0));
  });
  return { container, root };
}

afterEach(() => {
  resetRegistryForTesting();
  resetMfRuntimeForTesting();
  resetPluginSlotCacheForTesting();
  document.body.innerHTML = "";
});

describe("<PluginSlot/>", () => {
  it("renders fallback when no contributor is registered", async () => {
    setMfRuntime(fakeRuntime({ modules: {} }));
    const { container } = await mountAndFlush(
      createElement(PluginSlot, {
        id: "assistant-panel",
        fallback: createElement("span", { "data-testid": "fb" }, "empty"),
      }),
    );
    expect(container.querySelector("[data-testid='fb']")?.textContent).toBe(
      "empty",
    );
  });

  it("renders an error card for an unknown slot id", async () => {
    setMfRuntime(fakeRuntime({ modules: {} }));
    const { container } = await mountAndFlush(
      createElement(PluginSlot, { id: "not-a-real-slot" }),
    );
    const alert = container.querySelector("[data-codeless-plugin-error]");
    expect(alert?.textContent).toMatch(/unknown slot id/);
  });

  it("loads and mounts a contributor through MfRuntime", async () => {
    function Panel(props: { slotArg: string | null }): ReactElement {
      return createElement(
        "div",
        { "data-testid": "panel" },
        `slotArg=${String(props.slotArg)}`,
      );
    }
    setMfRuntime(
      fakeRuntime({ modules: { "notes/AssistantPanel": { default: Panel } } }),
    );
    registerPluginContributions({
      pluginId: "notes",
      remoteName: "notes",
      manifestUrl: "/plugins/notes/ui/mf-manifest.json",
      exposes: [
        {
          name: "AssistantPanel",
          module: "./AssistantPanel",
          slot: "assistant-panel",
        },
      ],
    });

    const { container } = await mountAndFlush(
      createElement(PluginSlot, {
        id: "assistant-panel",
        loading: createElement("span", null, "loading"),
      }),
    );

    expect(
      container.querySelector("[data-testid='panel']")?.textContent,
    ).toBe("slotArg=null");
  });

  it("passes the parameterised slot argument through as slotArg", async () => {
    function Card(props: { slotArg: string | null }): ReactElement {
      return createElement(
        "div",
        { "data-testid": "card" },
        `arg=${String(props.slotArg)}`,
      );
    }
    setMfRuntime(
      fakeRuntime({ modules: { "notes/AppendCard": { default: Card } } }),
    );
    registerPluginContributions({
      pluginId: "notes",
      remoteName: "notes",
      manifestUrl: "/plugins/notes/ui/mf-manifest.json",
      exposes: [
        {
          name: "AppendCard",
          module: "./AppendCard",
          slot: "tool-result:notes.append",
        },
      ],
    });
    const { container } = await mountAndFlush(
      createElement(PluginSlot, { id: "tool-result:notes.append" }),
    );
    expect(container.querySelector("[data-testid='card']")?.textContent).toBe(
      "arg=notes.append",
    );
  });

  it("isolates a crashing contributor inside its error boundary", async () => {
    function BadPanel(): ReactElement {
      throw new Error("boom from BadPanel");
    }
    function GoodPanel(): ReactElement {
      return createElement("div", { "data-testid": "good" }, "ok");
    }
    setMfRuntime(
      fakeRuntime({
        modules: {
          "bad/Panel": { default: BadPanel },
          "good/Panel": { default: GoodPanel },
        },
      }),
    );
    registerPluginContributions({
      pluginId: "bad",
      remoteName: "bad",
      manifestUrl: "/plugins/bad/ui/mf-manifest.json",
      exposes: [
        {
          name: "Panel",
          module: "./Panel",
          slot: "composer-attachment-action:bad",
        },
      ],
    });
    registerPluginContributions({
      pluginId: "good",
      remoteName: "good",
      manifestUrl: "/plugins/good/ui/mf-manifest.json",
      exposes: [
        {
          name: "Panel",
          module: "./Panel",
          slot: "composer-attachment-action:good",
        },
      ],
    });

    const errSpy = vi
      .spyOn(console, "error")
      .mockImplementation(() => undefined);

    const bad = await mountAndFlush(
      createElement(PluginSlot, { id: "composer-attachment-action:bad" }),
    );
    const good = await mountAndFlush(
      createElement(PluginSlot, { id: "composer-attachment-action:good" }),
    );

    expect(
      bad.container.querySelector("[data-codeless-plugin-error]")?.textContent,
    ).toMatch(/boom from BadPanel/);
    expect(
      good.container.querySelector("[data-testid='good']")?.textContent,
    ).toBe("ok");

    errSpy.mockRestore();
  });
});
