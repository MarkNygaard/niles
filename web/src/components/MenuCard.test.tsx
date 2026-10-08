import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MenuCard } from "./MenuCard";
import type { MenuItem } from "@/lib/menu";

const MENU: MenuItem[] = [
  { id: "groceries", hidden: false },
  { id: "chat", hidden: false },
];

describe("MenuCard", () => {
  it("shows the whole menu, Home first and Me last", () => {
    render(<MenuCard menu={MENU} onChange={vi.fn()} />);
    const text = document.body.textContent ?? "";
    expect(text.indexOf("Home")).toBeLessThan(text.indexOf("Groceries"));
    expect(text.indexOf("Chat")).toBeLessThan(text.indexOf("Me"));
    // Only the middle two can be picked up.
    expect(screen.getAllByRole("button", { name: /^Move / })).toHaveLength(2);
  });

  it("moves an entry and keeps what is hidden", () => {
    const onChange = vi.fn();
    render(
      <MenuCard
        menu={[
          { id: "groceries", hidden: true },
          { id: "chat", hidden: false },
        ]}
        onChange={onChange}
      />,
    );
    fireEvent.keyDown(screen.getByRole("button", { name: "Move Chat" }), { key: "ArrowUp" });
    expect(onChange).toHaveBeenCalledWith({ order: ["chat", "groceries"], hidden: ["groceries"] });
  });

  it("hides an entry with its switch and keeps the order", () => {
    const onChange = vi.fn();
    render(<MenuCard menu={MENU} onChange={onChange} />);
    fireEvent.click(screen.getByRole("switch", { name: "Show Groceries" }));
    expect(onChange).toHaveBeenCalledWith({ order: ["groceries", "chat"], hidden: ["groceries"] });
  });
});
