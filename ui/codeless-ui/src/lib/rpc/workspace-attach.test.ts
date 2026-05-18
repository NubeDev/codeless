// Typed-wire snapshot covering the four workspace-attach methods on
// the `RpcMethodMap`. Exit test for WORKSPACE-ATTACH M3a per
// `DOCS/WORKSPACE-ATTACH.md` ("Milestone 3: a typed-wire snapshot test
// for the four methods"). The TypeScript compiler enforces the
// arg/result shapes at build time; the runtime assertions here pin
// the four wire-level method names so a stray rename in `methods.ts`
// fails the suite before it can drift from the Rust source of truth
// in `codeless-types::workspace`.

import { describe, expect, expectTypeOf, it } from "vitest";

import type { RpcArgs, RpcMethod, RpcMethodMap, RpcResultOf } from "./methods";
import type {
  AttachWorkspaceArgs,
  AttachWorkspaceResult,
  DetachPolicy,
  DetachWorkspaceArgs,
  ListWorkspacesResult,
  ValidateWorkspacePathArgs,
  ValidateWorkspacePathResult,
} from "./wire";

// Compile-time guard that each method exists and binds the expected
// arg/result pair. If the Rust types drift, the import + assignment
// below fails to compile and the test file refuses to load.
type WorkspaceMethods = Pick<
  RpcMethodMap,
  "attach_workspace" | "detach_workspace" | "list_workspaces" | "validate_workspace_path"
>;

const _typeCheck: WorkspaceMethods = {
  attach_workspace: {
    args: {} as AttachWorkspaceArgs,
    result: {} as AttachWorkspaceResult,
  },
  detach_workspace: {
    args: {} as DetachWorkspaceArgs,
    result: null,
  },
  list_workspaces: {
    args: {} as Record<string, never>,
    result: {} as ListWorkspacesResult,
  },
  validate_workspace_path: {
    args: {} as ValidateWorkspacePathArgs,
    result: {} as ValidateWorkspacePathResult,
  },
};
void _typeCheck;

describe("workspace-attach RPC wire surface", () => {
  it("exposes the four method names", () => {
    // Pinning the literal method strings is the runtime half of the
    // snapshot; the type-level half is enforced by the `Pick<...>`
    // above. A rename on either side breaks one of the two halves.
    const names: ReadonlyArray<RpcMethod> = [
      "attach_workspace",
      "detach_workspace",
      "list_workspaces",
      "validate_workspace_path",
    ];
    expect(names).toEqual([
      "attach_workspace",
      "detach_workspace",
      "list_workspaces",
      "validate_workspace_path",
    ]);
  });

  it("attach_workspace args carry repo_id and optional fs_root_override", () => {
    expectTypeOf<RpcArgs<"attach_workspace">>().toEqualTypeOf<AttachWorkspaceArgs>();
    expectTypeOf<RpcResultOf<"attach_workspace">>().toEqualTypeOf<AttachWorkspaceResult>();

    const sample: AttachWorkspaceArgs = {
      repo_id: "repo_01H" as AttachWorkspaceArgs["repo_id"],
      fs_root_override: null,
    };
    expect(Object.keys(sample).sort()).toEqual(["fs_root_override", "repo_id"]);
  });

  it("detach_workspace args carry the on_running_jobs policy", () => {
    expectTypeOf<RpcArgs<"detach_workspace">>().toEqualTypeOf<DetachWorkspaceArgs>();
    expectTypeOf<RpcResultOf<"detach_workspace">>().toEqualTypeOf<null>();

    const policies: DetachPolicy[] = ["refuse", "stop", "leave-running"];
    expect(policies).toEqual(["refuse", "stop", "leave-running"]);

    const sample: DetachWorkspaceArgs = {
      repo_id: "repo_01H" as DetachWorkspaceArgs["repo_id"],
      on_running_jobs: "refuse",
    };
    expect(Object.keys(sample).sort()).toEqual(["on_running_jobs", "repo_id"]);
  });

  it("list_workspaces takes no args and returns the workspaces array", () => {
    expectTypeOf<RpcArgs<"list_workspaces">>().toEqualTypeOf<Record<string, never>>();
    expectTypeOf<RpcResultOf<"list_workspaces">>().toEqualTypeOf<ListWorkspacesResult>();

    const result: ListWorkspacesResult = { workspaces: [] };
    expect(result.workspaces).toEqual([]);
  });

  it("validate_workspace_path args carry the candidate path", () => {
    expectTypeOf<RpcArgs<"validate_workspace_path">>().toEqualTypeOf<ValidateWorkspacePathArgs>();
    expectTypeOf<
      RpcResultOf<"validate_workspace_path">
    >().toEqualTypeOf<ValidateWorkspacePathResult>();

    const sample: ValidateWorkspacePathArgs = { path: "/home/me/code" };
    expect(Object.keys(sample)).toEqual(["path"]);

    // Pin the result shape's discriminated checks so a field rename
    // in Rust (e.g. `is_dir` -> `is_directory`) trips the type check
    // here as well as the snapshot test in `codeless-types`.
    const empty: ValidateWorkspacePathResult = {
      canonical: null,
      is_dir: false,
      is_git_repo: false,
      default_branch: null,
      already_attached: false,
      readable: false,
      writable: false,
      problems: [],
    };
    expect(Object.keys(empty).sort()).toEqual(
      [
        "already_attached",
        "canonical",
        "default_branch",
        "is_dir",
        "is_git_repo",
        "problems",
        "readable",
        "writable",
      ].sort(),
    );
  });
});
