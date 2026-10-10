import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { TabBar } from "@/components/TabBar";

describe("TabBar", () => {
  it("marks the house as where you are on the front page", () => {
    render(<TabBar route="/" />);
    expect(screen.getByRole("link", { name: "Home" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("link", { name: "Groceries" })).not.toHaveAttribute("aria-current");
  });

  it("marks the list on the list", () => {
    render(<TabBar route="/groceries" />);
    expect(screen.getByRole("link", { name: "Groceries" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("marks Settings when Settings was put here", () => {
    render(<TabBar route="/me/settings" menu={[{ id: "settings", hidden: false }]} />);
    expect(screen.getByRole("link", { name: "Settings" })).toHaveAttribute("aria-current", "page");
  });

  it("has no Me: that is the avatar's", () => {
    render(<TabBar route="/" />);
    expect(screen.queryByRole("link", { name: "Me" })).toBeNull();
  });

  it("puts the entries in the arranged order and leaves out the hidden", () => {
    render(
      <TabBar
        route="/"
        menu={[
          { id: "chat", hidden: false },
          { id: "groceries", hidden: true },
        ]}
      />,
    );
    const labels = screen.getAllByRole("link").map((a) => a.textContent);
    expect(labels).toEqual(["Home", "Chat"]);
  });
});
