// Exit test for M4b (DOCS/WORKSPACE-ATTACH.md milestone 4):
// "happy-path attach modal" round-trip. Renders the Settings ->
// Workspaces tab against `MockRpcClient`, drives the picker +
// validator + confirm flow, and asserts the new row lands in the
// table. Also covers the detach happy path against an already-
// attached row so the per-row Detach button is exercised.

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type { AttachedWorkspace, RepoId } from "@/lib/rpc/wire";
import type { PathPicker } from "@/lib/shell/path-picker";
import { ShellProvider } from "@/lib/shell/provider";
import { useWorkspacesStore } from "@/modules/workspaces/store";

import { WorkspacesSection } from "./WorkspacesSection";

function mount(client: MockRpcClient, picker?: PathPicker) {
  return render(
    <RpcProvider client={client}>
      <ShellProvider
        capabilities={{ customWindowControls: false }}
        pathPicker={picker}
      >
        <WorkspacesSection />
      </ShellProvider>
    </RpcProvider>,
  );
}

beforeEach(() => {
  useWorkspacesStore.setState({
    workspaces: [],
    activeRepoId: null,
    status: "idle",
    error: null,
  });
});

afterEach(() => {
  cleanup();
});

describe("WorkspacesSection — Settings -> Workspaces tab", () => {
  it("renders the empty state when no workspaces are attached", async () => {
    const client = new MockRpcClient();
    mount(client);
    await waitFor(() =>
      expect(screen.getByTestId("workspaces-empty-state")).toBeInTheDocument(),
    );
  });

  it("opens the attach modal from the empty-state CTA", async () => {
    const client = new MockRpcClient();
    mount(client);
    await waitFor(() =>
      expect(screen.getByTestId("workspaces-empty-state")).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByTestId("workspaces-empty-attach-button"));
    await waitFor(() =>
      expect(screen.getByTestId("attach-workspace-dialog")).toBeInTheDocument(),
    );
  });

  it("round-trips a typed path: validate -> add_repo + attach_workspace -> table row", async () => {
    const client = new MockRpcClient();
    mount(client);
    await waitFor(() =>
      expect(screen.getByTestId("workspaces-empty-state")).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByTestId("workspaces-attach-button"));
    const pathInput = await screen.findByTestId("attach-ws-path-input");
    fireEvent.change(pathInput, { target: { value: "/tmp/myproject" } });

    // Wait for the debounced validator to populate the green-checks
    // panel. The mock returns a fully-valid result so the Attach
    // button must become enabled before we click.
    const submit = await screen.findByTestId("attach-ws-submit-button");
    await waitFor(
      () => {
        expect(screen.getByTestId("attach-ws-validation")).toBeInTheDocument();
        expect(submit).not.toBeDisabled();
      },
      { timeout: 2000 },
    );

    fireEvent.click(submit);
    await waitFor(
      () =>
        expect(screen.queryByTestId("attach-workspace-dialog")).not.toBeInTheDocument(),
      { timeout: 2000 },
    );

    await waitFor(() =>
      expect(screen.getByTestId("workspaces-table")).toBeInTheDocument(),
    );
    expect(screen.getByText("myproject")).toBeInTheDocument();
    expect(screen.getByText("/tmp/myproject")).toBeInTheDocument();
  });

  it("invokes the shell-injected PathPicker when Browse is clicked", async () => {
    const client = new MockRpcClient();
    let calls = 0;
    const picker: PathPicker = {
      async pickDirectory() {
        calls += 1;
        return "/tmp/from-picker";
      },
    };
    mount(client, picker);
    await waitFor(() =>
      expect(screen.getByTestId("workspaces-empty-state")).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByTestId("workspaces-attach-button"));
    const browse = await screen.findByTestId("attach-ws-browse-button");
    await act(async () => {
      fireEvent.click(browse);
    });
    await waitFor(() => expect(calls).toBe(1));
    expect(
      (screen.getByTestId("attach-ws-path-input") as HTMLInputElement).value,
    ).toBe("/tmp/from-picker");
  });

  it("detaches an attached workspace via the per-row Detach button", async () => {
    const client = new MockRpcClient();
    // Add a repo + pre-attached workspace so we have a row to detach.
    const repo = await client.call("add_repo", {
      name: "alpha",
      clone_url: "",
      default_branch: "main",
      local_path: "/tmp/alpha",
      git_auth: { kind: "ssh", key_path: "" },
      concurrency_cap: null,
      default_runner: null,
    });
    const seed: AttachedWorkspace = {
      repo_id: repo.id as RepoId,
      repo_name: repo.name,
      fs_root: repo.local_path,
      attached_at: 1_000,
      default_runner: null,
    };
    client.seedAttachedWorkspaces([seed]);

    mount(client);
    await waitFor(() =>
      expect(screen.getByTestId("workspaces-table")).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByTestId(`workspaces-detach-${repo.id}`));
    const submit = await screen.findByTestId("detach-ws-submit-button");
    fireEvent.click(submit);
    await waitFor(() =>
      expect(
        screen.queryByTestId(`workspaces-row-${repo.id}`),
      ).not.toBeInTheDocument(),
    );
  });
});
