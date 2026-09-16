import { describe, expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { useViewportHeight } from "./useViewportHeight";

function Probe() {
  return <span data-testid="height">{useViewportHeight()}</span>;
}

describe("useViewportHeight", () => {
  it("reports the screen it is on", () => {
    render(<Probe />);
    expect(screen.getByTestId("height")).toHaveTextContent(
      String(window.innerHeight),
    );
  });

  it("follows the browser's chrome coming and going", () => {
    // Which is the whole point: the sheet was sized once from a figure
    // that iOS had not refreshed yet.
    render(<Probe />);
    act(() => {
      window.innerHeight = 500;
      window.dispatchEvent(new Event("resize"));
    });
    expect(screen.getByTestId("height")).toHaveTextContent("500");
  });
});
