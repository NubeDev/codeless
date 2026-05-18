/**
 * Slot-contribution registry semantics. Tests pin the host-facing
 * surface PluginSlot relies on, plus the subscribe/notify path that
 * makes a runtime enable/disable cycle re-render mounted slots.
 */
import { describe, it, expect, afterEach } from "vitest";
import {
  registerPluginContributions,
  unregisterPluginContributions,
  getSlotContributors,
  subscribeToRegistry,
  resetRegistryForTesting,
  type PluginContribution,
} from "./registration";

function notesContribution(): PluginContribution {
  return {
    pluginId: "notes",
    remoteName: "notes",
    manifestUrl: "/plugins/notes/ui/mf-manifest.json",
    exposes: [
      { name: "AssistantPanel", module: "./AssistantPanel", slot: "assistant-panel" },
      { name: "AppendCard", module: "./AppendCard", slot: "tool-result:notes.append" },
    ],
  };
}

afterEach(() => resetRegistryForTesting());

describe("registerPluginContributions", () => {
  it("registers contributors and lists them by slot id", () => {
    registerPluginContributions(notesContribution());
    expect(getSlotContributors("assistant-panel")).toHaveLength(1);
    expect(getSlotContributors("tool-result:notes.append")).toHaveLength(1);
    expect(getSlotContributors("tool-result:other.tool")).toHaveLength(0);
  });

  it("drops contributions for unknown slot ids", () => {
    registerPluginContributions({
      pluginId: "weird",
      remoteName: "weird",
      manifestUrl: "/plugins/weird/ui/mf-manifest.json",
      exposes: [{ name: "X", module: "./X", slot: "not-a-real-slot" }],
    });
    expect(getSlotContributors("not-a-real-slot")).toHaveLength(0);
  });

  it("re-registering the same plugin replaces its previous entries", () => {
    registerPluginContributions(notesContribution());
    registerPluginContributions({
      pluginId: "notes",
      remoteName: "notes",
      manifestUrl: "/plugins/notes/ui/mf-manifest.json",
      exposes: [
        { name: "AssistantPanel", module: "./AssistantPanel", slot: "assistant-panel" },
      ],
    });
    expect(getSlotContributors("assistant-panel")).toHaveLength(1);
    expect(getSlotContributors("tool-result:notes.append")).toHaveLength(0);
  });

  it("isolates two plugins contributing to the same unbounded slot", () => {
    registerPluginContributions({
      pluginId: "p1",
      remoteName: "p1",
      manifestUrl: "/plugins/p1/ui/mf-manifest.json",
      exposes: [{ name: "A", module: "./A", slot: "composer-attachment-action:p1" }],
    });
    registerPluginContributions({
      pluginId: "p2",
      remoteName: "p2",
      manifestUrl: "/plugins/p2/ui/mf-manifest.json",
      exposes: [{ name: "B", module: "./B", slot: "composer-attachment-action:p2" }],
    });
    expect(getSlotContributors("composer-attachment-action:p1")).toHaveLength(1);
    expect(getSlotContributors("composer-attachment-action:p2")).toHaveLength(1);
  });
});

describe("unregisterPluginContributions", () => {
  it("drops just one plugin's contributions", () => {
    registerPluginContributions(notesContribution());
    unregisterPluginContributions("notes");
    expect(getSlotContributors("assistant-panel")).toHaveLength(0);
    expect(getSlotContributors("tool-result:notes.append")).toHaveLength(0);
  });

  it("is a no-op for an unknown plugin id", () => {
    expect(() => unregisterPluginContributions("does-not-exist")).not.toThrow();
  });
});

describe("subscribeToRegistry", () => {
  it("notifies on register and unregister", () => {
    let calls = 0;
    const off = subscribeToRegistry(() => {
      calls++;
    });
    registerPluginContributions(notesContribution());
    unregisterPluginContributions("notes");
    off();
    registerPluginContributions(notesContribution());
    expect(calls).toBe(2);
  });
});
