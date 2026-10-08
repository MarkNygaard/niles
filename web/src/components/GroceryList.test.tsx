import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { GroceryList, howMany } from "@/components/GroceryList";
import type { GroceryListProps } from "@/components/GroceryList";
import type { GroceryItem } from "@/lib/api";

const milk: GroceryItem = {
  id: 1,
  name: "Letmælk",
  said: "milk",
  added_at: "2026-10-08T10:00:00Z",
};
const bread: GroceryItem = {
  id: 2,
  name: "Rugbrød",
  quantity: "2",
  added_at: "2026-10-08T10:01:00Z",
};
const eggs: GroceryItem = {
  id: 3,
  name: "Æg",
  added_at: "2026-10-08T10:02:00Z",
  checked_at: "2026-10-08T11:00:00Z",
};

function renderList(props: Partial<GroceryListProps> = {}) {
  const handlers = {
    onAdd: vi.fn(),
    onToggle: vi.fn(),
    onEdit: vi.fn(),
    onRemove: vi.fn(),
    onClear: vi.fn(),
  };
  render(<GroceryList items={[milk, bread, eggs]} usual={[]} {...handlers} {...props} />);
  return handlers;
}

describe("GroceryList", () => {
  it("keeps what is in the basket apart from what is left", () => {
    renderList();
    const [toBuy, basket] = screen.getAllByRole("list");
    expect(within(toBuy).getAllByRole("checkbox")).toHaveLength(2);
    expect(within(basket).getByRole("checkbox", { name: "Æg" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  it("says what an item was asked for as, so a wrong guess can be fixed", () => {
    renderList();
    expect(screen.getByText("asked for as “milk”")).toBeInTheDocument();
  });

  it("puts a tapped item in the basket", () => {
    const { onToggle } = renderList();
    fireEvent.click(screen.getByRole("checkbox", { name: "Letmælk" }));
    expect(onToggle).toHaveBeenCalledWith(milk);
  });

  it("adds what is typed, and empties the field", () => {
    const { onAdd } = renderList();
    const field = screen.getByRole("textbox", { name: "Add to the list" });
    fireEvent.change(field, { target: { value: "  skyr " } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(onAdd).toHaveBeenCalledWith("skyr");
    expect(field).toHaveValue("");
  });

  it("adds a usual product in one press", () => {
    const { onAdd } = renderList({ usual: ["Smør"] });
    fireEvent.click(screen.getByRole("button", { name: "Add Smør" }));
    expect(onAdd).toHaveBeenCalledWith("Smør");
  });

  it("renames before it is bought", () => {
    const { onEdit } = renderList();
    fireEvent.click(screen.getByRole("button", { name: "Edit Letmælk" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Name" }), {
      target: { value: "Sødmælk" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Quantity" }), {
      target: { value: "2" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onEdit).toHaveBeenCalledWith(milk, { name: "Sødmælk", quantity: "2" });
  });

  it("does not offer to rename what is already bought", () => {
    renderList();
    expect(screen.queryByRole("button", { name: "Edit Æg" })).toBeNull();
  });

  it("removes from the editor", () => {
    const { onRemove } = renderList();
    fireEvent.click(screen.getByRole("button", { name: "Edit Rugbrød" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(onRemove).toHaveBeenCalledWith(bread);
  });

  it("clears the basket", () => {
    const { onClear } = renderList();
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(onClear).toHaveBeenCalled();
  });

  it("says how to start on an empty list", () => {
    renderList({ items: [] });
    expect(screen.getByText(/add milk to the list/)).toBeInTheDocument();
    expect(screen.queryByText("In the basket")).toBeNull();
  });

  it("offers nemlig.com only when it is switched on", () => {
    renderList();
    expect(screen.queryByRole("button", { name: /at nemlig.com/ })).toBeNull();
  });

  it("opens the nemlig.com picker for an item", () => {
    const onPick = vi.fn();
    renderList({ onPick });
    fireEvent.click(screen.getByRole("button", { name: "Choose Rugbrød at nemlig.com" }));
    expect(onPick).toHaveBeenCalledWith(bread);
  });

  it("shows the chosen product on the item", () => {
    const linked = {
      ...milk,
      nemlig: {
        id: "701012",
        name: "Letmælk 1,5% øko.",
        description: "1 l / Arla ØKO",
        price: 13.95,
        unit_price: "13,95 kr/l",
        image: "https://nemlig.com/x.jpg",
        available: true,
      },
    };
    renderList({ items: [linked], onPick: vi.fn() });
    expect(
      screen.getByRole("button", { name: "Letmælk 1,5% øko. at nemlig.com — change" }),
    ).toBeInTheDocument();
  });

  it("does not offer nemlig.com for what is already in the basket", () => {
    renderList({ items: [eggs], onPick: vi.fn() });
    expect(screen.queryByRole("button", { name: /at nemlig.com/ })).toBeNull();
  });

  it("sends to nemlig.com only what has a product chosen", () => {
    const onSend = vi.fn();
    const linked = { ...bread, nemlig: { id: "1", name: "Rugbrød", description: "", price: 20, unit_price: null, image: null, available: true } };
    renderList({ items: [milk, linked, eggs], onPick: vi.fn(), onSend });
    fireEvent.click(screen.getByRole("button", { name: "Send 1 to nemlig.com" }));
    expect(onSend).toHaveBeenCalled();
  });

  it("offers no sending when nothing has a product chosen", () => {
    renderList({ onPick: vi.fn(), onSend: vi.fn() });
    expect(screen.queryByRole("button", { name: /Send .* to nemlig.com/ })).toBeNull();
  });

  const linkedBread = {
    ...bread,
    nemlig: {
      id: "77",
      name: "Rugbrød",
      description: "1 stk.",
      price: 20,
      unit_price: null,
      image: null,
      available: true,
    },
  };

  it("estimates what the basket will cost", () => {
    // Two loaves at 20 kr.
    renderList({ items: [linkedBread], onPick: vi.fn(), onSend: vi.fn() });
    expect(screen.getByText("≈ 40,00 kr")).toBeInTheDocument();
  });

  it("prices from nemlig as it is now when that is known", () => {
    const current = new Map([["77", { ...linkedBread.nemlig, price: 15, offer: "15 kr" }]]);
    renderList({ items: [linkedBread], onPick: vi.fn(), onSend: vi.fn(), current });
    expect(screen.getByText("≈ 30,00 kr")).toBeInTheDocument();
    expect(screen.getByText("On offer · 15 kr")).toBeInTheDocument();
  });

  it("says when the chosen product is sold out", () => {
    const current = new Map([["77", { ...linkedBread.nemlig, available: false }]]);
    renderList({ items: [linkedBread], onPick: vi.fn(), onSend: vi.fn(), current });
    expect(screen.getByText("Sold out at nemlig.com")).toBeInTheDocument();
    // And not counted: it will not go in the basket.
    expect(screen.getByText("≈ 0,00 kr")).toBeInTheDocument();
  });

  it("reads how many the way the basket does", () => {
    expect(howMany("2")).toBe(2);
    expect(howMany("3 poser")).toBe(3);
    expect(howMany("500 g")).toBe(1);
    expect(howMany(undefined)).toBe(1);
  });
});
