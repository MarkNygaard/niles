import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { GroceryDeliveryCard, arrivesToday } from "@/components/GroceryDeliveryCard";
import type { NemligOrder } from "@/lib/api";

const ORDER: NemligOrder = {
  id: 1,
  status: 1,
  total: 512.5,
  delivery_start: "2026-10-10T07:00:00",
  delivery_end: "2026-10-10T09:00:00",
};

describe("GroceryDeliveryCard", () => {
  it("says when the groceries come, on the day they do", () => {
    render(<GroceryDeliveryCard order={ORDER} today="2026-10-10" />);
    expect(screen.getByText("Groceries arrive today, 07:00–09:00")).toBeInTheDocument();
    expect(screen.getByText("nemlig.com · 512,50 kr")).toBeInTheDocument();
  });

  it("is not there on any other day", () => {
    const { container } = render(<GroceryDeliveryCard order={ORDER} today="2026-10-09" />);
    expect(container).toBeEmptyDOMElement();
  });

  it("is not there without an order", () => {
    expect(arrivesToday(null, "2026-10-10")).toBe(false);
    expect(arrivesToday({ ...ORDER, delivery_start: null }, "2026-10-10")).toBe(false);
  });
});
