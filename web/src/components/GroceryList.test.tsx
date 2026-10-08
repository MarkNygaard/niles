import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { GroceryList } from "@/components/GroceryList";
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
});
