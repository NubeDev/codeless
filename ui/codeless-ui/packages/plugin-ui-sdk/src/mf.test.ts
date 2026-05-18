/**
 * MfRuntime install/reset semantics. Tests pin the contract the host
 * shell relies on: install-once, reinstall-throws-with-different-rt,
 * idempotent on same-rt.
 */
import { describe, it, expect, afterEach } from "vitest";
import {
  setMfRuntime,
  getMfRuntime,
  resetMfRuntimeForTesting,
  pluginManifestUrl,
  type MfRuntime,
} from "./mf";

function fakeRuntime(): MfRuntime {
  return {
    registerRemote: () => undefined,
    loadRemote: async <T,>() => ({}) as T,
  };
}

afterEach(() => resetMfRuntimeForTesting());

describe("MfRuntime install", () => {
  it("stores and returns the installed runtime", () => {
    expect(getMfRuntime()).toBeNull();
    const rt = fakeRuntime();
    setMfRuntime(rt);
    expect(getMfRuntime()).toBe(rt);
  });

  it("is idempotent for the same runtime instance", () => {
    const rt = fakeRuntime();
    setMfRuntime(rt);
    setMfRuntime(rt);
    expect(getMfRuntime()).toBe(rt);
  });

  it("throws when reinstalled with a different runtime", () => {
    setMfRuntime(fakeRuntime());
    expect(() => setMfRuntime(fakeRuntime())).toThrow(/already installed/);
  });

  it("resetMfRuntimeForTesting clears the slot", () => {
    setMfRuntime(fakeRuntime());
    resetMfRuntimeForTesting();
    expect(getMfRuntime()).toBeNull();
  });
});

describe("pluginManifestUrl", () => {
  it("produces a stable relative path", () => {
    expect(pluginManifestUrl("notes")).toBe("/plugins/notes/ui/mf-manifest.json");
  });

  it("respects a base URL and strips trailing slashes", () => {
    expect(pluginManifestUrl("notes", "http://h:1/")).toBe(
      "http://h:1/plugins/notes/ui/mf-manifest.json",
    );
    expect(pluginManifestUrl("notes", "http://h:1///")).toBe(
      "http://h:1/plugins/notes/ui/mf-manifest.json",
    );
  });

  it("encodes plugin ids that contain URL-special chars", () => {
    expect(pluginManifestUrl("a b")).toBe(
      "/plugins/a%20b/ui/mf-manifest.json",
    );
  });
});
