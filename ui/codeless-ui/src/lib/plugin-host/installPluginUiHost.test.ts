import { afterEach, describe, expect, it, vi } from "vitest";

import {
  getMfRuntime,
  getSlotContributors,
  type MfRuntime,
} from "@codeless/plugin-ui-sdk";

import { MockRpcClient } from "../rpc/mock-client";
import { RpcError } from "../rpc/error";
import type { RpcClient } from "../rpc/client";
import type { PluginListEntry } from "../rpc/methods";

import {
  installPluginUiHost,
  resetPluginUiHostForTesting,
} from "./installPluginUiHost";

afterEach(() => {
  resetPluginUiHostForTesting();
});

describe("installPluginUiHost", () => {
  it("installs an MF runtime even when list_plugins returns no rows", async () => {
    const rpc = new MockRpcClient();
    const out = await installPluginUiHost(rpc);
    expect(out.listed).toBe(true);
    expect(out.plugins).toEqual([]);
    expect(getMfRuntime()).not.toBeNull();
    expect(getSlotContributors("assistant-panel")).toEqual([]);
  });

  it("degrades to an empty registry when list_plugins is unavailable", async () => {
    const rpc: Pick<RpcClient, "call"> = {
      call: vi.fn().mockRejectedValue(new RpcError("not_found", "list_plugins")),
    };
    const out = await installPluginUiHost(rpc as RpcClient);
    expect(out.listed).toBe(false);
    expect(out.plugins).toEqual([]);
    expect(getMfRuntime()).not.toBeNull();
  });

  it("registers UI contributions from plugins that ship a remote", async () => {
    const fixture: PluginListEntry[] = [
      {
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
            {
              name: "AppendCard",
              module: "./AppendCard",
              slot: "tool-result:notes.append",
            },
          ],
        },
      },
      {
        // A backend-only plugin: no remote, no exposes.
        id: "backend-only",
        version: "0.0.1",
        remote_name: "backend-only",
        contributes_ui: false,
        ui: null,
      },
    ];
    const rpc: Pick<RpcClient, "call"> = {
      call: vi.fn().mockResolvedValue({ plugins: fixture }),
    };
    const registerRemote = vi.fn();
    const loadRemote = vi.fn().mockResolvedValue({ default: () => null });
    const fakeRuntime: MfRuntime = { registerRemote, loadRemote };

    const out = await installPluginUiHost(rpc as RpcClient, {
      mfRuntime: fakeRuntime,
    });

    expect(out.plugins).toHaveLength(2);
    expect(getSlotContributors("assistant-panel")).toHaveLength(1);
    expect(getSlotContributors("tool-result:notes.append")).toHaveLength(1);
    expect(getSlotContributors("assistant-panel")[0]?.pluginId).toBe("notes");
    expect(getMfRuntime()).toBe(fakeRuntime);
  });

  it("is idempotent — a second call returns the cached descriptor without re-fetching", async () => {
    const rpc: Pick<RpcClient, "call"> = {
      call: vi.fn().mockResolvedValue({ plugins: [] }),
    };
    const first = await installPluginUiHost(rpc as RpcClient);
    const second = await installPluginUiHost(rpc as RpcClient);
    expect(second).toBe(first);
    expect(rpc.call).toHaveBeenCalledTimes(1);
  });

  it("placeholder runtime rejects loadRemote with a structured error", async () => {
    const rpc: Pick<RpcClient, "call"> = {
      call: vi.fn().mockResolvedValue({ plugins: [] }),
    };
    const onPluginLoadError = vi.fn();
    await installPluginUiHost(rpc as RpcClient, { onPluginLoadError });
    const rt = getMfRuntime();
    expect(rt).not.toBeNull();
    await expect(rt!.loadRemote("notes", "AssistantPanel")).rejects.toThrow(
      /no MF runtime adapter installed/,
    );
    expect(onPluginLoadError).toHaveBeenCalledOnce();
  });
});
