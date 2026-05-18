// Exit test for WORKSPACE-ATTACH M3b. The generic
// `TauriIpcClient.call` already routes every `RpcMethod` through
// `invoke("rpc_<method>", { args })`, so M3b is a wire pin rather
// than four hand-written methods: these tests fail if the
// command-name convention (`rpc_<snake_case>`) or the `{ args }`
// envelope ever drifts away from the contract the Rust
// `codeless-tauri-desktop` crate will implement.
//
// Companion to `workspace-attach.test.ts` which pins the same four
// methods at the type level and the `HttpSseClient` JSON-RPC wire.

import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AttachWorkspaceArgs,
  AttachWorkspaceResult,
  DetachWorkspaceArgs,
  ListWorkspacesResult,
  ValidateWorkspacePathArgs,
  ValidateWorkspacePathResult,
} from "./wire";

// Hoisted so the `vi.mock` factory below can reach it; `vi.mock` is
// itself hoisted to the top of the module by Vitest.
const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  // `Channel` is unused by the four request/reply methods under test
  // but must exist so the module-level import in `tauri-ipc-client.ts`
  // resolves under the mock.
  Channel: class {
    id = 0;
    onmessage: ((env: unknown) => void) | null = null;
  },
}));

// Import after the mock is registered so the client picks up the
// stubbed `invoke`.
import { TauriIpcClient } from "./tauri-ipc-client";

describe("TauriIpcClient workspace-attach calls", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("attach_workspace invokes rpc_attach_workspace with { args }", async () => {
    const reply: AttachWorkspaceResult = {
      workspace: {
        repo_id: "repo_01H" as AttachWorkspaceResult["workspace"]["repo_id"],
        repo_name: "code",
        fs_root: "/home/me/code",
        attached_at: 0 as AttachWorkspaceResult["workspace"]["attached_at"],
        default_runner: null,
      },
    };
    invokeMock.mockResolvedValueOnce(reply);

    const args: AttachWorkspaceArgs = {
      repo_id: "repo_01H" as AttachWorkspaceArgs["repo_id"],
      fs_root_override: null,
    };
    const client = new TauriIpcClient();
    const got = await client.call("attach_workspace", args);

    expect(got).toBe(reply);
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("rpc_attach_workspace", { args });
  });

  it("detach_workspace invokes rpc_detach_workspace with { args } and null result", async () => {
    invokeMock.mockResolvedValueOnce(null);

    const args: DetachWorkspaceArgs = {
      repo_id: "repo_01H" as DetachWorkspaceArgs["repo_id"],
      on_running_jobs: "refuse",
    };
    const client = new TauriIpcClient();
    const got = await client.call("detach_workspace", args);

    expect(got).toBeNull();
    expect(invokeMock).toHaveBeenCalledWith("rpc_detach_workspace", { args });
  });

  it("list_workspaces invokes rpc_list_workspaces with empty args", async () => {
    const reply: ListWorkspacesResult = { workspaces: [] };
    invokeMock.mockResolvedValueOnce(reply);

    const client = new TauriIpcClient();
    const got = await client.call("list_workspaces", {});

    expect(got).toBe(reply);
    expect(invokeMock).toHaveBeenCalledWith("rpc_list_workspaces", { args: {} });
  });

  it("validate_workspace_path invokes rpc_validate_workspace_path with the candidate path", async () => {
    const reply: ValidateWorkspacePathResult = {
      canonical: "/home/me/code",
      is_dir: true,
      is_git_repo: true,
      default_branch: "main",
      already_attached: false,
      readable: true,
      writable: true,
      problems: [],
    };
    invokeMock.mockResolvedValueOnce(reply);

    const args: ValidateWorkspacePathArgs = { path: "/home/me/code" };
    const client = new TauriIpcClient();
    const got = await client.call("validate_workspace_path", args);

    expect(got).toBe(reply);
    expect(invokeMock).toHaveBeenCalledWith("rpc_validate_workspace_path", {
      args,
    });
  });
});
