import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { useThemeColor } from "./useThemeColor";

function Sheet({ color }: { color: string | null }) {
  useThemeColor(color);
  return null;
}

function themeColor() {
  return document
    .querySelector('meta[name="theme-color"]')
    ?.getAttribute("content");
}

describe("useThemeColor", () => {
  it("lends the chrome a colour and gives it back", () => {
    document.head.innerHTML = '<meta name="theme-color" content="#f7f7f8" />';
    const { unmount } = render(<Sheet color="oklch(0.7 0.16 60)" />);
    expect(themeColor()).toBe("oklch(0.7 0.16 60)");
    // Closing the sheet has to put the theme's own colour back, or the
    // status bar stays orange over a white page.
    unmount();
    expect(themeColor()).toBe("#f7f7f8");
  });

  it("follows the colour while it changes", () => {
    document.head.innerHTML = '<meta name="theme-color" content="#f7f7f8" />';
    const { rerender } = render(<Sheet color="red" />);
    rerender(<Sheet color="blue" />);
    expect(themeColor()).toBe("blue");
  });

  it("leaves it alone when there is nothing to lend", () => {
    document.head.innerHTML = '<meta name="theme-color" content="#f7f7f8" />';
    render(<Sheet color={null} />);
    expect(themeColor()).toBe("#f7f7f8");
  });
});
