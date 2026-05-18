import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { EmptyWorkspacesState } from "./EmptyWorkspacesState";

describe("EmptyWorkspacesState", () => {
  it("renders the doc-specified copy and CTA", () => {
    render(<EmptyWorkspacesState onAttachClick={() => {}} />);
    expect(screen.getByText("No workspaces attached.")).toBeInTheDocument();
    expect(
      screen.getByText(
        /Attach a directory on this machine to start working with codeless\./,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("workspaces-empty-attach-button"),
    ).toBeEnabled();
  });

  it("fires the attach callback on click", () => {
    const onAttachClick = vi.fn();
    render(<EmptyWorkspacesState onAttachClick={onAttachClick} />);
    fireEvent.click(screen.getByTestId("workspaces-empty-attach-button"));
    expect(onAttachClick).toHaveBeenCalledOnce();
  });

  it("disables the attach button when no callback is supplied", () => {
    render(<EmptyWorkspacesState />);
    expect(
      screen.getByTestId("workspaces-empty-attach-button"),
    ).toBeDisabled();
  });
});
