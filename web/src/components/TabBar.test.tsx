import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { TabBar } from "@/components/TabBar";

describe("TabBar", () => {
  it("marks the house as where you are on the front page", () => {
    render(<TabBar route="/" />);
    expect(screen.getByRole("link", { name: "Home" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("link", { name: "Me" })).not.toHaveAttribute("aria-current");
  });

  it("keeps Me marked on the pages inside it", () => {
    // Settings is reached from Me, so the tab that leads back to it is Me.
    render(<TabBar route="/me/settings" />);
    expect(screen.getByRole("link", { name: "Me" })).toHaveAttribute("aria-current", "page");
  });

  it("marks the list on the list", () => {
    render(<TabBar route="/groceries" />);
    expect(screen.getByRole("link", { name: "Groceries" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("does not mistake a route that merely starts with the same letters", () => {
    render(<TabBar route="/meals" />);
    expect(screen.getByRole("link", { name: "Me" })).not.toHaveAttribute("aria-current");
  });
});
