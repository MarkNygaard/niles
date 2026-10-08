import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { NemligPicker, kroner } from "@/components/NemligPicker";
import type { GroceryItem, NemligProduct } from "@/lib/api";

const ROLLS: NemligProduct = {
  id: "5060220",
  name: "Surdejsrundstykker",
  description: "6 stk. / 420 g / frost / Hatting",
  price: 14.95,
  unit_price: "35,60 kr/kg",
  image: "https://nemlig.com/scommerce/images/surdejsrundstykker.jpg",
  available: true,
};
const SOLD_OUT: NemligProduct = { ...ROLLS, id: "807006", name: "Møllehjul", price: 12.5, unit_price: "29,76 kr/kg", available: false };

const ITEM: GroceryItem = { id: 3, name: "Rundstykker", added_at: "2026-10-08T10:00:00Z" };

function open(item: GroceryItem = ITEM) {
  const search = vi.fn().mockResolvedValue([ROLLS, SOLD_OUT]);
  const onChoose = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <NemligPicker item={item} search={search} onChoose={onChoose} onClose={vi.fn()} />
    </QueryClientProvider>,
  );
  return { search, onChoose };
}

describe("NemligPicker", () => {
  it("searches by the item's name to begin with", async () => {
    const { search } = open();
    expect(await screen.findByText("Surdejsrundstykker")).toBeInTheDocument();
    expect(search).toHaveBeenCalledWith("Rundstykker");
    expect(screen.getByText("14,95 kr")).toBeInTheDocument();
    expect(screen.getByText(/35,60 kr\/kg/)).toBeInTheDocument();
  });

  it("chooses the product pressed", async () => {
    const { onChoose } = open();
    fireEvent.click(await screen.findByText("Surdejsrundstykker"));
    expect(onChoose).toHaveBeenCalledWith(ITEM, ROLLS);
  });

  it("says what is not available rather than hiding it", async () => {
    open();
    expect(await screen.findByText("Not available")).toBeInTheDocument();
  });

  it("marks the one chosen before, and can have it removed", async () => {
    const { onChoose } = open({ ...ITEM, nemlig: ROLLS });
    expect(await screen.findByRole("button", { name: /Surdejsrundstykker/, pressed: true })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Remove product" }));
    expect(onChoose).toHaveBeenCalledWith({ ...ITEM, nemlig: ROLLS }, null);
  });

  it("searches again for another word", async () => {
    const { search } = open();
    await screen.findByText("Surdejsrundstykker");
    fireEvent.change(screen.getByLabelText("Search nemlig.com"), { target: { value: "boller" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    await vi.waitFor(() => expect(search).toHaveBeenCalledWith("boller"));
  });

  it("holds the cards' places while searching", () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <NemligPicker item={ITEM} search={() => new Promise(() => {})} onChoose={vi.fn()} onClose={vi.fn()} />
      </QueryClientProvider>,
    );
    expect(screen.getByRole("list", { busy: true })).toBeInTheDocument();
  });

  it("writes kroner the Danish way", () => {
    expect(kroner(10.3)).toBe("10,30 kr");
  });
});
