// Router-level deep-link guarantee from BROWSER-LAUNCHER.md §"Deep-link
// is router-managed": `navigate()` must not strip the workspace
// param when the caller does not supply it, otherwise a tab switch
// (e.g. clicking a job row -> `/jobs/123`) silently drops the
// active-workspace deep-link and browser-back lands on the wrong
// workspace.

import { beforeEach, describe, expect, it } from "vitest";

import { navigate } from "./index";

beforeEach(() => {
  window.history.replaceState(null, "", "/");
});

describe("navigate() preserved query params", () => {
  it("carries the workspace param across path-only navigations", () => {
    window.history.replaceState(null, "", "/?workspace=r-a");
    navigate("/jobs/123");
    expect(window.location.pathname).toBe("/jobs/123");
    expect(window.location.search).toBe("?workspace=r-a");
  });

  it("preserves workspace when the target supplies its own unrelated query", () => {
    window.history.replaceState(null, "", "/?workspace=r-a");
    navigate("/jobs?filter=reviews");
    expect(window.location.pathname).toBe("/jobs");
    const params = new URLSearchParams(window.location.search);
    expect(params.get("workspace")).toBe("r-a");
    expect(params.get("filter")).toBe("reviews");
  });

  it("lets the caller override workspace explicitly", () => {
    window.history.replaceState(null, "", "/?workspace=r-a");
    navigate("/jobs?workspace=r-b");
    expect(new URLSearchParams(window.location.search).get("workspace")).toBe(
      "r-b",
    );
  });

  it("does nothing when the target equals the current URL", () => {
    window.history.replaceState(null, "", "/jobs?workspace=r-a");
    const before = window.history.length;
    navigate("/jobs");
    expect(window.history.length).toBe(before);
    expect(window.location.search).toBe("?workspace=r-a");
  });
});
